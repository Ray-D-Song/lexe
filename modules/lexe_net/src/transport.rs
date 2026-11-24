// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{tcp::OwnedWriteHalf, TcpStream},
};

pub struct PrefixedTcpStream {
    prefix: io::Cursor<Vec<u8>>,
    read: tokio::net::tcp::OwnedReadHalf,
    write: OwnedWriteHalf,
}

impl PrefixedTcpStream {
    pub fn new(
        prefix: Vec<u8>,
        read: tokio::net::tcp::OwnedReadHalf,
        write: OwnedWriteHalf,
    ) -> Self {
        Self {
            prefix: io::Cursor::new(prefix),
            read,
            write,
        }
    }

    pub fn from_reunited(prefix: Vec<u8>, stream: TcpStream) -> Self {
        let (read, write) = stream.into_split();
        Self::new(prefix, read, write)
    }
}

impl AsyncRead for PrefixedTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.prefix.position() < self.prefix.get_ref().len() as u64 {
            let pos = self.prefix.position() as usize;
            let remaining = &self.prefix.get_ref()[pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            self.prefix.set_position((pos + to_copy) as u64);
            return Poll::Ready(Ok(()));
        }

        Pin::new(&mut self.read).poll_read(cx, buf)
    }
}

impl AsyncWrite for PrefixedTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.write).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_shutdown(cx)
    }
}
