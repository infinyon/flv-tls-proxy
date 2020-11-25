use std::io::Error as IoError;
use std::net::SocketAddr;
use std::sync::Arc;

use log::debug;

use event_listener::Event;
use futures_lite::future::zip;
use futures_lite::AsyncReadExt;
use futures_lite::AsyncWriteExt;
use futures_util::stream::StreamExt;

use fluvio_future::net::{TcpListener, TcpStream};
use fluvio_future::openssl::{TlsAcceptor, TlsConnector, TlsError};
use fluvio_future::test_async;

use flv_tls_proxy::ProxyBuilder;

const CA_PATH: &str = "certs/certs/ca.crt";

const SERVER: &str = "127.0.0.1:19998";
const PROXY: &str = "127.0.0.1:20000";
const ITER: u16 = 10;

#[test_async]
async fn test_proxy() -> Result<(), TlsError> {
    run_test(
        TlsAcceptor::builder()?
            .with_certifiate_and_key_from_pem_files(
                "certs/certs/server.crt",
                "certs/certs/server.key",
            )?
            .build(),
        TlsConnector::builder()?
            .with_hostname_vertification_disabled()?
            .with_ca_from_pem_file(CA_PATH)?
            .build(),
    )
    .await
    .expect("proxy test failed");

    Ok(())
}

async fn run_test(acceptor: TlsAcceptor, connector: TlsConnector) -> Result<(), IoError> {
    let addr = SERVER.parse::<SocketAddr>().expect("parse");
    let event = Arc::new(Event::new());

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
            for item in buf.into_iter() {
                str_bytes.push(item);
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
            for item in buf.into_iter() {
                str_bytes.push(item);
            }
            let reply_message = String::from_utf8(str_bytes).expect("utf8");
            debug!(
                "client: loop {}, received reply message: {}",
                i, reply_message
            );
            assert_eq!(reply_message, format!("message{}reply", i));
        }

        debug!("client done");
        event.notify(1);
        Ok(()) as Result<(), IoError>
    };

    let proxy = ProxyBuilder::new(PROXY.to_owned(), acceptor, SERVER.to_string())
        .with_terminate(event.clone());

    let _ = zip(proxy.start(), zip(client_ft, server_ft)).await;

    // give little time to flush everything out

    Ok(())
}
