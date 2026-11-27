// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! `http.WebSocket` — the Node 22+ WebSocket client.
//!
//! The transport is the same pooled hyper client the rest of this crate uses,
//! so TLS goes through the path fixed for `https.get`. The frame layer is
//! `fastwebsockets`, which already implements RFC 6455 framing: fragmentation
//! reassembly, masking, 7/16/64-bit payload lengths, ping/pong and close codes.
//! This module maps that onto the WebSocket API Node exposes — it does not
//! reimplement the protocol.

use std::sync::{Arc, RwLock};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::{header, Method, Request, StatusCode};
use rquickjs::{
    class::{Trace, Tracer},
    prelude::{Opt, Rest, This},
    Class, Ctx, Exception, IntoJs, JsLifetime, Object, Result, Value,
};
use tokio::sync::mpsc;

use lexe_context::CtxExtension;
use lexe_events::{Emitter, EventEmitter, EventList};

/// `WebSocket.readyState`, matching Node and the browser.
pub const CONNECTING: i32 = 0;
pub const OPEN: i32 = 1;
pub const CLOSING: i32 = 2;
pub const CLOSED: i32 = 3;

/// What the JS thread hands to the connection task, already resolved to bytes.
enum Outgoing {
    Text(String),
    Binary(Vec<u8>),
    Close(u16, Vec<u8>),
}

#[rquickjs::class(rename = "WebSocket")]
pub struct WebSocket<'js> {
    emitter: EventEmitter<'js>,
    url: String,
    state: i32,
    binary_type: String,
    protocol: String,
    outgoing: Option<mpsc::UnboundedSender<Outgoing>>,
    /// Set when a `message`/`close`/`error` payload could not be delivered
    /// because the runtime was already tearing down.
    closed_notified: bool,
}

unsafe impl<'js> JsLifetime<'js> for WebSocket<'js> {
    type Changed<'to> = WebSocket<'to>;
}

impl<'js> Trace<'js> for WebSocket<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
    }
}

impl<'js> Emitter<'js> for WebSocket<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

/// A `message` event payload. Node passes a `MessageEvent` whose `data` is a
/// string for text frames, or the `binaryType` representation otherwise.
fn message_event<'js>(ctx: &Ctx<'js>, data: Value<'js>) -> Result<Object<'js>> {
    let event = Object::new(ctx.clone())?;
    event.set("type", "message")?;
    event.set("data", data)?;
    Ok(event)
}

fn close_event<'js>(
    ctx: &Ctx<'js>,
    code: u16,
    reason: &str,
    was_clean: bool,
) -> Result<Object<'js>> {
    let event = Object::new(ctx.clone())?;
    event.set("type", "close")?;
    event.set("code", code)?;
    event.set("reason", reason)?;
    event.set("wasClean", was_clean)?;
    Ok(event)
}

fn error_event<'js>(ctx: &Ctx<'js>, message: &str) -> Result<Object<'js>> {
    let event = Object::new(ctx.clone())?;
    event.set("type", "error")?;
    // Node's error event carries the failure as `error`; a plain `message`
    // keeps it inspectable either way.
    event.set("message", message)?;
    event.set("error", Exception::from_message(ctx.clone(), message)?)?;
    Ok(event)
}

impl<'js> WebSocket<'js> {
    fn set_state(&mut self, state: i32) {
        self.state = state;
    }

    /// Build the upgrade request for `url`.
    ///
    /// `ws:`/`wss:` are rewritten to `http:`/`https:` for hyper; the pooled
    /// client already knows how to reach both.
    fn handshake_request(
        url: &str,
    ) -> std::result::Result<(Request<Empty<Bytes>>, String), String> {
        let parsed: hyper::Uri = url
            .parse()
            .map_err(|_| format!("Invalid WebSocket URL: {url}"))?;
        let scheme = parsed.scheme_str().unwrap_or("");
        let http_scheme = match scheme {
            "ws" => "http",
            "wss" => "https",
            _ => return Err(format!("WebSocket URL must use ws: or wss:, got {url}")),
        };

        let mut parts = parsed.clone().into_parts();
        parts.scheme = Some(
            http_scheme
                .parse()
                .expect("http/https are valid scheme names"),
        );
        let uri =
            hyper::Uri::from_parts(parts).map_err(|_| format!("Invalid WebSocket URL: {url}"))?;

        let key = generate_key();
        let host = uri
            .authority()
            .map(|a| a.as_str().to_string())
            .unwrap_or_default();

        let request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header(header::HOST, host)
            .header(header::CONNECTION, "Upgrade")
            .header(header::UPGRADE, "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", &key)
            .body(Empty::new())
            .map_err(|err| format!("Invalid WebSocket request: {err}"))?;
        Ok((request, key))
    }

    /// Drive one connection to completion and report how it ended.
    async fn connect_and_pump(
        this: Class<'js, Self>,
        ctx: Ctx<'js>,
        client: crate::client::HyperClient,
        request: Request<Empty<Bytes>>,
        key: String,
        outgoing: mpsc::UnboundedReceiver<Outgoing>,
    ) -> Result<Ended> {
        let response = client
            .request(request.map(|body| {
                body.map_err(|never: std::convert::Infallible| match never {})
                    .boxed()
            }))
            .await
            .map_err(|err| {
                Exception::throw_message(&ctx, &format!("WebSocket handshake failed: {err}"))
            })?;

        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err(Exception::throw_message(
                &ctx,
                &format!(
                    "WebSocket handshake rejected with status {}",
                    response.status().as_u16()
                ),
            ));
        }
        // A proxy or a broken server can answer 101 without being a WebSocket.
        // Refusing here keeps garbage out of the frame layer.
        if !header_contains(response.headers(), header::CONNECTION, "upgrade") {
            return Err(Exception::throw_message(
                &ctx,
                "WebSocket handshake failed: missing Connection: Upgrade",
            ));
        }
        if !header_contains(response.headers(), header::UPGRADE, "websocket") {
            return Err(Exception::throw_message(
                &ctx,
                "WebSocket handshake failed: missing Upgrade: websocket",
            ));
        }
        // And the accept token must be the SHA-1 of *our* key: accepting a
        // mismatched one would let any 101 through.
        let expected = accept_for_key(&key);
        match response.headers().get("Sec-WebSocket-Accept") {
            Some(value) if value.as_bytes() == expected.as_bytes() => {},
            Some(_) => {
                return Err(Exception::throw_message(
                    &ctx,
                    "WebSocket handshake failed: invalid Sec-WebSocket-Accept",
                ))
            },
            None => {
                return Err(Exception::throw_message(
                    &ctx,
                    "WebSocket handshake failed: missing Sec-WebSocket-Accept",
                ))
            },
        }

        let upgraded = hyper::upgrade::on(response).await.map_err(|err| {
            Exception::throw_message(&ctx, &format!("WebSocket upgrade failed: {err}"))
        })?;
        let io = hyper_util::rt::TokioIo::new(upgraded);
        let mut socket =
            fastwebsockets::WebSocket::after_handshake(io, fastwebsockets::Role::Client);
        // Masking and ping replies are the frame layer's job; JS should never
        // have to think about either.
        socket.set_auto_apply_mask(true);
        socket.set_auto_pong(true);
        socket.set_max_message_size(64 * 1024 * 1024);

        this.borrow_mut().set_state(OPEN);
        Self::emit_str(this.clone(), &ctx, "open", vec![], false)?;

        Self::pump(this, ctx, socket, outgoing).await
    }

    /// Turn whatever the connection produced into the terminal events.
    fn finish(this: &Class<'js, Self>, ctx: &Ctx<'js>, outcome: Result<Ended>) -> Result<()> {
        {
            let mut borrow = this.borrow_mut();
            if borrow.closed_notified {
                return Ok(());
            }
            borrow.closed_notified = true;
            borrow.set_state(CLOSED);
            // Drop the write channel so the connection task cannot be kept
            // alive by it, and the object becomes collectable once JS drops it.
            borrow.outgoing = None;
        }
        match outcome {
            Ok(Ended::PeerClosed {
                code,
                reason,
                clean,
            }) => {
                let event = close_event(ctx, code, &reason, clean)?;
                Self::emit_str(this.clone(), ctx, "close", vec![event.into_value()], false)?;
            },
            Ok(Ended::LocallyClosed { code, reason }) => {
                let event = close_event(ctx, code, &reason, true)?;
                Self::emit_str(this.clone(), ctx, "close", vec![event.into_value()], false)?;
            },
            Ok(Ended::TransportEnded) => {
                let event = close_event(ctx, 1006, "", false)?;
                Self::emit_str(this.clone(), ctx, "close", vec![event.into_value()], false)?;
            },
            Err(err) => {
                // `rquickjs::Error`'s Display hides the text of a thrown
                // exception ("Exception generated by QuickJS"), so surface the
                // JS error's own message when there is one.
                let message = error_text(ctx, err);
                let event = error_event(ctx, &message)?;
                Self::emit_str(this.clone(), ctx, "error", vec![event.into_value()], false)?;
                let event = close_event(ctx, 1006, "", false)?;
                Self::emit_str(this.clone(), ctx, "close", vec![event.into_value()], false)?;
            },
        }
        Ok(())
    }

    async fn pump(
        this: Class<'js, Self>,
        ctx: Ctx<'js>,
        mut socket: fastwebsockets::WebSocket<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>,
        mut outgoing: mpsc::UnboundedReceiver<Outgoing>,
    ) -> Result<Ended> {
        use fastwebsockets::{Frame, OpCode, Payload};

        // Fragmented messages arrive frame by frame; these hold the pieces
        // until the final frame.
        let mut frag_buffer: Vec<u8> = Vec::new();
        let mut frag_opcode: Option<OpCode> = None;

        loop {
            tokio::select! {
                maybe = outgoing.recv() => {
                    let Some(item) = maybe else { break };
                    // Owned payloads: the frame borrows its body and has to
                    // outlive the `write_frame` call below.
                    let (frame, closing) = match item {
                        Outgoing::Text(text) => {
                            (Frame::text(Payload::Owned(text.into_bytes())), None)
                        },
                        Outgoing::Binary(bytes) => (Frame::binary(Payload::Owned(bytes)), None),
                        Outgoing::Close(code, reason) => {
                            let reason_text = String::from_utf8_lossy(&reason).into_owned();
                            (
                                Frame::close(code, &reason),
                                Some((code, reason_text)),
                            )
                        },
                    };
                    socket
                        .write_frame(frame)
                        .await
                        .map_err(|err| {
                            Exception::throw_message(&ctx, &format!("WebSocket write failed: {err}"))
                        })?;
                    if let Some((code, reason)) = closing {
                        return Ok(Ended::LocallyClosed { code, reason });
                    }
                }
                frame = socket.read_frame() => {
                    let frame = match frame {
                        Ok(frame) => frame,
                        Err(err) => {
                            if is_clean_disconnect(&err) {
                                return Ok(Ended::TransportEnded);
                            }
                            return Err(Exception::throw_message(
                                &ctx,
                                &format!("WebSocket read failed: {err}"),
                            ));
                        },
                    };
                    match frame.opcode {
                        OpCode::Text | OpCode::Binary | OpCode::Continuation => {
                            // `read_frame` yields one frame at a time, so a
                            // fragmented message arrives as Text/Binary with
                            // `fin == false` followed by Continuations. Accumulate
                            // until the final frame, then deliver one message.
                            if frame.opcode != OpCode::Continuation {
                                frag_opcode = Some(frame.opcode);
                                frag_buffer.clear();
                            }
                            frag_buffer.extend_from_slice(&frame.payload);
                            if !frame.fin {
                                continue;
                            }

                            let is_text = frag_opcode == Some(OpCode::Text);
                            let bytes = std::mem::take(&mut frag_buffer);
                            frag_opcode = None;

                            let data = if is_text {
                                match String::from_utf8(bytes) {
                                    Ok(text) => text.into_js(&ctx)?,
                                    Err(_) => {
                                        return Err(Exception::throw_message(
                                            &ctx,
                                            "WebSocket text frame was not valid UTF-8",
                                        ))
                                    },
                                }
                            } else {
                                Self::binary_payload(&this, &ctx, bytes)?
                            };
                            let event = message_event(&ctx, data)?;
                            Self::emit_str(
                                this.clone(),
                                &ctx,
                                "message",
                                vec![event.into_value()],
                                false,
                            )?;
                        },
                        OpCode::Ping | OpCode::Pong => {
                            // Both are handled inside the frame layer.
                        },
                        OpCode::Close => {
                            let (code, reason) = parse_close(&frame.payload);
                            return Ok(Ended::PeerClosed { code, reason, clean: true });
                        },
                    }
                }
            }
        }
        Ok(Ended::TransportEnded)
    }

    /// Convert a binary payload according to the socket's `binaryType`.
    fn binary_payload(
        this: &Class<'js, Self>,
        ctx: &Ctx<'js>,
        bytes: Vec<u8>,
    ) -> Result<Value<'js>> {
        let binary_type = this.borrow().binary_type.clone();
        let buffer = lexe_buffer::Buffer(bytes).into_js(ctx)?;
        match binary_type.as_str() {
            "arraybuffer" => {
                let object = buffer
                    .as_object()
                    .ok_or_else(|| Exception::throw_message(ctx, "buffer is not an object"))?;
                object.get::<_, Value>("buffer")
            },
            // Node also accepts "nodebuffer"; both give a Buffer here.
            _ => Ok(buffer),
        }
    }
}

#[rquickjs::methods]
impl<'js> WebSocket<'js> {
    #[qjs(constructor)]
    pub fn ctor(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Class<'js, Self>> {
        let mut args = args.0.into_iter();
        let url_value = args
            .next()
            .ok_or_else(|| Exception::throw_type(&ctx, "WebSocket requires a URL"))?;
        let url = url_value
            .as_string()
            .and_then(|text| text.to_string().ok())
            .ok_or_else(|| Exception::throw_type(&ctx, "WebSocket URL must be a string"))?;

        // `new WebSocket(url, protocols)` — a string or an array of them.
        let mut protocols: Vec<String> = Vec::new();
        match args.next() {
            Some(value) if value.is_string() => {
                if let Some(text) = value.as_string() {
                    protocols.push(text.to_string()?);
                }
            },
            Some(value) if value.is_array() => {
                let array = value.as_array().expect("array");
                for item in array.iter::<Value>() {
                    if let Some(text) = item?.as_string() {
                        protocols.push(text.to_string()?);
                    }
                }
            },
            _ => {},
        }

        let (request, key) = match Self::handshake_request(&url) {
            Ok(pair) => pair,
            Err(err) => return Err(Exception::throw_message(&ctx, &err)),
        };

        let client = match crate::HTTP_CLIENT.as_ref() {
            Ok(client) => client.clone(),
            Err(err) => return Err(Exception::throw_message(&ctx, &err.to_string())),
        };

        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel::<Outgoing>();
        let instance = Class::instance(
            ctx.clone(),
            Self {
                emitter: EventEmitter::new(),
                url,
                state: CONNECTING,
                binary_type: "blob".to_string(),
                protocol: protocols.first().cloned().unwrap_or_default(),
                outgoing: Some(outgoing_tx),
                closed_notified: false,
            },
        )?;

        let this = instance.clone();
        let ctx2 = ctx.clone();
        ctx.spawn_exit(async move {
            let this2 = this.clone();
            let outcome =
                Self::connect_and_pump(this2, ctx2.clone(), client, request, key, outgoing_rx)
                    .await;
            Self::finish(&this, &ctx2, outcome)?;
            Ok::<_, rquickjs::Error>(())
        })?;

        Ok(instance)
    }

    #[qjs(get, enumerable)]
    pub fn url(&self) -> String {
        self.url.clone()
    }

    #[qjs(get, rename = "readyState", enumerable)]
    pub fn ready_state(&self) -> i32 {
        self.state
    }

    #[qjs(get, rename = "binaryType", enumerable)]
    pub fn binary_type(&self) -> String {
        self.binary_type.clone()
    }

    #[qjs(set, rename = "binaryType")]
    pub fn set_binary_type(&mut self, value: String) {
        // Node throws on unknown values; the setter cannot report an error, so
        // an unsupported value leaves the previous one in place rather than
        // silently changing how payloads are represented.
        if value == "blob" || value == "arraybuffer" || value == "nodebuffer" {
            self.binary_type = value;
        }
    }

    #[qjs(get, enumerable)]
    pub fn protocol(&self) -> String {
        self.protocol.clone()
    }

    pub fn send(this: This<Class<'js, Self>>, ctx: Ctx<'js>, value: Value<'js>) -> Result<()> {
        if this.borrow().state != OPEN {
            return Err(Exception::throw_message(&ctx, "WebSocket is not open"));
        }
        let item = if let Some(text) = value.as_string() {
            Outgoing::Text(text.to_string()?)
        } else {
            // Buffer, TypedArray, DataView and ArrayBuffer all decode through
            // the same helper the HTTP client uses for request bodies.
            Outgoing::Binary(crate::client_request::chunk_to_bytes(&ctx, value)?)
        };
        let sender = this.borrow().outgoing.clone();
        if let Some(sender) = sender {
            let _ = sender.send(item);
        }
        Ok(())
    }

    pub fn close(this: This<Class<'js, Self>>, code: Opt<u16>, reason: Opt<String>) -> Result<()> {
        let sender = {
            let mut borrow = this.borrow_mut();
            if borrow.state >= CLOSING {
                return Ok(());
            }
            borrow.set_state(CLOSING);
            borrow.outgoing.clone()
        };
        if let Some(sender) = sender {
            let _ = sender.send(Outgoing::Close(
                code.0.unwrap_or(1000),
                reason.0.unwrap_or_default().into_bytes(),
            ));
        }
        Ok(())
    }
}

/// Best-effort human-readable text for a task error.
fn error_text(ctx: &Ctx<'_>, err: rquickjs::Error) -> String {
    match err {
        rquickjs::Error::Exception => {
            // The message lives on the thrown JS Error object.
            let value = ctx.catch();
            value
                .as_object()
                .and_then(|obj| obj.get::<_, String>("message").ok())
                .or_else(|| value.as_string().and_then(|s| s.to_string().ok()))
                .unwrap_or_else(|| "WebSocket failed".to_string())
        },
        other => other.to_string(),
    }
}

/// How a connection ended, decided inside the task and reported by `finish`.
enum Ended {
    PeerClosed {
        code: u16,
        reason: String,
        clean: bool,
    },
    LocallyClosed {
        code: u16,
        reason: String,
    },
    #[allow(clippy::enum_variant_names)]
    TransportEnded,
}

fn is_clean_disconnect(err: &fastwebsockets::WebSocketError) -> bool {
    matches!(
        err,
        fastwebsockets::WebSocketError::ConnectionClosed
            | fastwebsockets::WebSocketError::UnexpectedEOF
    )
}

fn header_contains(headers: &header::HeaderMap, name: header::HeaderName, value: &str) -> bool {
    headers.get_all(name).iter().any(|existing| {
        existing
            .to_str()
            .map(|text| {
                text.split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case(value))
            })
            .unwrap_or(false)
    })
}

/// `Sec-WebSocket-Accept` = base64(SHA-1(key + GUID)).
fn accept_for_key(key: &str) -> String {
    use base64::Engine;
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

/// A fresh 16-byte key, base64-encoded, as the handshake requires.
fn generate_key() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = ((nanos >> ((index % 16) * 8)) & 0xff) as u8 ^ (index as u8);
        }
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode a close frame payload into `(code, reason)`.
fn parse_close(payload: &[u8]) -> (u16, String) {
    if payload.len() < 2 {
        return (1005, String::new());
    }
    let code = u16::from_be_bytes([payload[0], payload[1]]);
    let reason = String::from_utf8_lossy(&payload[2..]).into_owned();
    (code, reason)
}
