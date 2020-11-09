// copy from futures_util
// add tracing support

use core::task::{Context, Poll};
use std::io;
use std::pin::Pin;

use futures_util::future::Future;
use futures_util::io::AsyncBufRead as BufRead;
use futures_util::io::AsyncRead as Read;
use futures_util::io::AsyncWrite as Write;
use futures_util::io::BufReader;
use futures_util::ready;
use log::trace;
use pin_project_lite::pin_project;

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
            loop_index: u64,
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
                trace!(
                    "{}, loop {} with amt {}",
                    this.label,
                    this.loop_index,
                    this.amt
                );
                let buffer = ready!(this.reader.as_mut().poll_fill_buf(cx))?;
                if buffer.is_empty() {
                    trace!("{}, buffer is empty, flushing and exiting", this.label);
                    ready!(this.writer.as_mut().poll_flush(cx))?;
                    return Poll::Ready(Ok(*this.amt));
                }

                trace!("{}, read {} bytes", this.label, buffer.len());
                let i = ready!(this.writer.as_mut().poll_write(cx, buffer))?;
                trace!("{}, write {} bytes", this.label, i);
                if i == 0 {
                    trace!("{}, no write occuring, returing with ready", this.label);
                    return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
                }
                *this.amt += i as u64;
                // trace!("{},consuming amt: {}", this.label, this.amt);
                this.reader.as_mut().consume(i);
                *this.loop_index += 1;
            }
        }
    }

    let future = CopyFuture {
        reader: BufReader::new(reader),
        writer,
        amt: 0,
        loop_index: 0,
        label,
    };
    future.await
}
