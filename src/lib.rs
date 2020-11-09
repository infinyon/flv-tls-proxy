mod copy;
mod proxy;
pub use proxy::*;

pub mod authenticator;

pub mod tls {

    cfg_if::cfg_if! {
        if #[cfg(feature = "rust_tls")] {
            pub use fluvio_future::rust_tls::DefaultServerTlsStream;
            pub use fluvio_future::rust_tls::TlsAcceptor;
        } else if #[cfg(feature  = "native_tls")] {
            pub use fluvio_future::native_tls::DefaultServerTlsStream;
            pub use fluvio_future::native_tls::TlsAcceptor;
        }
    }
}
