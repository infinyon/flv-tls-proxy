use async_trait::async_trait;
use fluvio_future::net::TcpStream;
use fluvio_future::tls::DefaultServerTlsStream;

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(
        &self,
        incoming_tls_stream: &DefaultServerTlsStream,
        target_tcp_stream: &TcpStream,
    ) -> Result<bool, std::io::Error>;
}

pub(crate) struct NullAuthenticator;

#[async_trait]
impl Authenticator for NullAuthenticator {
    async fn authenticate(
        &self,
        _: &DefaultServerTlsStream,
        _: &TcpStream,
    ) -> Result<bool, std::io::Error> {
        Ok(true)
    }
}
