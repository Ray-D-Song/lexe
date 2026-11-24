// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Optional hook for `https.createServer`: after TLS handshake, hand the cleartext
//! stream to `http.Server` instead of running the default TLSSocket I/O loop.

use std::pin::Pin;
use std::sync::OnceLock;

use rquickjs::{Class, Ctx, Result, Value};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::tls_socket::TlsSocket;

pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncReadWrite for T {}

type HandoffFn = for<'js> fn(
    ctx: Ctx<'js>,
    http_server: Value<'js>,
    tls_socket: Class<'js, TlsSocket<'js>>,
    stream: Pin<Box<dyn AsyncReadWrite + 'static>>,
) -> Result<()>;

static HANDOFF: OnceLock<HandoffFn> = OnceLock::new();

/// Registered by `lexe_http` when the https module loads.
pub fn register_http_handoff(handler: HandoffFn) {
    let _ = HANDOFF.set(handler);
}

pub(crate) fn http_handoff_registered() -> bool {
    HANDOFF.get().is_some()
}

pub(crate) fn handoff_to_http<'js, IO>(
    ctx: Ctx<'js>,
    http_server: Value<'js>,
    tls_socket: Class<'js, TlsSocket<'js>>,
    stream: IO,
) -> Result<()>
where
    IO: AsyncReadWrite + 'static,
{
    let handler = HANDOFF
        .get()
        .ok_or_else(|| rquickjs::Exception::throw_message(&ctx, "HTTP handoff not registered"))?;
    handler(ctx, http_server, tls_socket, Box::pin(stream))
}
