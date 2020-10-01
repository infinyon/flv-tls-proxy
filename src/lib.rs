use std::io::Error as IoError;

use futures_lite::io::copy;
use fluvio_future::net::TcpListener;
use fluvio_future::net::TcpStream;
use fluvio_future::task::spawn;
use fluvio_future::tls::DefaultServerTlsStream;
use fluvio_future::tls::TlsAcceptor;


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
        .unwrap_or("".to_owned());

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



/*
mod copy {

    use std::io;
    use std::pin::Pin;

    use core::task::{Context, Poll};
    use log::trace;
    use pin_project_lite::pin_project;

    use futures_util::future::Future;
    use futures_util::io::AsyncBufRead as BufRead;
    use futures_util::io::AsyncRead as Read;
    use futures_util::io::AsyncWrite as Write;
    use futures_util::io::BufReader;
    use futures_util::ready;

    pub async fn copy<R, W>(reader: &mut R, writer: &mut W, label: String) -> io::Result<u64>
    where
        R: Read + Unpin + ?Sized,
        W: Write + Unpin + ?Sized,
    {
        pin_project! {
            struct CopyFuture<R, W> {
                #[pin]
                reader: R,
                #[pin]
                writer: W,
                amt: u64,
                #[pin]
                label: String
            }
        }

        impl<R, W> Future for CopyFuture<R, W>
        where
            R: BufRead,
            W: Write + Unpin,
        {
            type Output = io::Result<u64>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let mut this = self.project();

                loop {
                    trace!("{}, starting loop with amt {}", this.label, this.amt);
                    let buffer = ready!(this.reader.as_mut().poll_fill_buf(cx))?;
                    if buffer.is_empty() {
                        trace!("{}, buffer is empty, flushing", this.label);
                        ready!(this.writer.as_mut().poll_flush(cx))?;
                        return Poll::Ready(Ok(*this.amt));
                    }

                    trace!("{}, read {}", this.label, buffer.len());
                    let i = ready!(this.writer.as_mut().poll_write(cx, buffer))?;
                    trace!("{}, write: {}", this.label, i);
                    if i == 0 {
                        trace!("{}, no write occur returning", this.label);
                        return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
                    }
                    *this.amt += i as u64;
                    trace!("{},consuming amt: {}", this.label, this.amt);
                    this.reader.as_mut().consume(i);
                }
            }
        }

        let future = CopyFuture {
            reader: BufReader::new(reader),
            writer,
            amt: 0,
            label,
        };
        future.await
    }
}
*/