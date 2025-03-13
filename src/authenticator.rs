use anyhow::Result;
use async_trait::async_trait;

use fluvio_future::net::TcpStream;
use fluvio_future::openssl::DefaultServerTlsStream;

/// Abstracts logic to authenticate incoming stream and forward authoization context to target
#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(
        &self,
        incoming_tls_stream: &DefaultServerTlsStream,
        target_tcp_stream: &TcpStream,
    ) -> Result<bool>;
}

/// Null implementation where authenticate always returns true
pub(crate) struct NullAuthenticator;

#[async_trait]
impl Authenticator for NullAuthenticator {
    async fn authenticate(&self, _: &DefaultServerTlsStream, _: &TcpStream) -> Result<bool> {
        Ok(true)
    }
}
