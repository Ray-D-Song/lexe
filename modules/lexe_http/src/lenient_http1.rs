// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    cell::Cell,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use hyper::StatusCode;
use lexe_async_hooks::begin_http_incoming_message_scope;
use lexe_buffer::Buffer;
use lexe_context::CtxExtension;
use lexe_events::Emitter;
use lexe_net::Socket;
use lexe_timers::resume_timer_loop_if_idle;
use lexe_utils::{bytes::ObjectBytes, diagnostics, event_loop};
use rquickjs::{Class, Ctx, IntoJs, Object, Undefined, Value};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::{mpsc, oneshot},
    time::{interval, MissedTickBehavior},
};

use crate::{
    headers::is_valid_header_token,
    io::WriteCmd,
    server::{
        attach_upgrade_writer, await_oneshot_pumping, build_incoming_headers_js, socket_class,
        HttpJsSerialGuard, IncomingMessage, JsEventLoopPump, RawResponseHead, ResponseFrame,
        ResponseSink, Server, ServerResponse, HTTP_DISPATCH_NEST,
    },
};

pub(crate) struct LenientConnConfig {
    pub(crate) max_headers_count: usize,
    pub(crate) max_header_size: usize,
    pub(crate) max_request_body_size: usize,
    pub(crate) headers_timeout: u64,
    pub(crate) request_timeout: u64,
    pub(crate) keep_alive_timeout: u64,
}

pub(crate) struct LenientClientError {
    pub(crate) message: String,
    pub(crate) code: &'static str,
}

struct ParsedHead {
    method: String,
    target: String,
    minor_version: u8,
    headers: Vec<(String, String)>,
}

fn latin1_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

fn trim_ows(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(bytes.len());
    let rest = &bytes[start..];
    let end = rest
        .iter()
        .rposition(|&b| b != b' ' && b != b'\t')
        .map(|i| i + 1)
        .unwrap_or(0);
    &rest[..end]
}

fn skip_leading_empty_lines(buf: &mut Vec<u8>) {
    let mut i = 0;
    while i < buf.len() {
        if buf[i] == b'\r' && buf.get(i + 1) == Some(&b'\n') {
            i += 2;
        } else if buf[i] == b'\n' {
            i += 1;
        } else {
            break;
        }
    }
    if i > 0 {
        buf.drain(..i);
    }
}

fn find_lenient_header_end(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < buf.len() {
        if buf[i] == b'\r' && buf.get(i + 1) == Some(&b'\n') {
            if buf.get(i + 2) == Some(&b'\r') && buf.get(i + 3) == Some(&b'\n') {
                return Some((i, i + 4));
            }
            if buf.get(i + 2) == Some(&b'\n') {
                return Some((i, i + 3));
            }
        } else if buf[i] == b'\n' {
            if buf.get(i + 1) == Some(&b'\n') {
                return Some((i, i + 2));
            }
            if buf.get(i + 1) == Some(&b'\r') && buf.get(i + 2) == Some(&b'\n') {
                return Some((i, i + 3));
            }
        }
        i += 1;
    }
    None
}

enum ReadHeadOutcome {
    Head(Vec<u8>),
    TooLarge,
    Eof,
    Timeout,
}

async fn read_http1_head<R: AsyncRead + Unpin>(
    read_half: &mut R,
    buf: &mut Vec<u8>,
    max_header_size: usize,
    headers_timeout: u64,
) -> std::result::Result<ReadHeadOutcome, LenientClientError> {
    let deadline = (headers_timeout > 0)
        .then(|| tokio::time::Instant::now() + Duration::from_millis(headers_timeout));
    let mut tmp = [0u8; 4096];

    loop {
        skip_leading_empty_lines(buf);
        if let Some((head_end, body_start)) = find_lenient_header_end(buf) {
            if head_end > max_header_size {
                return Ok(ReadHeadOutcome::TooLarge);
            }
            let head = buf[..head_end].to_vec();
            buf.drain(..body_start);
            return Ok(ReadHeadOutcome::Head(head));
        }
        if buf.len() > max_header_size {
            return Ok(ReadHeadOutcome::TooLarge);
        }

        let read_res = if let Some(dl) = deadline {
            match tokio::time::timeout_at(dl, read_half.read(&mut tmp)).await {
                Ok(res) => res,
                Err(_) => return Ok(ReadHeadOutcome::Timeout),
            }
        } else {
            read_half.read(&mut tmp).await
        };

        match read_res {
            Ok(0) => {
                skip_leading_empty_lines(buf);
                if buf.is_empty() {
                    return Ok(ReadHeadOutcome::Eof);
                }
                return Err(LenientClientError {
                    message: "Parse Error: EOF".to_string(),
                    code: "HPE_INVALID_EOF_STATE",
                });
            },
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(err) => {
                return Err(LenientClientError {
                    message: err.to_string(),
                    code: "HPE_INVALID_EOF_STATE",
                });
            },
        }
    }
}

enum ParseHeadResult {
    Parsed(ParsedHead),
    TooManyHeaders,
}

fn parse_lenient_head(
    head: &[u8],
    max_headers_count: usize,
) -> std::result::Result<ParseHeadResult, LenientClientError> {
    let mut lines = head.split(|&b| b == b'\n').map(|line| {
        if let Some(stripped) = line.strip_suffix(b"\r") {
            stripped
        } else {
            line
        }
    });

    let Some(req_line) = lines.next() else {
        return Err(LenientClientError {
            message: "Invalid HTTP request line".to_string(),
            code: "HPE_INVALID_REQUEST",
        });
    };

    let req_trimmed = trim_ows(req_line);
    let Some(first_sp) = req_trimmed.iter().position(|&b| b == b' ') else {
        return Err(LenientClientError {
            message: "Invalid HTTP request line".to_string(),
            code: "HPE_INVALID_REQUEST",
        });
    };
    let method_bytes = &req_trimmed[..first_sp];
    let Ok(method_str) = std::str::from_utf8(method_bytes) else {
        return Err(LenientClientError {
            message: "Invalid HTTP method".to_string(),
            code: "HPE_INVALID_METHOD",
        });
    };
    if !is_valid_header_token(method_str) {
        return Err(LenientClientError {
            message: "Invalid HTTP method".to_string(),
            code: "HPE_INVALID_METHOD",
        });
    }

    let after_method = trim_ows(&req_trimmed[first_sp..]);
    let Some(last_sp) = after_method.iter().rposition(|&b| b == b' ') else {
        return Err(LenientClientError {
            message: "Missing HTTP version".to_string(),
            code: "HPE_INVALID_VERSION",
        });
    };
    let target_bytes = trim_ows(&after_method[..last_sp]);
    let version_bytes = trim_ows(&after_method[last_sp + 1..]);
    if target_bytes.is_empty()
        || target_bytes
            .iter()
            .any(|&b| b == 0 || b == b'\r' || b == b'\n')
    {
        return Err(LenientClientError {
            message: "Invalid HTTP request target".to_string(),
            code: "HPE_INVALID_URL",
        });
    }
    let version_str = std::str::from_utf8(version_bytes).unwrap_or("");
    let minor_version = match version_str {
        "HTTP/1.0" => 0,
        "HTTP/1.1" => 1,
        v if v.starts_with("HTTP/1.")
            && v.len() > 7
            && v[7..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            1
        },
        _ => {
            return Err(LenientClientError {
                message: "Invalid HTTP version".to_string(),
                code: "HPE_INVALID_VERSION",
            });
        },
    };

    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line[0] == b' ' || line[0] == b'\t' {
            let Some((_, last_val)) = headers.last_mut() else {
                return Err(LenientClientError {
                    message: "Unexpected folded header line".to_string(),
                    code: "HPE_INVALID_HEADER_TOKEN",
                });
            };
            let cont = latin1_to_string(trim_ows(line));
            if !cont.is_empty() {
                if !last_val.is_empty() {
                    last_val.push(' ');
                }
                last_val.push_str(&cont);
            }
            continue;
        }
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            return Err(LenientClientError {
                message: "Missing colon in header".to_string(),
                code: "HPE_INVALID_HEADER_TOKEN",
            });
        };
        let name_bytes = &line[..colon];
        let name_str = std::str::from_utf8(name_bytes).unwrap_or("");
        if !is_valid_header_token(name_str) {
            return Err(LenientClientError {
                message: "Invalid header name".to_string(),
                code: "HPE_INVALID_HEADER_TOKEN",
            });
        }
        if max_headers_count > 0 && headers.len() >= max_headers_count {
            return Ok(ParseHeadResult::TooManyHeaders);
        }
        let val = latin1_to_string(trim_ows(&line[colon + 1..]));
        headers.push((name_str.to_string(), val));
    }

    Ok(ParseHeadResult::Parsed(ParsedHead {
        method: method_str.to_string(),
        target: latin1_to_string(target_bytes),
        minor_version,
        headers,
    }))
}

enum LineOutcome {
    Line(Vec<u8>),
    Timeout,
    Eof,
    EarlyResponse(RawResponseHead),
}

async fn read_line_lenient<'js, R: AsyncRead + Unpin>(
    ctx: &Ctx<'js>,
    read_half: &mut R,
    buf: &mut Vec<u8>,
    deadline: Option<tokio::time::Instant>,
    rx: &mut oneshot::Receiver<RawResponseHead>,
    js_pump: &mut JsEventLoopPump,
) -> LineOutcome {
    let mut tmp = [0u8; 1024];
    let mut pump_tick = interval(Duration::from_millis(1));
    pump_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = buf.drain(..pos + 1).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return LineOutcome::Line(line);
        }
        let res = if let Some(dl) = deadline {
            tokio::select! {
                res_head = &mut *rx => match res_head {
                    Ok(head) => return LineOutcome::EarlyResponse(head),
                    Err(_) => return LineOutcome::Eof,
                },
                res = tokio::time::timeout_at(dl, read_half.read(&mut tmp)) => match res {
                    Ok(r) => r,
                    Err(_) => return LineOutcome::Timeout,
                },
                _ = pump_tick.tick() => {
                    js_pump.tick(ctx);
                    continue;
                }
            }
        } else {
            tokio::select! {
                res_head = &mut *rx => match res_head {
                    Ok(head) => return LineOutcome::EarlyResponse(head),
                    Err(_) => return LineOutcome::Eof,
                },
                res = read_half.read(&mut tmp) => res,
                _ = pump_tick.tick() => {
                    js_pump.tick(ctx);
                    continue;
                }
            }
        };
        match res {
            Ok(0) | Err(_) => return LineOutcome::Eof,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
}

struct BodyIo<'a, 'js, R> {
    ctx: &'a Ctx<'js>,
    req: &'a Class<'js, IncomingMessage<'js>>,
    read_half: &'a mut R,
    buf: &'a mut Vec<u8>,
    deadline: Option<tokio::time::Instant>,
    rx: &'a mut oneshot::Receiver<RawResponseHead>,
    js_pump: &'a mut JsEventLoopPump,
}

async fn read_exact_or_chunk_into_req<'js, R: AsyncRead + Unpin>(
    io: &mut BodyIo<'_, 'js, R>,
    len: usize,
    max_body_size: usize,
) -> std::result::Result<(), BodyAbort> {
    let mut remaining = len;
    let mut tmp = [0u8; 8192];
    let mut pump_tick = interval(Duration::from_millis(1));
    pump_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    while remaining > 0 {
        while io.req.borrow().is_paused() && !io.req.borrow().is_destroyed() {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(1)) => io.js_pump.tick(io.ctx),
                _ = pump_tick.tick() => io.js_pump.tick(io.ctx),
            }
        }
        if io.req.borrow().is_destroyed() {
            return Err(BodyAbort::Destroyed);
        }
        if !io.buf.is_empty() {
            let take = remaining.min(io.buf.len());
            let data: Vec<u8> = io.buf.drain(..take).collect();
            if io.req.borrow().body_len().saturating_add(data.len()) > max_body_size {
                return Err(BodyAbort::TooLarge);
            }
            IncomingMessage::push_body_chunk(io.req, io.ctx, &data);
            remaining -= take;
            continue;
        }

        let read_fut = io.read_half.read(&mut tmp);
        let n = if let Some(dl) = io.deadline {
            tokio::select! {
                res_head = &mut *io.rx => match res_head {
                    Ok(head) => return Err(BodyAbort::EarlyResponse(head)),
                    Err(_) => return Err(BodyAbort::Destroyed),
                },
                res = tokio::time::timeout_at(dl, read_fut) => match res {
                    Ok(Ok(0)) | Ok(Err(_)) => return Err(BodyAbort::Eof),
                    Ok(Ok(n)) => n,
                    Err(_) => return Err(BodyAbort::Timeout),
                },
                _ = pump_tick.tick() => {
                    io.js_pump.tick(io.ctx);
                    continue;
                }
            }
        } else {
            tokio::select! {
                res_head = &mut *io.rx => match res_head {
                    Ok(head) => return Err(BodyAbort::EarlyResponse(head)),
                    Err(_) => return Err(BodyAbort::Destroyed),
                },
                res = read_fut => match res {
                    Ok(0) | Err(_) => return Err(BodyAbort::Eof),
                    Ok(n) => n,
                },
                _ = pump_tick.tick() => {
                    io.js_pump.tick(io.ctx);
                    continue;
                }
            }
        };
        io.buf.extend_from_slice(&tmp[..n]);
    }
    Ok(())
}

enum BodyAbort {
    Destroyed,
    Eof,
    TooLarge,
    Timeout,
    EarlyResponse(RawResponseHead),
}

async fn consume_lenient_body<'js, R: AsyncRead + Unpin>(
    io: &mut BodyIo<'_, 'js, R>,
    is_chunked: bool,
    content_length: Option<usize>,
    config: &LenientConnConfig,
) -> std::result::Result<(), BodyAbort> {
    if is_chunked {
        loop {
            let line = match read_line_lenient(
                io.ctx,
                io.read_half,
                io.buf,
                io.deadline,
                io.rx,
                io.js_pump,
            )
            .await
            {
                LineOutcome::Line(l) => l,
                LineOutcome::Timeout => return Err(BodyAbort::Timeout),
                LineOutcome::Eof => return Err(BodyAbort::Eof),
                LineOutcome::EarlyResponse(head) => return Err(BodyAbort::EarlyResponse(head)),
            };
            let size_bytes = line
                .split(|&b| b == b';')
                .next()
                .map(trim_ows)
                .unwrap_or_default();
            let size_str = std::str::from_utf8(size_bytes).unwrap_or("");
            let Ok(chunk_size) = usize::from_str_radix(size_str, 16) else {
                return Err(BodyAbort::Eof);
            };
            if chunk_size == 0 {
                // Consume trailer lines until blank line
                loop {
                    let tline = match read_line_lenient(
                        io.ctx,
                        io.read_half,
                        io.buf,
                        io.deadline,
                        io.rx,
                        io.js_pump,
                    )
                    .await
                    {
                        LineOutcome::Line(l) => l,
                        LineOutcome::Timeout => return Err(BodyAbort::Timeout),
                        LineOutcome::Eof => break,
                        LineOutcome::EarlyResponse(head) => {
                            return Err(BodyAbort::EarlyResponse(head))
                        },
                    };
                    if tline.is_empty() {
                        break;
                    }
                    if let Some(colon) = tline.iter().position(|&b| b == b':') {
                        let name = latin1_to_string(trim_ows(&tline[..colon])).to_ascii_lowercase();
                        let val = latin1_to_string(trim_ows(&tline[colon + 1..]));
                        io.req.borrow_mut().push_trailer(&name, val);
                    }
                }
                break;
            }
            read_exact_or_chunk_into_req(io, chunk_size, config.max_request_body_size).await?;
            // Consume CRLF or LF terminating this chunk
            match read_line_lenient(io.ctx, io.read_half, io.buf, io.deadline, io.rx, io.js_pump)
                .await
            {
                LineOutcome::EarlyResponse(head) => return Err(BodyAbort::EarlyResponse(head)),
                LineOutcome::Timeout => return Err(BodyAbort::Timeout),
                _ => {},
            }
        }
        return Ok(());
    }

    if let Some(len) = content_length {
        if len > config.max_request_body_size {
            return Err(BodyAbort::TooLarge);
        }
        if len > 0 {
            read_exact_or_chunk_into_req(io, len, config.max_request_body_size).await?;
        }
    }
    Ok(())
}

async fn write_simple_response(
    write_tx: &mpsc::UnboundedSender<WriteCmd>,
    status: StatusCode,
    close: bool,
) {
    let reason = status.canonical_reason().unwrap_or("");
    let conn = if close { "close" } else { "keep-alive" };
    let msg = format!(
        "HTTP/1.1 {} {reason}\r\nConnection: {conn}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        status.as_u16()
    );
    let _ = write_tx.send(WriteCmd::Bytes(msg.into_bytes()));
    let (tx, rx) = oneshot::channel();
    let _ = write_tx.send(WriteCmd::Flush(tx));
    let _ = rx.await;
}

async fn send_raw_response<'js>(
    ctx: &Ctx<'js>,
    js_pump: &mut JsEventLoopPump,
    write_tx: &mpsc::UnboundedSender<WriteCmd>,
    mut head: RawResponseHead,
    force_close: bool,
    keep_alive_line: Option<String>,
) {
    let reason = head.status_message.clone().unwrap_or_else(|| {
        StatusCode::from_u16(head.status_code)
            .ok()
            .and_then(|s| s.canonical_reason().map(str::to_string))
            .unwrap_or_default()
    });
    let mut wire = String::new();
    wire.push_str(&format!("HTTP/1.1 {} {reason}\r\n", head.status_code));

    let bodyless = head.is_head
        || matches!(head.status_code, 204 | 304)
        || (100..200).contains(&head.status_code);
    let has_content_length = head.headers.has("content-length");
    let has_transfer_encoding = head.headers.has("transfer-encoding");
    let use_chunked = !bodyless && !has_content_length && !has_transfer_encoding;

    for (name, value) in head.headers.iter_raw_pairs() {
        if force_close && name.eq_ignore_ascii_case("connection") {
            continue;
        }
        wire.push_str(name);
        wire.push_str(": ");
        wire.push_str(value);
        wire.push_str("\r\n");
    }
    if force_close {
        wire.push_str("Connection: close\r\n");
    } else if !head.headers.has("connection") {
        wire.push_str("Connection: keep-alive\r\n");
    }
    if let Some(ka) = keep_alive_line {
        if !head.headers.has("keep-alive") {
            wire.push_str("Keep-Alive: ");
            wire.push_str(&ka);
            wire.push_str("\r\n");
        }
    }
    if use_chunked {
        wire.push_str("Transfer-Encoding: chunked\r\n");
    }
    wire.push_str("\r\n");

    let _ = write_tx.send(WriteCmd::Bytes(wire.into_bytes()));

    let mut trailers = None;
    let mut tick = interval(Duration::from_millis(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        let frame = tokio::select! {
            biased;
            f = head.body_rx.recv() => f,
            _ = tick.tick() => {
                js_pump.tick(ctx);
                continue;
            }
        };
        let Some(frame) = frame else {
            break;
        };
        match frame {
            ResponseFrame::Data(bytes) => {
                if bodyless || bytes.is_empty() {
                    continue;
                }
                if use_chunked || has_transfer_encoding {
                    let mut out = format!("{:x}\r\n", bytes.len()).into_bytes();
                    out.extend_from_slice(&bytes);
                    out.extend_from_slice(b"\r\n");
                    let _ = write_tx.send(WriteCmd::Bytes(out));
                } else {
                    let _ = write_tx.send(WriteCmd::Bytes(bytes.to_vec()));
                }
            },
            ResponseFrame::Trailers(map) => {
                trailers = Some(map);
            },
        }
    }

    if (use_chunked || has_transfer_encoding) && !bodyless {
        let mut term = b"0\r\n".to_vec();
        if let Some(map) = trailers {
            for (name, value) in &map {
                term.extend_from_slice(name.as_str().as_bytes());
                term.extend_from_slice(b": ");
                term.extend_from_slice(value.as_bytes());
                term.extend_from_slice(b"\r\n");
            }
        }
        term.extend_from_slice(b"\r\n");
        let _ = write_tx.send(WriteCmd::Bytes(term));
    }

    let (tx, rx) = oneshot::channel();
    let _ = write_tx.send(WriteCmd::Flush(tx));
    let _ = rx.await;
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn serve_lenient_connection<'js, R>(
    ctx: Ctx<'js>,
    server: Class<'js, Server<'js>>,
    connection_socket: Value<'js>,
    mut read_half: R,
    write_tx: mpsc::UnboundedSender<WriteCmd>,
    config: LenientConnConfig,
    active_requests: Arc<AtomicUsize>,
    activity_tx: mpsc::UnboundedSender<()>,
    requests_on_socket: Arc<AtomicUsize>,
    http_rt_key: usize,
) -> std::result::Result<(), LenientClientError>
where
    R: AsyncRead + Unpin,
{
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let max_header_size = config.max_header_size.max(8192);
    let mut upgraded = false;

    loop {
        let head_bytes = match read_http1_head(
            &mut read_half,
            &mut buf,
            max_header_size,
            config.headers_timeout,
        )
        .await?
        {
            ReadHeadOutcome::Head(h) => h,
            ReadHeadOutcome::TooLarge => {
                write_simple_response(&write_tx, StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE, true)
                    .await;
                break;
            },
            ReadHeadOutcome::Eof | ReadHeadOutcome::Timeout => break,
        };

        let parsed = match parse_lenient_head(&head_bytes, config.max_headers_count)? {
            ParseHeadResult::Parsed(p) => p,
            ParseHeadResult::TooManyHeaders => {
                write_simple_response(&write_tx, StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE, true)
                    .await;
                break;
            },
        };

        let outcome = HTTP_DISPATCH_NEST
            .scope(Cell::new(0), async {
                let _js_serial = HttpJsSerialGuard::acquire(http_rt_key).await;
                active_requests.fetch_add(1, Ordering::Relaxed);
                event_loop::adjust_http_inflight(http_rt_key, 1);
                let close_after = dispatch_lenient_request(
                    &ctx,
                    &server,
                    &connection_socket,
                    &mut read_half,
                    &mut buf,
                    &write_tx,
                    &config,
                    &requests_on_socket,
                    parsed,
                )
                .await;
                event_loop::adjust_http_inflight(http_rt_key, -1);
                active_requests.fetch_sub(1, Ordering::Relaxed);
                let _ = activity_tx.send(());
                close_after
            })
            .await;

        match outcome {
            DispatchControl::Continue => continue,
            DispatchControl::Close => break,
            DispatchControl::Upgraded => {
                upgraded = true;
                break;
            },
        }
    }

    if !upgraded {
        let (tx, rx) = oneshot::channel();
        let _ = write_tx.send(WriteCmd::Shutdown(tx));
        let _ = rx.await;
    }

    Ok(())
}

enum DispatchControl {
    Continue,
    Close,
    Upgraded,
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_lenient_request<'js, R: AsyncRead + Unpin>(
    ctx: &Ctx<'js>,
    server: &Class<'js, Server<'js>>,
    connection_socket: &Value<'js>,
    read_half: &mut R,
    buf: &mut Vec<u8>,
    write_tx: &mpsc::UnboundedSender<WriteCmd>,
    config: &LenientConnConfig,
    requests_on_socket: &Arc<AtomicUsize>,
    parsed: ParsedHead,
) -> DispatchControl {
    let req_num = requests_on_socket.fetch_add(1, Ordering::Relaxed) + 1;
    let max_req = server.borrow().max_requests_per_socket();

    let connection_hdr = parsed
        .headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("connection"))
        .map(|(_, v)| v.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(",");
    let has_close_hdr = connection_hdr.split(',').any(|t| t.trim() == "close");
    let has_keepalive_hdr = connection_hdr.split(',').any(|t| t.trim() == "keep-alive");
    let client_wants_close = if parsed.minor_version == 0 {
        !has_keepalive_hdr || has_close_hdr
    } else {
        has_close_hdr
    };

    let Ok((headers_obj, raw_headers)) = build_incoming_headers_js(ctx, parsed.headers.clone())
    else {
        return DispatchControl::Close;
    };
    let Ok(req) = Class::instance(
        ctx.clone(),
        match IncomingMessage::from_server_request(
            ctx,
            parsed.method.clone(),
            parsed.target.clone(),
            headers_obj,
            raw_headers,
            connection_socket.clone(),
        ) {
            Ok(m) => m,
            Err(_) => return DispatchControl::Close,
        },
    ) else {
        return DispatchControl::Close;
    };

    if max_req > 0 && req_num > max_req {
        let _ = Server::emit_str(
            server.clone(),
            ctx,
            "dropRequest",
            vec![req.into_value(), connection_socket.clone()],
            false,
        );
        write_simple_response(write_tx, StatusCode::SERVICE_UNAVAILABLE, true).await;
        return DispatchControl::Close;
    }

    let is_connect = parsed.method.eq_ignore_ascii_case("CONNECT");
    let upgrade_val = parsed
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("upgrade"))
        .map(|(_, v)| v.clone());
    let is_upgrade = upgrade_val.is_some();
    if is_connect || is_upgrade {
        let upgrade_event = if is_connect { "connect" } else { "upgrade" };
        if !server.borrow().has_listener_str(upgrade_event) {
            write_simple_response(write_tx, StatusCode::BAD_REQUEST, true).await;
            return DispatchControl::Close;
        }
        let Ok(socket) = socket_class(ctx, connection_socket) else {
            return DispatchControl::Close;
        };
        attach_upgrade_writer(&socket, write_tx.clone());
        let reader = Arc::new(std::sync::Mutex::new(Vec::new()));
        Socket::attach_raw_reader(&socket, reader.clone());
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let event_socket = socket.clone();
        let event_ctx = ctx.clone();
        ctx.spawn_exit_simple(async move {
            while let Some(bytes) = event_rx.recv().await {
                let value = Buffer(bytes).into_js(&event_ctx)?;
                Socket::emit_str(event_socket.clone(), &event_ctx, "data", vec![value], false)?;
            }
            Socket::emit_str(event_socket, &event_ctx, "end", vec![], false)?;
            Ok(())
        });

        let head_buf = ObjectBytes::Vec(std::mem::take(buf))
            .into_js(ctx)
            .unwrap_or_else(|_| Undefined.into_value(ctx.clone()));
        let head_resp = if is_connect {
            "HTTP/1.1 200 Connection Established\r\n\r\n".to_string()
        } else {
            format!(
                "HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: {}\r\n\r\n",
                upgrade_val.unwrap_or_default()
            )
        };
        let _ = write_tx.send(WriteCmd::Bytes(head_resp.into_bytes()));
        let _ = Server::emit_str(
            server.clone(),
            ctx,
            upgrade_event,
            vec![req.into_value(), connection_socket.clone(), head_buf],
            false,
        );
        let mut tmp = [0u8; 8192];
        loop {
            match read_half.read(&mut tmp).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let bytes = tmp[..n].to_vec();
                    reader.lock().unwrap().extend_from_slice(&bytes);
                    let _ = event_tx.send(bytes);
                },
            }
        }
        return DispatchControl::Upgraded;
    }

    let req_object = match req.clone().into_value().as_object() {
        Some(obj) => obj.clone(),
        None => return DispatchControl::Close,
    };
    let Ok(_http_scope) = begin_http_incoming_message_scope(ctx.clone(), req_object) else {
        return DispatchControl::Close;
    };

    let expect_val = parsed
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("expect"))
        .map(|(_, v)| v.trim().to_ascii_lowercase());
    let expects_continue = expect_val.as_deref() == Some("100-continue");
    let has_other_expect = expect_val.as_deref().is_some_and(|v| v != "100-continue");
    if has_other_expect && !server.borrow().has_listener_str("checkExpectation") {
        write_simple_response(write_tx, StatusCode::EXPECTATION_FAILED, true).await;
        return DispatchControl::Close;
    }

    let event_name = if expects_continue && server.borrow().has_listener_str("checkContinue") {
        "checkContinue"
    } else if has_other_expect {
        "checkExpectation"
    } else {
        "request"
    };

    let (tx, mut rx) = oneshot::channel();
    let req_value = req.clone().into_value();
    let track_finish = diagnostics::has_subscribers(ctx, "http.server.response.finish");
    let mut res_inner = ServerResponse::new_internal(
        ResponseSink::Raw(tx),
        Some(write_tx.clone()),
        parsed.method.eq_ignore_ascii_case("HEAD"),
        req_value.clone(),
        connection_socket.clone(),
        track_finish.then(|| server.clone().into_value()),
        track_finish,
    );
    res_inner.set_perf_start_ms(
        lexe_utils::perf::has_observers(
            lexe_utils::perf::rt_key_for_ctx(ctx),
            lexe_utils::perf::PerfEntryKind::Http,
        )
        .then(lexe_utils::time::monotonic_elapsed_ms),
    );
    let Ok(res) = Class::instance(ctx.clone(), res_inner) else {
        return DispatchControl::Close;
    };
    let res_value = res.clone().into_value();

    let _ = diagnostics::publish(ctx, "http.server.response.created", |ctx| {
        let msg = Object::new(ctx.clone())?;
        msg.set("request", req_value.clone())?;
        msg.set("response", res_value.clone())?;
        Ok(msg)
    });
    let _ = diagnostics::publish(ctx, "http.server.request.start", |ctx| {
        let msg = Object::new(ctx.clone())?;
        msg.set("request", req_value.clone())?;
        msg.set("response", res_value.clone())?;
        msg.set("socket", connection_socket.clone())?;
        msg.set("server", server.clone().into_value())?;
        Ok(msg)
    });

    if Server::emit_str(
        server.clone(),
        ctx,
        event_name,
        vec![req_value, res_value],
        false,
    )
    .is_err()
    {
        if ctx.has_exception() {
            let _ = ctx.catch();
        }
        if !await_flag_headers_sent(&res) {
            write_simple_response(write_tx, StatusCode::INTERNAL_SERVER_ERROR, true).await;
            return DispatchControl::Close;
        }
    }

    let mut js_pump = JsEventLoopPump::new(ctx);
    let _ = resume_timer_loop_if_idle(ctx);
    for _ in 0..512 {
        if res.borrow().has_continue_requested() || await_flag_headers_sent(&res) {
            break;
        }
        js_pump.tick(ctx);
    }

    let force_close = client_wants_close || (max_req > 0 && req_num == max_req);
    let keep_alive_line = if !force_close && max_req > 0 && req_num < max_req {
        Some(format!(
            "timeout={}, max={max_req}",
            config.keep_alive_timeout / 1000
        ))
    } else {
        None
    };

    if await_flag_headers_sent(&res) {
        if let Some(head) = await_oneshot_pumping(ctx, &mut js_pump, rx).await {
            send_raw_response(
                ctx,
                &mut js_pump,
                write_tx,
                head,
                force_close,
                keep_alive_line,
            )
            .await;
        }
        return if client_wants_close {
            DispatchControl::Close
        } else {
            DispatchControl::Continue
        };
    }

    let check_continue = event_name == "checkContinue";
    let consume_body = !check_continue || res.borrow().has_continue_requested();
    if consume_body {
        if expects_continue {
            let _ = write_tx.send(WriteCmd::Bytes(b"HTTP/1.1 100 Continue\r\n\r\n".to_vec()));
        }
        let is_chunked = parsed.headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("transfer-encoding")
                && v.to_ascii_lowercase().contains("chunked")
        });
        let content_length = parsed
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.trim().parse::<usize>().ok());
        let deadline = (config.request_timeout > 0)
            .then(|| tokio::time::Instant::now() + Duration::from_millis(config.request_timeout));
        let mut body_io = BodyIo {
            ctx,
            req: &req,
            read_half,
            buf,
            deadline,
            rx: &mut rx,
            js_pump: &mut js_pump,
        };
        match consume_lenient_body(&mut body_io, is_chunked, content_length, config).await {
            Ok(()) => {
                IncomingMessage::finish_body(&req, ctx);
            },
            Err(BodyAbort::TooLarge) => {
                write_simple_response(write_tx, StatusCode::PAYLOAD_TOO_LARGE, true).await;
                return DispatchControl::Close;
            },
            Err(BodyAbort::Timeout) => {
                write_simple_response(write_tx, StatusCode::REQUEST_TIMEOUT, true).await;
                return DispatchControl::Close;
            },
            Err(BodyAbort::EarlyResponse(head)) => {
                send_raw_response(
                    ctx,
                    &mut js_pump,
                    write_tx,
                    head,
                    force_close,
                    keep_alive_line,
                )
                .await;
                return if client_wants_close {
                    DispatchControl::Close
                } else {
                    DispatchControl::Continue
                };
            },
            Err(BodyAbort::Destroyed | BodyAbort::Eof) => {
                IncomingMessage::finish_body(&req, ctx);
            },
        }
    }

    if let Some(head) = await_oneshot_pumping(ctx, &mut js_pump, rx).await {
        send_raw_response(
            ctx,
            &mut js_pump,
            write_tx,
            head,
            force_close,
            keep_alive_line,
        )
        .await;
    }

    if client_wants_close {
        DispatchControl::Close
    } else {
        DispatchControl::Continue
    }
}

fn await_flag_headers_sent<'js>(res: &Class<'js, ServerResponse<'js>>) -> bool {
    res.borrow().is_headers_sent()
}
