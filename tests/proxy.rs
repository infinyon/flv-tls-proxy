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
        let incoming_stream = incoming.next().await.expect("expect incoming");
        let mut tcp_stream = incoming_stream.expect("no stream");

        for i in 0..ITER {
            let mut buf: Vec<u8> = vec![0; 1024];
            let n = tcp_stream.read(&mut buf).await.expect("read");

            debug!("server: loop {}, received reply back bytes: {}", i, n);
            let mut str_bytes = vec![];
            for j in 0..n {
                str_bytes.push(buf[j]);
            }
            let message = String::from_utf8(str_bytes).expect("utf8");
            debug!("server: loop {}, received message: {}", i, message);
            assert_eq!(message, format!("message{}", i));
            let resply = format!("{}reply", message);
            let reply_bytes = resply.as_bytes();
            debug!("sever: send back reply: {}", resply);
            tcp_stream
                .write_all(reply_bytes)
                .await
                .expect("send failed");
        }

        debug!("server done");

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
            debug!("client: loop {} sending test message: {}", i, message);
            let bytes = message.as_bytes();
            tls_stream.write_all(bytes).await.expect("send failed");
            let mut buf: Vec<u8> = vec![0; 1024];
            let n = tls_stream.read(&mut buf).await.expect("read");
            debug!("client: loop {}, received reply back bytes: {}", i, n);
            let mut str_bytes = vec![];
            for j in 0..n {
                str_bytes.push(buf[j]);
            }
            let reply_message = String::from_utf8(str_bytes).expect("utf8");
            debug!(
                "client: loop {}, received reply message: {}",
                i, reply_message
            );
            assert_eq!(reply_message, format!("message{}reply", i));
        }

        debug!("client done");
        Ok(()) as Result<(), IoError>
    };

    let proxy = flv_tls_proxy::start(PROXY, acceptor.clone(), SERVER.to_string());

    let _ = zip(proxy, zip(client_ft, server_ft)).await;

    Ok(())
}
