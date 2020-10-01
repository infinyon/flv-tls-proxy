use std::io::Error as IoError;
use std::net::SocketAddr;
use std::time;

use log::debug;

use futures_lite::future::zip;
use futures_lite::AsyncReadExt;
use futures_lite::AsyncWriteExt;
use futures_util::stream::StreamExt;

use async_net::TcpListener;
use async_net::TcpStream;

use fluvio_future::test_async;
use fluvio_future::timer::sleep;
use fluvio_future::tls::TlsAcceptor;
use fluvio_future::tls::TlsConnector;

use fluvio_future::tls::AcceptorBuilder;
use fluvio_future::tls::ConnectorBuilder;

// const CA_PATH: &'static str = "certs/certs/ca.crt";

const SERVER: &str = "127.0.0.1:19998";
const PROXY: &str = "127.0.0.1:20000";
const ITER: u16 = 10;

#[test_async]
async fn test_proxy() -> Result<(), IoError> {
    test_tls(
        AcceptorBuilder::new_no_client_authentication()
            .load_server_certs("certs/certs/server.crt", "certs/certs/server.key")?
            .build(),
        ConnectorBuilder::new().no_cert_verification().build(),
    )
    .await
    .expect("no client cert test failed");

    // test client authentication
    /*
    test_tls(
        AcceptorBuilder::new_client_authenticate(CA_PATH)?
            .load_server_certs("certs/certs/server.crt", "certs/certs/server.key")?
            .build(),
        ConnectorBuilder::new()
            .load_client_certs("certs/certs/client.crt", "certs/certs/client.key")?
            .load_ca_cert(CA_PATH)?
            .build(),
    )
    .await
    .expect("client cert test fail");
    */

    Ok(())
}

async fn test_tls(acceptor: TlsAcceptor, connector: TlsConnector) -> Result<(), IoError> {
    let addr = SERVER.parse::<SocketAddr>().expect("parse");

    let server_ft = async {
        debug!("server: binding");
        let listener = TcpListener::bind(&addr).await.expect("listener failed");
        debug!("server: successfully binding. waiting for incoming");

        let mut incoming = listener.incoming();
        while let Some(incoming_stream) = incoming.next().await {
            debug!("server: new incoming connection");

            let mut tcp_stream = incoming_stream.expect("no stream");

            let mut i: u16 = 0;

            let mut buf: Vec<u8> = vec![0; 1024];
            let n = tcp_stream.read(&mut buf).await.expect("read");

            debug!("client: loop {}, received reply back bytes: {}", i, n);
            let mut str_bytes = vec![];
            for j in 0..n {
                str_bytes.push(buf[j]);
            }
            let message = String::from_utf8(str_bytes).expect("utf8");
            assert_eq!(message, format!("message{}", i));
            let resply = format!("{}reply", message);
            let reply_bytes = resply.as_bytes();
            debug!("sever: send back reply: {}", resply);
            tcp_stream
                .write_all(reply_bytes)
                .await
                .expect("send failed");

            assert!(i < ITER);

            i += 1;
            if i == ITER {
                return Ok(()) as Result<(), IoError>;
            }
        }

        Ok(()) as Result<(), IoError>
    };

    let client_ft = async {
        debug!("client: sleep to give server chance to come up");
        sleep(time::Duration::from_millis(200)).await;
        debug!("client: trying to connect");
        let tcp_stream = TcpStream::connect(PROXY.to_owned())
            .await
            .expect("connection fail");
        let mut tls_stream = connector
            .connect("localhost", tcp_stream)
            .await
            .expect("tls failed");

        debug!("client: got connection. waiting");

        // do loop for const
        for i in 0..ITER {
            let message = format!("message{}", i);
            let bytes = message.as_bytes();
            debug!("client: loop {} sending test message", i);
            tls_stream.write_all(bytes).await.expect("send failed");
            let mut buf: Vec<u8> = vec![0; 1024];
            let n = tls_stream.read(&mut buf).await.expect("read");
            debug!("client: loop {}, received reply back bytes: {}", i, n);
            let mut str_bytes = vec![];
            for j in 0..n {
                str_bytes.push(buf[j]);
            }
            let message = String::from_utf8(str_bytes).expect("utf8");
            assert_eq!(message, format!("message{}reply", i));
        }

        Ok(()) as Result<(), IoError>
    };

    let proxy = proxy::start(PROXY, acceptor.clone(), SERVER.to_string());

    let _ = zip(proxy, zip(client_ft, server_ft)).await;

    Ok(())
}

mod proxy {

    use std::io::Error as IoError;

    use async_net::TcpListener;
    use async_net::TcpStream;
    use fluvio_future::task::spawn;
    use fluvio_future::tls::DefaultServerTlsStream;
    use fluvio_future::tls::TlsAcceptor;
    use futures_lite::io::copy;

    use futures_util::io::AsyncReadExt;
    use futures_util::stream::StreamExt;
    use log::debug;
    use log::error;
    use log::info;

    /// start TLS proxy at addr to target
    pub async fn start(addr: &str, acceptor: TlsAcceptor, target: String) -> Result<(), IoError> {
        let listener = TcpListener::bind(addr).await?;
        info!("proxy started at: {}", addr);
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            debug!("server: got connection from client");
            if let Ok(tcp_stream) = stream {
                spawn(process_stream(acceptor.clone(), tcp_stream, target.clone()));
            } else {
                error!("no stream detected");
            }
        }

        info!("server terminated");
        Ok(())
    }

    async fn process_stream(acceptor: TlsAcceptor, raw_stream: TcpStream, target: String) {
        let source = raw_stream
            .peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "".to_owned());

        debug!("new connection from {}", source);

        let handshake = acceptor.accept(raw_stream);

        match handshake.await {
            Ok(inner_stream) => {
                debug!("handshake success from: {}", source);
                if let Err(err) = proxy(inner_stream, target, source.clone()).await {
                    error!("error processing tls: {} from source: {}", err, source);
                }
            }
            Err(err) => error!("error handshaking: {} from source: {}", err, source),
        }
    }

    async fn proxy(
        tls_stream: DefaultServerTlsStream,
        target: String,
        source: String,
    ) -> Result<(), IoError> {
        use futures_lite::future::zip;

        debug!(
            "trying to connect to target at: {} from source: {}",
            target, source
        );
        let mut tcp_stream = TcpStream::connect(&target).await?;

        debug!("connect to target: {} from source: {}", target, source);
        let mut target_sink = tcp_stream.clone();

        //let (mut target_stream, mut target_sink) = tcp_stream.split();
        let (mut from_tls_stream, mut from_tls_sink) = tls_stream.split();

        let s_t = format!("{}->{}", source, target);
        let t_s = format!("{}->{}", target, source);
        let source_to_target_ft = async {
            match copy(&mut from_tls_stream, &mut target_sink).await {
                Ok(len) => {
                    debug!("{} copy from source to target: len {}", s_t, len);
                }
                Err(err) => {
                    error!("{} error copying: {}", s_t, err);
                }
            }
        };

        let target_to_source = async {
            match copy(&mut tcp_stream, &mut from_tls_sink).await {
                Ok(len) => {
                    debug!("{} copy from target: len {}", t_s, len);
                }
                Err(err) => {
                    error!("{} error copying: {}", t_s, err);
                }
            }
        };

        zip(source_to_target_ft, target_to_source).await;

        Ok(())
    }
}
