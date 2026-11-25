// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! Native DNS resolver (`ResolverState`, UDP+TCP transport, `NativeResolver`).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::result::Result as StdResult;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rquickjs::function::Opt;
use rquickjs::{Array, ArrayBuffer, Ctx, Exception, Object, Result, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::Notify;

use crate::wire::{
    decode_response, encode_query, reverse_ip_to_arpa, DnsRecord, DnsWireError, TYPE_A, TYPE_AAAA,
    TYPE_ANY, TYPE_CAA, TYPE_CNAME, TYPE_MX, TYPE_NAPTR, TYPE_NS, TYPE_PTR, TYPE_SOA, TYPE_SRV,
    TYPE_TLSA, TYPE_TXT,
};

static NEXT_TX_ID: AtomicU16 = AtomicU16::new(1);

static DEFAULT_RESOLVER: LazyLock<Arc<ResolverState>> =
    LazyLock::new(|| Arc::new(ResolverState::new(5000, 4)));

pub fn default_resolver_state() -> Arc<ResolverState> {
    Arc::clone(&DEFAULT_RESOLVER)
}

/// Throw a JS `TypeError` (or `RangeError` if specified) with a Node error `code`.
pub(crate) fn throw_coded_error(
    ctx: &Ctx<'_>,
    ctor: &str,
    code: &str,
    message: &str,
) -> rquickjs::Error {
    let Ok(factory) = ctx.eval::<rquickjs::Function, _>(ctor) else {
        return Exception::throw_message(ctx, message);
    };
    let Ok(val): StdResult<Value<'_>, _> = factory.call((message,)) else {
        return Exception::throw_message(ctx, message);
    };
    if let Some(obj) = val.as_object() {
        let _ = obj.set("code", code);
    }
    match Exception::from_value(val) {
        Ok(exc) => exc.throw(),
        Err(err) => err,
    }
}

/// Create a Node-compatible DNS `Error` object with `code`, `syscall`, and optional `hostname`.
pub(crate) fn create_dns_error<'js>(
    ctx: &Ctx<'js>,
    code: &str,
    syscall: &str,
    hostname: Option<&str>,
) -> Result<Exception<'js>> {
    let message = match hostname {
        Some(h) => format!("{syscall} {code} {h}"),
        None => format!("{syscall} {code}"),
    };
    let exc = Exception::from_message(ctx.clone(), &message)?;
    let obj = exc.as_object();
    obj.set("code", code)?;
    obj.set("syscall", syscall)?;
    if let Some(h) = hostname {
        obj.set("hostname", h)?;
    }
    Ok(exc)
}

fn throw_dns_error(
    ctx: &Ctx<'_>,
    code: &str,
    syscall: &str,
    hostname: Option<&str>,
) -> rquickjs::Error {
    match create_dns_error(ctx, code, syscall, hostname) {
        Ok(exc) => exc.throw(),
        Err(err) => err,
    }
}

/// Discover OS nameservers (`/etc/resolv.conf` on Unix, safe fallback elsewhere).
pub fn system_nameservers() -> Vec<SocketAddr> {
    #[cfg(unix)]
    {
        if let Ok(contents) = std::fs::read_to_string("/etc/resolv.conf") {
            let mut servers = Vec::new();
            for line in contents.lines() {
                let line = line.split(['#', ';']).next().unwrap_or("").trim();
                let mut parts = line.split_whitespace();
                if parts.next() == Some("nameserver") {
                    if let Some(ip_str) = parts.next() {
                        let ip_clean = ip_str.split('%').next().unwrap_or(ip_str);
                        if let Ok(ip) = ip_clean.parse::<IpAddr>() {
                            servers.push(SocketAddr::new(ip, 53));
                        }
                    }
                }
            }
            if !servers.is_empty() {
                return servers;
            }
        }
    }
    vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 53)]
}

/// Format a `SocketAddr` the way Node's `dns.getServers()` does.
pub fn format_server(addr: SocketAddr) -> String {
    match addr {
        SocketAddr::V4(v4) => {
            if v4.port() == 53 {
                v4.ip().to_string()
            } else {
                format!("{}:{}", v4.ip(), v4.port())
            }
        },
        SocketAddr::V6(v6) => {
            if v6.port() == 53 {
                v6.ip().to_string()
            } else {
                format!("[{}]:{}", v6.ip(), v6.port())
            }
        },
    }
}

/// Parse a server string matching Node's `setServers` (`ip`, `ip:port`, `[ipv6]:port`).
pub fn parse_server(s: &str) -> StdResult<SocketAddr, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("IP address is empty".to_string());
    }

    // Try plain IP address first (covers both IPv4 without port and bare IPv6 like "::1")
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, 53));
    }

    // Bracketed IPv6 with optional :port (`[::1]` or `[::1]:5353`)
    if let Some(after_open) = trimmed.strip_prefix('[') {
        if let Some((ip_part, rest)) = after_open.split_once(']') {
            let ip: Ipv6Addr = ip_part
                .parse()
                .map_err(|_| format!("Invalid IP address: '{s}'"))?;
            if rest.is_empty() {
                return Ok(SocketAddr::new(IpAddr::V6(ip), 53));
            }
            if let Some(port_str) = rest.strip_prefix(':') {
                let port: u16 = port_str
                    .parse()
                    .ok()
                    .filter(|&p| p > 0)
                    .ok_or_else(|| format!("Invalid IP address: '{s}'"))?;
                return Ok(SocketAddr::new(IpAddr::V6(ip), port));
            }
        }
        return Err(format!("Invalid IP address: '{s}'"));
    }

    // IPv4:port
    if let Some((ip_part, port_str)) = trimmed.rsplit_once(':') {
        if let Ok(ip) = ip_part.parse::<Ipv4Addr>() {
            let port: u16 = port_str
                .parse()
                .ok()
                .filter(|&p| p > 0)
                .ok_or_else(|| format!("Invalid IP address: '{s}'"))?;
            return Ok(SocketAddr::new(IpAddr::V4(ip), port));
        }
    }

    Err(format!("Invalid IP address: '{s}'"))
}

pub struct ResolverState {
    servers: RwLock<Vec<SocketAddr>>,
    local_v4: RwLock<Ipv4Addr>,
    local_v6: RwLock<Ipv6Addr>,
    timeout_ms: u64,
    tries: usize,
    cancel_epoch: AtomicU64,
    cancel_notify: Notify,
}

impl ResolverState {
    pub fn new(timeout_ms: i64, tries: i64) -> Self {
        let resolved_timeout = if timeout_ms <= 0 {
            5000
        } else {
            timeout_ms as u64
        };
        let resolved_tries = if tries <= 0 { 4 } else { tries as usize };
        Self {
            servers: RwLock::new(system_nameservers()),
            local_v4: RwLock::new(Ipv4Addr::UNSPECIFIED),
            local_v6: RwLock::new(Ipv6Addr::UNSPECIFIED),
            timeout_ms: resolved_timeout,
            tries: resolved_tries,
            cancel_epoch: AtomicU64::new(0),
            cancel_notify: Notify::new(),
        }
    }

    pub fn get_servers(&self) -> Vec<String> {
        let guard = self.servers.read().unwrap_or_else(|e| e.into_inner());
        guard.iter().copied().map(format_server).collect()
    }

    pub fn set_servers(&self, raw: &[String]) -> StdResult<(), String> {
        let mut parsed = Vec::with_capacity(raw.len());
        for s in raw {
            parsed.push(parse_server(s)?);
        }
        let mut guard = self.servers.write().unwrap_or_else(|e| e.into_inner());
        *guard = parsed;
        Ok(())
    }

    pub fn set_local_address(&self, v4: Ipv4Addr, v6: Ipv6Addr) {
        *self.local_v4.write().unwrap_or_else(|e| e.into_inner()) = v4;
        *self.local_v6.write().unwrap_or_else(|e| e.into_inner()) = v6;
    }

    pub fn cancel(&self) {
        self.cancel_epoch.fetch_add(1, Ordering::AcqRel);
        self.cancel_notify.notify_waiters();
    }

    fn next_tx_id() -> u16 {
        let seq = NEXT_TX_ID.fetch_add(1, Ordering::Relaxed);
        let entropy = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u16)
            .unwrap_or(0);
        seq.wrapping_add(entropy)
    }

    pub async fn query(
        &self,
        hostname: &str,
        qtype: u16,
    ) -> StdResult<Vec<DnsRecord>, DnsWireError> {
        let servers = {
            let guard = self.servers.read().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        if servers.is_empty() {
            return Err(DnsWireError::ConnRefused);
        }
        let local_v4 = *self.local_v4.read().unwrap_or_else(|e| e.into_inner());
        let local_v6 = *self.local_v6.read().unwrap_or_else(|e| e.into_inner());
        let start_epoch = self.cancel_epoch.load(Ordering::Acquire);

        let mut last_err = DnsWireError::Timeout;
        for _attempt in 0..self.tries {
            for &server in &servers {
                if self.cancel_epoch.load(Ordering::Acquire) != start_epoch {
                    return Err(DnsWireError::Cancelled);
                }

                let tx_id = Self::next_tx_id();
                let packet = encode_query(tx_id, hostname, qtype)?;
                let bind_addr = match server {
                    SocketAddr::V4(_) => SocketAddr::V4(SocketAddrV4::new(local_v4, 0)),
                    SocketAddr::V6(_) => SocketAddr::V6(SocketAddrV6::new(local_v6, 0, 0, 0)),
                };

                match self
                    .query_server_once(server, bind_addr, &packet, tx_id, start_epoch)
                    .await
                {
                    Ok(records) => {
                        let filtered: Vec<DnsRecord> = if qtype == TYPE_ANY {
                            records
                        } else {
                            records.into_iter().filter(|r| r.rtype() == qtype).collect()
                        };
                        if filtered.is_empty() {
                            return Err(DnsWireError::NoData);
                        }
                        return Ok(filtered);
                    },
                    Err(DnsWireError::Cancelled) => {
                        return Err(DnsWireError::Cancelled);
                    },
                    Err(err) if !err.is_retryable() => {
                        return Err(err);
                    },
                    Err(err) => {
                        last_err = err;
                    },
                }
            }
        }

        Err(last_err)
    }

    async fn query_server_once(
        &self,
        server: SocketAddr,
        bind_addr: SocketAddr,
        packet: &[u8],
        tx_id: u16,
        start_epoch: u64,
    ) -> StdResult<Vec<DnsRecord>, DnsWireError> {
        let timeout_dur = Duration::from_millis(self.timeout_ms);
        let socket = UdpSocket::bind(bind_addr)
            .await
            .map_err(|_| DnsWireError::ConnRefused)?;
        socket
            .send_to(packet, server)
            .await
            .map_err(|_| DnsWireError::ConnRefused)?;

        let mut buf = vec![0u8; 4096];
        let deadline = tokio::time::Instant::now() + timeout_dur;

        let decoded = loop {
            if self.cancel_epoch.load(Ordering::Acquire) != start_epoch {
                return Err(DnsWireError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(DnsWireError::Timeout);
            }

            let recv_fut = tokio::time::timeout(remaining, socket.recv_from(&mut buf));
            tokio::select! {
                _ = self.cancel_notify.notified() => {
                    if self.cancel_epoch.load(Ordering::Acquire) != start_epoch {
                        return Err(DnsWireError::Cancelled);
                    }
                }
                res = recv_fut => {
                    let (n, _from) = match res {
                        Ok(Ok(pair)) => pair,
                        Ok(Err(e)) => {
                            if e.kind() == std::io::ErrorKind::ConnectionRefused {
                                return Err(DnsWireError::ConnRefused);
                            }
                            return Err(DnsWireError::ServFail);
                        },
                        Err(_) => return Err(DnsWireError::Timeout),
                    };
                    if n >= 2 && u16::from_be_bytes([buf[0], buf[1]]) != tx_id {
                        continue;
                    }
                    break decode_response(&buf[..n], tx_id)?;
                }
            }
        };

        if decoded.truncated {
            let remaining = deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .max(Duration::from_millis(250));
            return self
                .query_tcp(server, packet, tx_id, remaining, start_epoch)
                .await;
        }

        Ok(decoded.answers)
    }

    async fn query_tcp(
        &self,
        server: SocketAddr,
        packet: &[u8],
        tx_id: u16,
        timeout_dur: Duration,
        start_epoch: u64,
    ) -> StdResult<Vec<DnsRecord>, DnsWireError> {
        let tcp_fut = async {
            let mut stream = TcpStream::connect(server)
                .await
                .map_err(|_| DnsWireError::ConnRefused)?;
            let len_prefix = (packet.len() as u16).to_be_bytes();
            stream
                .write_all(&len_prefix)
                .await
                .map_err(|_| DnsWireError::ServFail)?;
            stream
                .write_all(packet)
                .await
                .map_err(|_| DnsWireError::ServFail)?;
            stream.flush().await.map_err(|_| DnsWireError::ServFail)?;

            let mut resp_len_buf = [0u8; 2];
            stream
                .read_exact(&mut resp_len_buf)
                .await
                .map_err(|_| DnsWireError::BadResp)?;
            let resp_len = u16::from_be_bytes(resp_len_buf) as usize;
            let mut resp = vec![0u8; resp_len];
            stream
                .read_exact(&mut resp)
                .await
                .map_err(|_| DnsWireError::BadResp)?;
            let decoded = decode_response(&resp, tx_id)?;
            if decoded.truncated {
                return Err(DnsWireError::BadResp);
            }
            Ok(decoded.answers)
        };

        tokio::pin!(tcp_fut);
        let timeout_sleep = tokio::time::sleep(timeout_dur);
        tokio::pin!(timeout_sleep);

        loop {
            tokio::select! {
                _ = self.cancel_notify.notified() => {
                    if self.cancel_epoch.load(Ordering::Acquire) != start_epoch {
                        return Err(DnsWireError::Cancelled);
                    }
                }
                _ = &mut timeout_sleep => {
                    return Err(DnsWireError::Timeout);
                }
                res = &mut tcp_fut => {
                    return res;
                }
            }
        }
    }
}

pub fn rrtype_from_str(rrtype: &str) -> Option<(u16, &'static str)> {
    match rrtype {
        "A" => Some((TYPE_A, "queryA")),
        "AAAA" => Some((TYPE_AAAA, "queryAaaa")),
        "ANY" => Some((TYPE_ANY, "queryAny")),
        "CAA" => Some((TYPE_CAA, "queryCaa")),
        "CNAME" => Some((TYPE_CNAME, "queryCname")),
        "MX" => Some((TYPE_MX, "queryMx")),
        "NAPTR" => Some((TYPE_NAPTR, "queryNaptr")),
        "NS" => Some((TYPE_NS, "queryNs")),
        "PTR" => Some((TYPE_PTR, "queryPtr")),
        "SOA" => Some((TYPE_SOA, "querySoa")),
        "SRV" => Some((TYPE_SRV, "querySrv")),
        "TLSA" => Some((TYPE_TLSA, "queryTlsa")),
        "TXT" => Some((TYPE_TXT, "queryTxt")),
        _ => None,
    }
}

/// Convert decoded DNS records to the exact JS shape expected by Node.js v24.
pub fn records_to_js<'js>(
    ctx: &Ctx<'js>,
    records: Vec<DnsRecord>,
    qtype: u16,
    ttl: bool,
) -> Result<Value<'js>> {
    if qtype == TYPE_SOA {
        for rec in records {
            if let DnsRecord::Soa {
                nsname,
                hostmaster,
                serial,
                refresh,
                retry,
                expire,
                minttl,
            } = rec
            {
                let obj = Object::new(ctx.clone())?;
                obj.set("nsname", nsname)?;
                obj.set("hostmaster", hostmaster)?;
                obj.set("serial", serial)?;
                obj.set("refresh", refresh)?;
                obj.set("retry", retry)?;
                obj.set("expire", expire)?;
                obj.set("minttl", minttl)?;
                return Ok(obj.into_value());
            }
        }
        return Err(throw_dns_error(ctx, "ENODATA", "querySoa", None));
    }

    let arr = Array::new(ctx.clone())?;
    let mut idx = 0usize;

    for rec in records {
        if qtype == TYPE_ANY {
            let val = any_record_to_js(ctx, rec)?;
            arr.set(idx, val)?;
            idx += 1;
            continue;
        }
        match rec {
            DnsRecord::A { address, ttl: rttl } => {
                if ttl {
                    let obj = Object::new(ctx.clone())?;
                    obj.set("address", address.to_string())?;
                    obj.set("ttl", rttl)?;
                    arr.set(idx, obj)?;
                } else {
                    arr.set(idx, address.to_string())?;
                }
                idx += 1;
            },
            DnsRecord::Aaaa { address, ttl: rttl } => {
                if ttl {
                    let obj = Object::new(ctx.clone())?;
                    obj.set("address", address.to_string())?;
                    obj.set("ttl", rttl)?;
                    arr.set(idx, obj)?;
                } else {
                    arr.set(idx, address.to_string())?;
                }
                idx += 1;
            },
            DnsRecord::Cname(v) | DnsRecord::Ns(v) | DnsRecord::Ptr(v) => {
                arr.set(idx, v)?;
                idx += 1;
            },
            DnsRecord::Mx { priority, exchange } => {
                let obj = Object::new(ctx.clone())?;
                obj.set("priority", priority)?;
                obj.set("exchange", exchange)?;
                arr.set(idx, obj)?;
                idx += 1;
            },
            DnsRecord::Txt(chunks) => {
                let inner = Array::new(ctx.clone())?;
                for (ci, s) in chunks.into_iter().enumerate() {
                    inner.set(ci, s)?;
                }
                arr.set(idx, inner)?;
                idx += 1;
            },
            DnsRecord::Srv {
                priority,
                weight,
                port,
                name,
            } => {
                let obj = Object::new(ctx.clone())?;
                obj.set("name", name)?;
                obj.set("port", port)?;
                obj.set("priority", priority)?;
                obj.set("weight", weight)?;
                arr.set(idx, obj)?;
                idx += 1;
            },
            DnsRecord::Naptr {
                order,
                preference,
                flags,
                service,
                regexp,
                replacement,
            } => {
                let obj = Object::new(ctx.clone())?;
                obj.set("flags", flags)?;
                obj.set("service", service)?;
                obj.set("regexp", regexp)?;
                obj.set("replacement", replacement)?;
                obj.set("order", order)?;
                obj.set("preference", preference)?;
                arr.set(idx, obj)?;
                idx += 1;
            },
            DnsRecord::Caa {
                critical,
                tag,
                value,
            } => {
                let obj = Object::new(ctx.clone())?;
                obj.set("critical", critical)?;
                obj.set(tag, value)?;
                arr.set(idx, obj)?;
                idx += 1;
            },
            DnsRecord::Tlsa {
                cert_usage,
                selector,
                match_type,
                data,
            } => {
                let obj = Object::new(ctx.clone())?;
                obj.set("certUsage", cert_usage)?;
                obj.set("selector", selector)?;
                obj.set("match", match_type)?;
                obj.set("data", ArrayBuffer::new(ctx.clone(), data)?.into_value())?;
                arr.set(idx, obj)?;
                idx += 1;
            },
            DnsRecord::Soa { .. } => {},
        }
    }

    Ok(arr.into_value())
}

fn any_record_to_js<'js>(ctx: &Ctx<'js>, rec: DnsRecord) -> Result<Value<'js>> {
    let obj = Object::new(ctx.clone())?;
    match rec {
        DnsRecord::A { address, ttl } => {
            obj.set("address", address.to_string())?;
            obj.set("ttl", ttl)?;
            obj.set("type", "A")?;
        },
        DnsRecord::Aaaa { address, ttl } => {
            obj.set("address", address.to_string())?;
            obj.set("ttl", ttl)?;
            obj.set("type", "AAAA")?;
        },
        DnsRecord::Cname(v) => {
            obj.set("value", v)?;
            obj.set("type", "CNAME")?;
        },
        DnsRecord::Ns(v) => {
            obj.set("value", v)?;
            obj.set("type", "NS")?;
        },
        DnsRecord::Ptr(v) => {
            obj.set("value", v)?;
            obj.set("type", "PTR")?;
        },
        DnsRecord::Mx { priority, exchange } => {
            obj.set("priority", priority)?;
            obj.set("exchange", exchange)?;
            obj.set("type", "MX")?;
        },
        DnsRecord::Txt(entries) => {
            let inner = Array::new(ctx.clone())?;
            for (i, s) in entries.into_iter().enumerate() {
                inner.set(i, s)?;
            }
            obj.set("entries", inner)?;
            obj.set("type", "TXT")?;
        },
        DnsRecord::Srv {
            priority,
            weight,
            port,
            name,
        } => {
            obj.set("name", name)?;
            obj.set("port", port)?;
            obj.set("priority", priority)?;
            obj.set("weight", weight)?;
            obj.set("type", "SRV")?;
        },
        DnsRecord::Soa {
            nsname,
            hostmaster,
            serial,
            refresh,
            retry,
            expire,
            minttl,
        } => {
            obj.set("nsname", nsname)?;
            obj.set("hostmaster", hostmaster)?;
            obj.set("serial", serial)?;
            obj.set("refresh", refresh)?;
            obj.set("retry", retry)?;
            obj.set("expire", expire)?;
            obj.set("minttl", minttl)?;
            obj.set("type", "SOA")?;
        },
        DnsRecord::Naptr {
            order,
            preference,
            flags,
            service,
            regexp,
            replacement,
        } => {
            obj.set("flags", flags)?;
            obj.set("service", service)?;
            obj.set("regexp", regexp)?;
            obj.set("replacement", replacement)?;
            obj.set("order", order)?;
            obj.set("preference", preference)?;
            obj.set("type", "NAPTR")?;
        },
        DnsRecord::Caa {
            critical,
            tag,
            value,
        } => {
            obj.set("critical", critical)?;
            obj.set(tag, value)?;
            obj.set("type", "CAA")?;
        },
        DnsRecord::Tlsa {
            cert_usage,
            selector,
            match_type,
            data,
        } => {
            obj.set("certUsage", cert_usage)?;
            obj.set("selector", selector)?;
            obj.set("match", match_type)?;
            obj.set("data", ArrayBuffer::new(ctx.clone(), data)?.into_value())?;
            obj.set("type", "TLSA")?;
        },
    }
    Ok(obj.into_value())
}

#[rquickjs::class]
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
pub struct NativeResolver {
    #[qjs(skip_trace)]
    state: Arc<ResolverState>,
}

impl NativeResolver {
    #[allow(dead_code)]
    pub fn from_state(state: Arc<ResolverState>) -> Self {
        Self { state }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl NativeResolver {
    #[qjs(constructor)]
    pub fn new(timeout_ms: Opt<i64>, tries: Opt<i64>) -> Self {
        Self {
            state: Arc::new(ResolverState::new(
                timeout_ms.0.unwrap_or(-1),
                tries.0.unwrap_or(4),
            )),
        }
    }

    pub fn get_servers(&self) -> Vec<String> {
        self.state.get_servers()
    }

    pub fn set_servers(&self, ctx: Ctx<'_>, servers: Value<'_>) -> Result<()> {
        let Some(arr) = servers.as_array() else {
            return Err(throw_coded_error(
                &ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "The \"servers\" argument must be an instance of Array",
            ));
        };
        let mut items = Vec::with_capacity(arr.len());
        for entry in arr.iter::<Value<'_>>() {
            let entry = entry?;
            let Some(s) = entry.as_string() else {
                return Err(throw_coded_error(
                    &ctx,
                    "TypeError",
                    "ERR_INVALID_ARG_TYPE",
                    "The \"servers\" items must be of type string",
                ));
            };
            items.push(s.to_string()?);
        }
        self.state
            .set_servers(&items)
            .map_err(|msg| throw_coded_error(&ctx, "TypeError", "ERR_INVALID_IP_ADDRESS", &msg))?;
        Ok(())
    }

    pub fn set_local_address(
        &self,
        ctx: Ctx<'_>,
        ipv4: Opt<Value<'_>>,
        ipv6: Opt<Value<'_>>,
    ) -> Result<()> {
        let v4 = match ipv4.0 {
            None => Ipv4Addr::UNSPECIFIED,
            Some(v) if v.is_null() || v.is_undefined() => Ipv4Addr::UNSPECIFIED,
            Some(v) => {
                let s = v
                    .as_string()
                    .ok_or_else(|| {
                        throw_coded_error(
                            &ctx,
                            "TypeError",
                            "ERR_INVALID_ARG_TYPE",
                            "The \"ipv4\" argument must be of type string",
                        )
                    })?
                    .to_string()?;
                s.parse::<Ipv4Addr>().map_err(|_| {
                    throw_coded_error(
                        &ctx,
                        "TypeError",
                        "ERR_INVALID_IP_ADDRESS",
                        &format!("Invalid IP address: '{s}'"),
                    )
                })?
            },
        };
        let v6 = match ipv6.0 {
            None => Ipv6Addr::UNSPECIFIED,
            Some(v) if v.is_null() || v.is_undefined() => Ipv6Addr::UNSPECIFIED,
            Some(v) => {
                let s = v
                    .as_string()
                    .ok_or_else(|| {
                        throw_coded_error(
                            &ctx,
                            "TypeError",
                            "ERR_INVALID_ARG_TYPE",
                            "The \"ipv6\" argument must be of type string",
                        )
                    })?
                    .to_string()?;
                s.parse::<Ipv6Addr>().map_err(|_| {
                    throw_coded_error(
                        &ctx,
                        "TypeError",
                        "ERR_INVALID_IP_ADDRESS",
                        &format!("Invalid IP address: '{s}'"),
                    )
                })?
            },
        };
        self.state.set_local_address(v4, v6);
        Ok(())
    }

    pub fn cancel(&self) {
        self.state.cancel();
    }

    pub fn validate_reverse_ip(&self, ctx: Ctx<'_>, ip: Value<'_>) -> Result<String> {
        let Some(ip_str) = ip.as_string() else {
            return Err(throw_coded_error(
                &ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "The \"ip\" argument must be of type string",
            ));
        };
        let ip_raw = ip_str.to_string()?;
        ip_raw
            .parse::<IpAddr>()
            .map_err(|_| throw_dns_error(&ctx, "EINVAL", "getHostByAddr", Some(&ip_raw)))?;
        Ok(ip_raw)
    }

    pub async fn query<'js>(
        &self,
        ctx: Ctx<'js>,
        hostname: Value<'js>,
        rrtype: String,
        ttl: Opt<bool>,
    ) -> Result<Value<'js>> {
        let Some(host_str) = hostname.as_string() else {
            return Err(throw_coded_error(
                &ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "The \"hostname\" argument must be of type string",
            ));
        };
        let host = host_str.to_string()?;
        let Some((qtype, syscall)) = rrtype_from_str(&rrtype) else {
            return Err(throw_coded_error(
                &ctx,
                "TypeError",
                "ERR_INVALID_ARG_VALUE",
                &format!("The argument 'rrtype' is invalid. Received '{rrtype}'"),
            ));
        };
        let state = Arc::clone(&self.state);
        match state.query(&host, qtype).await {
            Ok(records) => records_to_js(&ctx, records, qtype, ttl.0.unwrap_or(false)),
            Err(err) => Err(throw_dns_error(&ctx, err.code(), syscall, Some(&host))),
        }
    }

    pub async fn reverse<'js>(&self, ctx: Ctx<'js>, ip: Value<'js>) -> Result<Value<'js>> {
        let Some(ip_str) = ip.as_string() else {
            return Err(throw_coded_error(
                &ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "The \"ip\" argument must be of type string",
            ));
        };
        let ip_raw = ip_str.to_string()?;
        let parsed: IpAddr = ip_raw
            .parse()
            .map_err(|_| throw_dns_error(&ctx, "EINVAL", "getHostByAddr", Some(&ip_raw)))?;
        let arpa = reverse_ip_to_arpa(parsed);
        let state = Arc::clone(&self.state);
        match state.query(&arpa, TYPE_PTR).await {
            Ok(records) => records_to_js(&ctx, records, TYPE_PTR, false),
            Err(err) => Err(throw_dns_error(
                &ctx,
                err.code(),
                "getHostByAddr",
                Some(&ip_raw),
            )),
        }
    }
}
