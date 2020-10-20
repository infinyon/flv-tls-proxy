use std::io::Error as IoError;

use fluvio_future::net::TcpListener;
use fluvio_future::net::TcpStream;
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
    use async_std::task::spawn_local;

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
        match copy(from_tls_stream, &mut target_sink).await {
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

    spawn_local(source_to_target_ft);
    spawn_local(target_to_source);

    Ok(())
}
