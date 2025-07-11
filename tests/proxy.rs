use std::net::SocketAddr;
use std::sync::Arc;

use event_listener::Event;
use futures_lite::AsyncReadExt;
use futures_lite::AsyncWriteExt;
use futures_lite::future::zip;
use futures_util::stream::StreamExt;
use rustls::pki_types::ServerName;
use tracing::debug;

use fluvio_future::net::{TcpListener, TcpStream};
use fluvio_future::rust_tls::{TlsAcceptor, TlsConnector};

use flv_tls_proxy::ProxyBuilder;

const CA_PATH: &str = "certs/certs/ca.crt";

const SERVER: &str = "127.0.0.1:19998";
const PROXY: &str = "127.0.0.1:20000";
const ITER: u16 = 1000;

#[fluvio_future::test]
async fn test_proxy() {
    run_test(
        fluvio_future::rust_tls::AcceptorBuilder::with_safe_defaults()
            .no_client_authentication()
            .load_server_certs("certs/certs/server.crt", "certs/certs/server.key")
            .unwrap()
            .build(),
        fluvio_future::rust_tls::ConnectorBuilder::with_safe_defaults()
            .load_ca_cert(CA_PATH)
            .unwrap()
            .load_client_certs("certs/certs/server.crt", "certs/certs/server.key")
            .unwrap()
            .build(),
    )
    .await
}

async fn run_test(acceptor: TlsAcceptor, connector: TlsConnector) {
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

            debug!(i, n, "server: loop received reply back bytes");
            let mut str_bytes = vec![];
            for item in buf.into_iter().take(n) {
                str_bytes.push(item);
            }
            let message = String::from_utf8(str_bytes).expect("utf8");
            debug!(i, message, "server: loop, received message");
            assert_eq!(message, format!("message{}", i));
            let resply = format!("{}reply", message);
            let reply_bytes = resply.as_bytes();
            debug!(resply, "sever: send back reply");
            tcp_stream
                .write_all(reply_bytes)
                .await
                .expect("send failed");
        }

        debug!("server done");
    };

    let client_ft = async {
        debug!("client: sleep to give server chance to come up");

        debug!("client: trying to connect");
        let tcp_stream = TcpStream::connect(PROXY.to_owned())
            .await
            .expect("connection fail");
        let server = ServerName::try_from("localhost").expect("server name parse failed");
        let mut tls_stream = connector
            .connect(server, tcp_stream)
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
            for item in buf.into_iter().take(n) {
                str_bytes.push(item);
            }
            let reply_message = String::from_utf8(str_bytes).expect("utf8");
            debug!(
                "client: loop {}, received reply message: {}",
                i, reply_message
            );
            assert_eq!(reply_message, format!("message{}reply", i));
        }

        // Properly shutdown the TLS connection
        debug!("client: shutting down TLS connection");
        if let Err(err) = tls_stream.close().await {
            debug!("client: TLS shutdown warning: {}", err);
        }

        debug!("client done");
        event.notify(1);
    };

    let proxy = ProxyBuilder::new(PROXY.to_owned(), acceptor, SERVER.to_string())
        .with_terminate(event.clone());

    let _ = zip(proxy.start(), zip(client_ft, server_ft)).await;

    // give little time to flush everything out
}
