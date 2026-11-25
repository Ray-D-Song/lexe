// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf, ReadHalf, WriteHalf},
    sync::{mpsc, oneshot},
};

pub(crate) enum WriteCmd {
    Bytes(Vec<u8>),
    InjectCloseNextHeader,
    Flush(oneshot::Sender<io::Result<()>>),
    Shutdown(oneshot::Sender<io::Result<()>>),
}

#[derive(Clone)]
struct SharedWriteError(Arc<Mutex<Option<(io::ErrorKind, String)>>>);

impl SharedWriteError {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    fn set(&self, err: &io::Error) {
        let mut guard = self.0.lock().unwrap();
        if guard.is_none() {
            *guard = Some((err.kind(), err.to_string()));
        }
    }

    fn take_or_broken_pipe(&self) -> io::Error {
        if let Some((kind, msg)) = self.0.lock().unwrap().as_ref() {
            io::Error::new(*kind, msg.clone())
        } else {
            io::Error::new(io::ErrorKind::BrokenPipe, "HTTP writer closed")
        }
    }

    fn current(&self) -> Option<io::Error> {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .map(|(kind, msg)| io::Error::new(*kind, msg.clone()))
    }
}

pub(crate) struct HttpServerIo<T> {
    read: ReadHalf<T>,
    write_tx: mpsc::UnboundedSender<WriteCmd>,
    write_err: SharedWriteError,
    flush_rx: Option<oneshot::Receiver<io::Result<()>>>,
    shutdown_rx: Option<oneshot::Receiver<io::Result<()>>>,
}

impl<T> HttpServerIo<T>
where
    T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    pub(crate) fn split_with_writer(
        stream: T,
    ) -> (
        Self,
        mpsc::UnboundedSender<WriteCmd>,
        tokio::task::JoinHandle<()>,
    ) {
        let (read, write) = tokio::io::split(stream);
        let (write_tx, write_rx) = mpsc::unbounded_channel();
        let write_err = SharedWriteError::new();
        let writer_handle = tokio::spawn(run_writer_task(write, write_rx, write_err.clone()));
        let io = Self {
            read,
            write_tx: write_tx.clone(),
            write_err,
            flush_rx: None,
            shutdown_rx: None,
        };
        (io, write_tx, writer_handle)
    }

    pub(crate) fn into_read_half(self) -> ReadHalf<T> {
        self.read
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for HttpServerIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.read).poll_read(cx, buf)
    }
}

impl<T: Unpin> AsyncWrite for HttpServerIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(err) = self.write_err.current() {
            return Poll::Ready(Err(err));
        }
        if self.write_tx.send(WriteCmd::Bytes(buf.to_vec())).is_err() {
            return Poll::Ready(Err(self.write_err.take_or_broken_pipe()));
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(err) = self.write_err.current() {
            self.flush_rx = None;
            return Poll::Ready(Err(err));
        }
        if self.flush_rx.is_none() {
            let (tx, rx) = oneshot::channel();
            if self.write_tx.send(WriteCmd::Flush(tx)).is_err() {
                return Poll::Ready(Err(self.write_err.take_or_broken_pipe()));
            }
            self.flush_rx = Some(rx);
        }
        let rx = self.flush_rx.as_mut().expect("flush_rx initialized");
        match Pin::new(rx).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(res)) => {
                self.flush_rx = None;
                Poll::Ready(res)
            },
            Poll::Ready(Err(_)) => {
                self.flush_rx = None;
                Poll::Ready(Err(self.write_err.take_or_broken_pipe()))
            },
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(err) = self.write_err.current() {
            self.shutdown_rx = None;
            return Poll::Ready(Err(err));
        }
        if self.shutdown_rx.is_none() {
            let (tx, rx) = oneshot::channel();
            if self.write_tx.send(WriteCmd::Shutdown(tx)).is_err() {
                return Poll::Ready(Err(self.write_err.take_or_broken_pipe()));
            }
            self.shutdown_rx = Some(rx);
        }
        let rx = self.shutdown_rx.as_mut().expect("shutdown_rx initialized");
        match Pin::new(rx).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(res)) => {
                self.shutdown_rx = None;
                Poll::Ready(res)
            },
            Poll::Ready(Err(_)) => {
                self.shutdown_rx = None;
                Poll::Ready(Err(self.write_err.take_or_broken_pipe()))
            },
        }
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn is_1xx_status_line(line: &[u8]) -> bool {
    let Some(space) = line.iter().position(|&b| b == b' ') else {
        return false;
    };
    let rest = &line[space + 1..];
    rest.len() >= 3 && rest[0] == b'1' && rest[1].is_ascii_digit() && rest[2].is_ascii_digit()
}

/// Rewrite one serialized HTTP/1.x response header block (`head` without `\r\n\r\n`)
/// so that it carries `connection: close` (replacing `connection: keep-alive` if any).
fn rewrite_header_with_close(head: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let Some(status_line) = lines.next() else {
        return head.to_vec();
    };
    let mut out = String::with_capacity(head.len() + 24);
    out.push_str(status_line);
    let mut has_close = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, val)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("connection") {
                let lower_val = val.trim().to_ascii_lowercase();
                if lower_val == "keep-alive" {
                    continue;
                }
                if lower_val.split(',').any(|p| p.trim() == "close") {
                    has_close = true;
                }
            }
        }
        out.push_str("\r\n");
        out.push_str(line);
    }
    if !has_close {
        out.push_str("\r\nconnection: close");
    }
    out.push_str("\r\n\r\n");
    out.into_bytes()
}

async fn write_bytes_checked<T: AsyncWrite + Unpin>(
    writer: &mut WriteHalf<T>,
    bytes: &[u8],
    err_store: &SharedWriteError,
) -> bool {
    if let Err(err) = writer.write_all(bytes).await {
        err_store.set(&err);
        false
    } else {
        true
    }
}

async fn run_writer_task<T: AsyncWrite + Unpin>(
    mut writer: WriteHalf<T>,
    mut write_rx: mpsc::UnboundedReceiver<WriteCmd>,
    err_store: SharedWriteError,
) {
    let mut inject_close_next = false;
    let mut pending_head: Vec<u8> = Vec::new();
    let mut failed = false;

    while let Some(cmd) = write_rx.recv().await {
        match cmd {
            WriteCmd::InjectCloseNextHeader => {
                if !failed && !pending_head.is_empty() {
                    let drained = std::mem::take(&mut pending_head);
                    if !write_bytes_checked(&mut writer, &drained, &err_store).await {
                        failed = true;
                    }
                }
                inject_close_next = true;
            },
            WriteCmd::Bytes(bytes) => {
                if failed {
                    continue;
                }
                if !inject_close_next {
                    if !write_bytes_checked(&mut writer, &bytes, &err_store).await {
                        failed = true;
                    }
                    continue;
                }
                pending_head.extend_from_slice(&bytes);
                while inject_close_next {
                    let Some(end) = find_header_end(&pending_head) else {
                        if pending_head.len() > 256 * 1024 {
                            // Safety bound: stop buffering if malformed large write arrives.
                            let drained = std::mem::take(&mut pending_head);
                            if !write_bytes_checked(&mut writer, &drained, &err_store).await {
                                failed = true;
                            }
                            inject_close_next = false;
                        }
                        break;
                    };
                    let head_block: Vec<u8> = pending_head.drain(..end + 4).collect();
                    let first_line_end = head_block
                        .windows(2)
                        .position(|w| w == b"\r\n")
                        .unwrap_or(end);
                    if is_1xx_status_line(&head_block[..first_line_end]) {
                        if !write_bytes_checked(&mut writer, &head_block, &err_store).await {
                            failed = true;
                            inject_close_next = false;
                            break;
                        }
                        continue;
                    }
                    let rewritten = rewrite_header_with_close(&head_block[..end]);
                    inject_close_next = false;
                    if !write_bytes_checked(&mut writer, &rewritten, &err_store).await {
                        failed = true;
                        break;
                    }
                    if !pending_head.is_empty() {
                        let rest = std::mem::take(&mut pending_head);
                        if !write_bytes_checked(&mut writer, &rest, &err_store).await {
                            failed = true;
                        }
                    }
                }
            },
            WriteCmd::Flush(ack) => {
                if !failed && !pending_head.is_empty() {
                    let drained = std::mem::take(&mut pending_head);
                    if !write_bytes_checked(&mut writer, &drained, &err_store).await {
                        failed = true;
                    }
                    inject_close_next = false;
                }
                if failed {
                    let _ = ack.send(Err(err_store.take_or_broken_pipe()));
                    continue;
                }
                match writer.flush().await {
                    Ok(()) => {
                        let _ = ack.send(Ok(()));
                    },
                    Err(err) => {
                        err_store.set(&err);
                        failed = true;
                        let _ = ack.send(Err(err));
                    },
                }
            },
            WriteCmd::Shutdown(ack) => {
                if !failed && !pending_head.is_empty() {
                    let drained = std::mem::take(&mut pending_head);
                    if !write_bytes_checked(&mut writer, &drained, &err_store).await {
                        failed = true;
                    }
                }
                if failed {
                    let _ = ack.send(Err(err_store.take_or_broken_pipe()));
                    break;
                }
                match writer.shutdown().await {
                    Ok(()) => {
                        let _ = ack.send(Ok(()));
                    },
                    Err(err) => {
                        err_store.set(&err);
                        let _ = ack.send(Err(err));
                    },
                }
                break;
            },
        }
    }
}
