use std::{io::Error as IoError, sync::Arc};

use anyhow::Result;
use event_listener::Event;
use futures_util::stream::StreamExt;
use tracing::{debug, error, info, instrument};

use fluvio_future::net::TcpStream;
use fluvio_future::rust_tls::{DefaultServerTlsStream, TlsAcceptor};

type TerminateEvent = Arc<Event>;

use crate::authenticator::{Authenticator, NullAuthenticator};

type SharedAuthenticator = Arc<Box<dyn Authenticator>>;

/// start TLS proxy at addr to target
pub async fn start(addr: &str, acceptor: TlsAcceptor, target: String) -> Result<(), IoError> {
    let builder = ProxyBuilder::new(addr.to_string(), acceptor, target);
    builder.start().await
}

/// start TLS proxy with authenticator at addr to target
pub async fn start_with_authenticator(
    addr: &str,
    acceptor: TlsAcceptor,
    target: String,
    authenticator: Box<dyn Authenticator>,
) -> Result<(), IoError> {
    let builder =
        ProxyBuilder::new(addr.to_string(), acceptor, target).with_authenticator(authenticator);
    builder.start().await
}

pub struct ProxyBuilder {
    addr: String,
    acceptor: TlsAcceptor,
    target: String,
    authenticator: Box<dyn Authenticator>,
    terminate: TerminateEvent,
}

impl ProxyBuilder {
    pub fn new(addr: String, acceptor: TlsAcceptor, target: String) -> Self {
        Self {
            addr,
            acceptor,
            target,
            authenticator: Box::new(NullAuthenticator),
            terminate: Arc::new(Event::new()),
        }
    }

    pub fn with_authenticator(mut self, authenticator: Box<dyn Authenticator>) -> Self {
        self.authenticator = authenticator;
        self
    }

    pub fn with_terminate(mut self, terminate: TerminateEvent) -> Self {
        self.terminate = terminate;
        self
    }

    #[instrument(skip(self))]
    pub async fn start(self) -> Result<(), IoError> {
        use tokio::select;

        use fluvio_future::net::TcpListener;
        use fluvio_future::task::spawn;

        let listener = TcpListener::bind(&self.addr).await?;
        info!(self.addr, "proxy started at");
        let mut incoming = listener.incoming();
        let shared_authenticator = Arc::new(self.authenticator);

        loop {
            select! {
                _ = self.terminate.listen() => {
                    info!("terminate event received");
                    return Ok(());
                }
                incoming_stream = incoming.next() => {
                    if let Some(stream) = incoming_stream {
                        debug!("server: got connection from client");
                        if let Ok(tcp_stream) = stream {
                            let acceptor = self.acceptor.clone();
                            let target = self.target.clone();
                            spawn(process_stream(
                                acceptor,
                                tcp_stream,
                                target,
                                shared_authenticator.clone()
                            ));
                        } else {
                            error!("no stream detected");
                            return Ok(());
                        }

                    } else {
                        info!("no more incoming streaming");
                        return Ok(());
                    }
                }

            }
        }
    }
}

/// start TLS stream at addr to target
#[instrument(skip(acceptor, raw_stream, authenticator))]
async fn process_stream(
    acceptor: TlsAcceptor,
    raw_stream: TcpStream,
    target: String,
    authenticator: SharedAuthenticator,
) {
    let source = raw_stream
        .peer_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| "".to_owned());

    info!(source, "new connection from");

    let handshake = acceptor.accept(raw_stream).await;

    match handshake {
        Ok(inner_stream) => {
            info!(source, "handshake success");
            if let Err(err) = proxy(inner_stream, target, source.clone(), authenticator).await {
                error!(?source, ?err, "error processing tls");
            }
        }
        Err(err) => error!(?source, ?err, "error handshaking"),
    }
}

#[instrument(skip(tls_stream, authenticator))]
async fn proxy(
    tls_stream: DefaultServerTlsStream,
    target: String,
    source: String,
    authenticator: SharedAuthenticator,
) -> Result<()> {
    use tokio_util::compat::FuturesAsyncReadCompatExt;

    debug!("trying to connect to target");
    let tcp_stream = TcpStream::connect(&target).await?;
    info!("open tcp stream");

    let auth_success = authenticator.authenticate(&tls_stream, &tcp_stream).await?;
    if !auth_success {
        info!("authentication failed, dropping connection");
        return Ok(());
    } else {
        info!("authentication succeeded");
    }

    debug!(?source, ?target, "starting bidirectional copy between",);

    // Convert futures AsyncRead/AsyncWrite to tokio AsyncRead/AsyncWrite
    let mut tls_compat = tls_stream.compat();
    let mut tcp_compat = tcp_stream.compat();

    match tokio::io::copy_bidirectional(&mut tls_compat, &mut tcp_compat).await {
        Ok((tls_to_target_bytes, target_to_tls_bytes)) => {
            info!(
                ?source,
                ?tls_to_target_bytes,
                ?target_to_tls_bytes,
                "proxy connection completed",
            );
        }
        Err(err) => {
            error!(?source, ?err, "error in bidirectional");
        }
    }

    Ok(())
}
