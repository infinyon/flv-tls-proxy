use std::io::Error as IoError;
use std::net::SocketAddr;
use std::time;

use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;
use log::debug;

use futures_lite::future::zip;
use futures_lite::stream::StreamExt;
use futures_util::sink::SinkExt;
use tokio_util::codec::BytesCodec;
use tokio_util::codec::Framed;
use tokio_util::compat::FuturesAsyncReadCompatExt;

use fluvio_future::tls::TlsAcceptor;
use fluvio_future::tls::TlsConnector;

use fluvio_future::net::TcpListener;
use fluvio_future::net::TcpStream;
use fluvio_future::test_async;
use fluvio_future::timer::sleep;

use fluvio_future::tls::AcceptorBuilder;
use fluvio_future::tls::AllTcpStream;
use fluvio_future::tls::ConnectorBuilder;

use flv_tls_proxy::start;

// const CA_PATH: &'static str = "certs/certs/ca.crt";

const SERVER: &str = "127.0.0.1:19998";
const PROXY: &str = "127.0.0.1:20000";
const ITER: u16 = 10;

fn to_bytes(bytes: &[u8]) -> Bytes {
    let mut buf = BytesMut::with_capacity(bytes.len());
    buf.put_slice(&bytes);
    buf.freeze()
}

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
        while let Some(stream) = incoming.next().await {
            debug!("server: new incoming connection");

            let tcp_stream = stream.expect("no stream");

            let mut framed = Framed::new(tcp_stream.compat(), BytesCodec::new());

            let mut i: u16 = 0;
            if let Some(value) = framed.next().await {
                let bytes = value.expect("invalid value");
                debug!(
                    "server: loop {}, received from client: {} bytes",
                    i,
                    bytes.len()
                );

                let slice = bytes.as_ref();
                let mut str_bytes = vec![];
                for b in slice {
                    str_bytes.push(b.to_owned());
                }
                let message = String::from_utf8(str_bytes).expect("utf8");
                assert_eq!(message, format!("message{}", i));
                let resply = format!("{}reply", message);
                let reply_bytes = resply.as_bytes();
                debug!("sever: send back reply: {}", resply);
                framed
                    .send(to_bytes(reply_bytes))
                    .await
                    .expect("send failed");

                assert!(i < ITER);

                i += 1;
                if i == ITER {
                    return Ok(()) as Result<(), IoError>;
                }
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
        let tls_stream = connector
            .connect("localhost", tcp_stream)
            .await
            .expect("tls failed");
        let all_stream = AllTcpStream::Tls(tls_stream);
        let mut framed = Framed::new(all_stream.compat(), BytesCodec::new());
        debug!("client: got connection. waiting");

        // do loop for const
        for i in 0..ITER {
            let message = format!("message{}", i);
            let bytes = message.as_bytes();
            debug!("client: loop {} sending test message", i);
            framed.send(to_bytes(bytes)).await.expect("send failed");
            let reply = framed.next().await.expect("messages").expect("frame");
            debug!("client: loop {}, received reply back", i);
            let slice = reply.as_ref();
            let mut str_bytes = vec![];
            for b in slice {
                str_bytes.push(b.to_owned());
            }
            let message = String::from_utf8(str_bytes).expect("utf8");
            assert_eq!(message, format!("message{}reply", i));
        }


        Ok(()) as Result<(), IoError>
    };

    let proxy = start(PROXY, acceptor.clone(), SERVER.to_string());

    let _ = zip(proxy, zip(client_ft, server_ft)).await;

    Ok(())
}
