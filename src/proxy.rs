use std::{io::Error as IoError, sync::Arc};

use event_listener::Event;
use futures_lite::io::copy;
use futures_util::io::AsyncReadExt;
use futures_util::stream::StreamExt;
use log::debug;
use log::error;
use log::info;

use fluvio_future::net::TcpStream;

use crate::tls::DefaultServerTlsStream;
use crate::tls::TlsAcceptor;

type TerminateEvent = Arc<Event>;

use crate::authenticator::{Authenticator, NullAuthenticator};

type SharedAuthenticator = Arc<Box<dyn Authenticator>>;

pub async fn start(addr: &str, acceptor: TlsAcceptor, target: String) -> Result<(), IoError> {
    let builder = ProxyBuilder::new(addr.to_string(), acceptor, target);
    builder.start().await
}

/// start TLS proxy at addr to target

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

    pub async fn start(self) -> Result<(), IoError> {
        use tokio::select;

        use fluvio_future::net::TcpListener;
        use fluvio_future::task::spawn;

        let listener = TcpListener::bind(&self.addr).await?;
        info!("proxy started at: {}", self.addr);
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

/// start TLS proxy at addr to target

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

    debug!("new connection from {}", source);

    let handshake = acceptor.accept(raw_stream);

    match handshake.await {
        Ok(inner_stream) => {
            debug!("handshake success from: {}", source);
            if let Err(err) = proxy(inner_stream, target, source.clone(), authenticator).await {
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
    authenticator: SharedAuthenticator,
) -> Result<(), IoError> {
    use futures_lite::future::zip;

    debug!(
        "trying to connect to target at: {} from source: {}",
        target, source
    );
    let mut tcp_stream = TcpStream::connect(&target).await?;

    let auth_success = authenticator.authenticate(&tls_stream, &tcp_stream).await?;
    if !auth_success {
        debug!("authentication failed, dropping connection");
        return Ok(());
    } else {
        debug!("authentication succeeded");
    }

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
