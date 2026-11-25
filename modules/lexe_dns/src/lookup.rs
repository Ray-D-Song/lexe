// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::net::{IpAddr, SocketAddr};
use std::result::Result as StdResult;
use std::sync::atomic::{AtomicU8, Ordering};

use either::Either;
use lexe_context::CtxExtension;
use lexe_hooking::{invoke_async_hook, register_finalization_registry, HookType};
use lexe_utils::{provider::ProviderType, result::ResultExt};
use rquickjs::{
    prelude::Opt, qjs, Ctx, Error, Exception, FromJs, Function, IntoJs, Null, Object, Result, Value,
};

use crate::resolver::{create_dns_error, throw_coded_error};

const ERROR_MSG_OPTIONS_FAMILY: &str = "The argument 'family' must be one of: 0, 4, 6";
const ERROR_MSG_OPTIONS_ORDER: &str =
    "The argument 'order' must be one of: 'verbatim', 'ipv4first', 'ipv6first'";

#[cfg(unix)]
pub const DNS_ADDRCONFIG: i32 = libc::AI_ADDRCONFIG;
#[cfg(unix)]
pub const DNS_V4MAPPED: i32 = libc::AI_V4MAPPED;
#[cfg(unix)]
pub const DNS_ALL: i32 = libc::AI_ALL;

#[cfg(not(unix))]
pub const DNS_ADDRCONFIG: i32 = 0x0400;
#[cfg(not(unix))]
pub const DNS_V4MAPPED: i32 = 0x0800;
#[cfg(not(unix))]
pub const DNS_ALL: i32 = 0x0100;

/// Process-wide default address result order (`0 = verbatim`, `1 = ipv4first`, `2 = ipv6first`).
static DEFAULT_RESULT_ORDER: AtomicU8 = AtomicU8::new(0);

fn current_default_result_order() -> LookupOrder {
    match DEFAULT_RESULT_ORDER.load(Ordering::Acquire) {
        1 => LookupOrder::Ipv4First,
        2 => LookupOrder::Ipv6First,
        _ => LookupOrder::Verbatim,
    }
}

pub fn get_default_result_order() -> &'static str {
    match current_default_result_order() {
        LookupOrder::Verbatim => "verbatim",
        LookupOrder::Ipv4First => "ipv4first",
        LookupOrder::Ipv6First => "ipv6first",
    }
}

pub fn set_default_result_order(ctx: Ctx<'_>, order: Value<'_>) -> Result<()> {
    let Some(s) = order.as_string() else {
        return Err(throw_coded_error(
            &ctx,
            "TypeError",
            "ERR_INVALID_ARG_VALUE",
            "The argument 'order' must be one of: 'verbatim', 'ipv4first', 'ipv6first'",
        ));
    };
    let text = s.to_string()?;
    let code = match text.as_str() {
        "verbatim" => 0,
        "ipv4first" => 1,
        "ipv6first" => 2,
        _ => {
            return Err(throw_coded_error(
                &ctx,
                "TypeError",
                "ERR_INVALID_ARG_VALUE",
                ERROR_MSG_OPTIONS_ORDER,
            ));
        },
    };
    DEFAULT_RESULT_ORDER.store(code, Ordering::Release);
    Ok(())
}

fn create_lookup_error<'js>(
    ctx: &Ctx<'js>,
    message: &str,
    hostname: &str,
) -> Result<Exception<'js>> {
    let exc = Exception::from_message(ctx.clone(), message)?;
    let obj = exc.as_object();
    let _ = obj.set("code", "ENOTFOUND");
    let _ = obj.set("syscall", "getaddrinfo");
    let _ = obj.set("hostname", hostname);
    Ok(exc)
}

pub fn lookup<'js>(
    ctx: Ctx<'js>,
    hostname: String,
    options_or_callback: Either<Function<'js>, LookupOptions>,
    callback: Opt<Function<'js>>,
) -> Result<()> {
    let (cb, options) = match options_or_callback {
        Either::Left(cb) => (cb, LookupOptions::default()),
        Either::Right(options) => {
            let cb = callback
                .0
                .or_throw_msg(&ctx, "Callback parameter is missing")?;
            (cb, options)
        },
    };

    // SAFETY: Since it checks in advance whether it is a Function type, we can always get a pointer to the Function.
    let uid = unsafe { qjs::JS_VALUE_GET_PTR(cb.as_raw()) } as usize;
    register_finalization_registry(&ctx, cb.clone().into_value(), uid, None)?;
    invoke_async_hook(&ctx, HookType::Init, ProviderType::GetAddrInfoReqWrap, uid)?;

    ctx.clone().spawn_exit(async move {
        match lookup_records(&hostname, options).await {
            Ok(addrs) => {
                invoke_async_hook(&ctx, HookType::Before, ProviderType::None, uid)?;
                if options.all {
                    () = cb.call((Null.into_js(&ctx), addrs))?;
                } else {
                    let addr = addrs.into_iter().next();
                    if let Some(addr) = addr {
                        () = cb.call((Null.into_js(&ctx), addr.address, addr.family))?;
                    } else {
                        let exc = create_lookup_error(&ctx, "No address found", &hostname)?;
                        () = cb.call((exc,))?;
                    }
                }
                invoke_async_hook(&ctx, HookType::After, ProviderType::None, uid)?;
                Ok::<_, Error>(())
            },
            Err(err) => {
                invoke_async_hook(&ctx, HookType::Before, ProviderType::None, uid)?;
                let exc = create_lookup_error(&ctx, &err.to_string(), &hostname)?;
                () = cb.call((exc,))?;
                invoke_async_hook(&ctx, HookType::After, ProviderType::None, uid)?;
                Ok(())
            },
        }
    })?;
    Ok(())
}

/// Promise-based `dns.promises.lookup` / `dns/promises.lookup`.
pub async fn lookup_promise<'js>(
    ctx: Ctx<'js>,
    hostname: String,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let options = match options.0 {
        Some(value) => LookupOptions::from_js(&ctx, value)?,
        None => LookupOptions::default(),
    };

    match lookup_records(&hostname, options).await {
        Ok(addrs) if options.all => addrs.into_js(&ctx),
        Ok(addrs) => match addrs.into_iter().next() {
            Some(addr) => addr.into_js(&ctx),
            None => Err(create_lookup_error(&ctx, "No address found", &hostname)?.throw()),
        },
        Err(err) => Err(create_lookup_error(&ctx, &err.to_string(), &hostname)?.throw()),
    }
}

/// Shared DNS lookup core used by callback and Promise APIs.
pub async fn lookup_records(
    hostname: &str,
    options: LookupOptions,
) -> StdResult<Vec<LookupValue>, std::io::Error> {
    #[cfg(unix)]
    if options.hints != 0 {
        return lookup_with_hints_unix(hostname, options).await;
    }

    let mut addrs: Vec<LookupValue> = tokio::net::lookup_host((hostname, 0))
        .await?
        .filter_map(|addr| {
            if matches!(options.family, 4 | 0) {
                if let SocketAddr::V4(ipv4) = addr {
                    return Some(LookupValue {
                        address: ipv4.ip().to_string(),
                        family: 4,
                    });
                }
            }
            if matches!(options.family, 6 | 0) {
                if let SocketAddr::V6(ipv6) = addr {
                    return Some(LookupValue {
                        address: ipv6.ip().to_string(),
                        family: 6,
                    });
                }
            }
            None
        })
        .collect();
    sort_lookup_addrs(&mut addrs, options.order);
    Ok(addrs)
}

fn sort_lookup_addrs(addrs: &mut [LookupValue], order: LookupOrder) {
    match order {
        LookupOrder::Verbatim => {},
        LookupOrder::Ipv4First => {
            addrs.sort_by_key(|a| a.family);
        },
        LookupOrder::Ipv6First => {
            addrs.sort_by_key(|b| std::cmp::Reverse(b.family));
        },
    }
}

#[cfg(unix)]
async fn lookup_with_hints_unix(
    hostname: &str,
    options: LookupOptions,
) -> StdResult<Vec<LookupValue>, std::io::Error> {
    let host = hostname.to_string();
    let family = options.family;
    let hints_flags = options.hints;
    let order = options.order;

    tokio::task::spawn_blocking(move || -> StdResult<Vec<LookupValue>, std::io::Error> {
        let c_host = std::ffi::CString::new(host)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL byte"))?;
        let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
        hints.ai_flags = hints_flags;
        hints.ai_family = match family {
            4 => libc::AF_INET,
            6 => libc::AF_INET6,
            _ => libc::AF_UNSPEC,
        };
        hints.ai_socktype = libc::SOCK_STREAM;

        let mut res: *mut libc::addrinfo = std::ptr::null_mut();
        let rc = unsafe { libc::getaddrinfo(c_host.as_ptr(), std::ptr::null(), &hints, &mut res) };
        if rc != 0 {
            return Err(std::io::Error::other(
                "failed to lookup address information: Name or service not known",
            ));
        }

        let mut out = Vec::new();
        let mut cur = res;
        while !cur.is_null() {
            let info = unsafe { &*cur };
            if info.ai_family == libc::AF_INET && !info.ai_addr.is_null() && matches!(family, 4 | 0)
            {
                let sin = unsafe { &*(info.ai_addr as *const libc::sockaddr_in) };
                let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                let entry = LookupValue {
                    address: ip.to_string(),
                    family: 4,
                };
                if !out
                    .iter()
                    .any(|x: &LookupValue| x.address == entry.address && x.family == entry.family)
                {
                    out.push(entry);
                }
            } else if info.ai_family == libc::AF_INET6
                && !info.ai_addr.is_null()
                && matches!(family, 6 | 0)
            {
                let sin6 = unsafe { &*(info.ai_addr as *const libc::sockaddr_in6) };
                let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
                let entry = LookupValue {
                    address: ip.to_string(),
                    family: 6,
                };
                if !out
                    .iter()
                    .any(|x: &LookupValue| x.address == entry.address && x.family == entry.family)
                {
                    out.push(entry);
                }
            }
            cur = info.ai_next;
        }
        unsafe { libc::freeaddrinfo(res) };
        sort_lookup_addrs(&mut out, order);
        Ok(out)
    })
    .await
    .map_err(|_| std::io::Error::other("failed to lookup address information"))?
}

fn validate_lookup_service_args(
    ctx: &Ctx<'_>,
    address: Value<'_>,
    port: Value<'_>,
) -> Result<(IpAddr, u16)> {
    let Some(addr_str) = address.as_string() else {
        return Err(throw_coded_error(
            ctx,
            "TypeError",
            "ERR_INVALID_ARG_VALUE",
            "The \"address\" argument must be a valid IP address string",
        ));
    };
    let addr_text = addr_str.to_string()?;
    let ip = addr_text.parse::<IpAddr>().map_err(|_| {
        throw_coded_error(
            ctx,
            "TypeError",
            "ERR_INVALID_ARG_VALUE",
            &format!("The argument 'address' is invalid. Received '{addr_text}'"),
        )
    })?;

    let port_num = if let Some(n) = port.as_number() {
        if !n.is_finite() || n.fract() != 0.0 || !(0.0..=65535.0).contains(&n) {
            return Err(throw_coded_error(
                ctx,
                "RangeError",
                "ERR_SOCKET_BAD_PORT",
                &format!("Port should be >= 0 and < 65536. Received {n}."),
            ));
        }
        n as u16
    } else if let Some(s) = port.as_string() {
        let text = s.to_string()?;
        text.parse::<u16>().map_err(|_| {
            throw_coded_error(
                ctx,
                "RangeError",
                "ERR_SOCKET_BAD_PORT",
                &format!("Port should be >= 0 and < 65536. Received '{text}'."),
            )
        })?
    } else {
        return Err(throw_coded_error(
            ctx,
            "RangeError",
            "ERR_SOCKET_BAD_PORT",
            "Port should be >= 0 and < 65536.",
        ));
    };

    Ok((ip, port_num))
}

async fn resolve_service_os(ip: IpAddr, port: u16) -> StdResult<(String, String), &'static str> {
    tokio::task::spawn_blocking(move || -> StdResult<(String, String), &'static str> {
        #[cfg(unix)]
        {
            let mut host_buf = [0 as libc::c_char; 1025];
            let mut serv_buf = [0 as libc::c_char; 32];
            let rc = match ip {
                IpAddr::V4(v4) => {
                    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
                    sa.sin_family = libc::AF_INET as libc::sa_family_t;
                    sa.sin_port = port.to_be();
                    sa.sin_addr = libc::in_addr {
                        s_addr: u32::from_ne_bytes(v4.octets()),
                    };
                    unsafe {
                        libc::getnameinfo(
                            (&raw const sa) as *const libc::sockaddr,
                            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                            host_buf.as_mut_ptr(),
                            host_buf.len() as libc::socklen_t,
                            serv_buf.as_mut_ptr(),
                            serv_buf.len() as libc::socklen_t,
                            0,
                        )
                    }
                },
                IpAddr::V6(v6) => {
                    let mut sa6: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
                    sa6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                    sa6.sin6_port = port.to_be();
                    sa6.sin6_addr = libc::in6_addr {
                        s6_addr: v6.octets(),
                    };
                    unsafe {
                        libc::getnameinfo(
                            (&raw const sa6) as *const libc::sockaddr,
                            std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                            host_buf.as_mut_ptr(),
                            host_buf.len() as libc::socklen_t,
                            serv_buf.as_mut_ptr(),
                            serv_buf.len() as libc::socklen_t,
                            0,
                        )
                    }
                },
            };
            if rc != 0 {
                return Err("ENOTFOUND");
            }
            let host = unsafe { std::ffi::CStr::from_ptr(host_buf.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            let serv = unsafe { std::ffi::CStr::from_ptr(serv_buf.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            Ok((host, serv))
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Networking::WinSock::{
                getnameinfo, AF_INET, AF_INET6, IN6_ADDR, IN6_ADDR_0, IN_ADDR, IN_ADDR_0, SOCKADDR,
                SOCKADDR_IN, SOCKADDR_IN6,
            };
            let mut host_buf = [0u8; 1025];
            let mut serv_buf = [0u8; 32];
            let rc = match ip {
                IpAddr::V4(v4) => {
                    let mut sa: SOCKADDR_IN = unsafe { std::mem::zeroed() };
                    sa.sin_family = AF_INET;
                    sa.sin_port = port.to_be();
                    sa.sin_addr = IN_ADDR {
                        S_un: IN_ADDR_0 {
                            S_addr: u32::from_ne_bytes(v4.octets()),
                        },
                    };
                    unsafe {
                        getnameinfo(
                            (&raw const sa) as *const SOCKADDR,
                            std::mem::size_of::<SOCKADDR_IN>() as i32,
                            host_buf.as_mut_ptr(),
                            host_buf.len() as i32,
                            serv_buf.as_mut_ptr(),
                            serv_buf.len() as i32,
                            0,
                        )
                    }
                },
                IpAddr::V6(v6) => {
                    let mut sa6: SOCKADDR_IN6 = unsafe { std::mem::zeroed() };
                    sa6.sin6_family = AF_INET6;
                    sa6.sin6_port = port.to_be();
                    sa6.sin6_addr = IN6_ADDR {
                        u: IN6_ADDR_0 { Byte: v6.octets() },
                    };
                    unsafe {
                        getnameinfo(
                            (&raw const sa6) as *const SOCKADDR,
                            std::mem::size_of::<SOCKADDR_IN6>() as i32,
                            host_buf.as_mut_ptr(),
                            host_buf.len() as i32,
                            serv_buf.as_mut_ptr(),
                            serv_buf.len() as i32,
                            0,
                        )
                    }
                },
            };
            if rc != 0 {
                return Err("ENOTFOUND");
            }
            let host = unsafe { std::ffi::CStr::from_ptr(host_buf.as_ptr() as *const i8) }
                .to_string_lossy()
                .into_owned();
            let serv = unsafe { std::ffi::CStr::from_ptr(serv_buf.as_ptr() as *const i8) }
                .to_string_lossy()
                .into_owned();
            Ok((host, serv))
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok((ip.to_string(), port.to_string()))
        }
    })
    .await
    .map_err(|_| "ESERVFAIL")?
}

/// Callback-style `dns.lookupService(address, port, callback)`.
pub fn lookup_service_callback<'js>(
    ctx: Ctx<'js>,
    address: Value<'js>,
    port: Value<'js>,
    callback: Value<'js>,
) -> Result<()> {
    let Some(cb) = callback.as_function().cloned() else {
        return Err(throw_coded_error(
            &ctx,
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            "The \"callback\" argument must be of type function",
        ));
    };
    let (ip, port_num) = validate_lookup_service_args(&ctx, address, port)?;
    let ip_str = ip.to_string();

    ctx.clone().spawn_exit(async move {
        match resolve_service_os(ip, port_num).await {
            Ok((host, serv)) => {
                () = cb.call((Null.into_js(&ctx), host, serv))?;
            },
            Err(code) => {
                let exc = create_dns_error(&ctx, code, "getnameinfo", Some(&ip_str))?;
                () = cb.call((exc,))?;
            },
        }
        Ok::<_, Error>(())
    })?;
    Ok(())
}

/// Promise-style `dns.promises.lookupService(address, port)`.
pub async fn lookup_service_promise<'js>(
    ctx: Ctx<'js>,
    address: Value<'js>,
    port: Value<'js>,
) -> Result<Object<'js>> {
    let (ip, port_num) = validate_lookup_service_args(&ctx, address, port)?;
    let ip_str = ip.to_string();
    match resolve_service_os(ip, port_num).await {
        Ok((host, serv)) => {
            let obj = Object::new(ctx.clone())?;
            obj.set("hostname", host)?;
            obj.set("service", serv)?;
            Ok(obj)
        },
        Err(code) => Err(create_dns_error(&ctx, code, "getnameinfo", Some(&ip_str))?.throw()),
    }
}

pub struct LookupValue {
    pub address: String,
    pub family: i32,
}

impl<'js> IntoJs<'js> for LookupValue {
    fn into_js(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("address", self.address)?;
        object.set("family", self.family)?;
        Ok(object.into_value())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupOrder {
    Verbatim,
    Ipv4First,
    Ipv6First,
}

#[derive(Clone, Copy)]
pub struct LookupOptions {
    pub family: i32,
    pub all: bool,
    pub order: LookupOrder,
    pub hints: i32,
}

impl Default for LookupOptions {
    fn default() -> Self {
        Self {
            family: 0,
            all: false,
            order: current_default_result_order(),
            hints: 0,
        }
    }
}

impl<'js> FromJs<'js> for LookupOptions {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        let mut family = 0;
        let mut all = false;
        let mut order = current_default_result_order();
        let mut hints = 0;

        if let Some(v) = value.as_int() {
            if !matches!(v, 4 | 6 | 0) {
                return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_FAMILY));
            }
            family = v;
        } else if let Some(options) = value.as_object() {
            if let Ok(family_value) = options.get::<_, Value<'js>>("family") {
                if let Some(v) = family_value.as_int() {
                    if !matches!(v, 4 | 6 | 0) {
                        return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_FAMILY));
                    }
                    family = v;
                } else if let Some(v) = family_value.as_string() {
                    let family_string = v.to_string()?;
                    match family_string.as_str() {
                        "IPv4" => family = 4,
                        "IPv6" => family = 6,
                        _ => {
                            return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_FAMILY));
                        },
                    }
                } else if family_value.is_null() || family_value.is_undefined() {
                    // Use default family
                } else {
                    return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_FAMILY));
                }
            }

            if let Ok(all_value) = options.get::<_, bool>("all") {
                all = all_value;
            }

            if let Ok(order_val) = options.get::<_, Value<'js>>("order") {
                if let Some(order_str) = order_val.as_string() {
                    let order_text = order_str.to_string()?;
                    match order_text.as_str() {
                        "verbatim" => order = LookupOrder::Verbatim,
                        "ipv4first" => order = LookupOrder::Ipv4First,
                        "ipv6first" => order = LookupOrder::Ipv6First,
                        _ => {
                            return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_ORDER));
                        },
                    }
                } else if !order_val.is_null() && !order_val.is_undefined() {
                    return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_ORDER));
                } else if let Ok(verbatim) = options.get::<_, bool>("verbatim") {
                    order = if verbatim {
                        LookupOrder::Verbatim
                    } else {
                        LookupOrder::Ipv4First
                    };
                }
            } else if let Ok(verbatim) = options.get::<_, bool>("verbatim") {
                order = if verbatim {
                    LookupOrder::Verbatim
                } else {
                    LookupOrder::Ipv4First
                };
            }

            if let Ok(hints_val) = options.get::<_, Value<'js>>("hints") {
                if !hints_val.is_null() && !hints_val.is_undefined() {
                    if let Some(n) = hints_val.as_number() {
                        if !n.is_finite() || n.fract() != 0.0 {
                            return Err(throw_coded_error(
                                ctx,
                                "TypeError",
                                "ERR_INVALID_ARG_VALUE",
                                "The argument 'options.hints' is invalid",
                            ));
                        }
                        let mask = n as i64;
                        let allowed = i64::from(DNS_ADDRCONFIG | DNS_V4MAPPED | DNS_ALL);
                        if (mask & !allowed) != 0 {
                            return Err(throw_coded_error(
                                ctx,
                                "TypeError",
                                "ERR_INVALID_ARG_VALUE",
                                &format!(
                                    "The argument 'options.hints' is invalid. Received {mask}"
                                ),
                            ));
                        }
                        hints = mask as i32;
                    } else {
                        return Err(throw_coded_error(
                            ctx,
                            "TypeError",
                            "ERR_INVALID_ARG_TYPE",
                            "The \"options.hints\" property must be of type number",
                        ));
                    }
                }
            }
        } else if value.is_null() || value.is_undefined() {
            // Use default options
        } else {
            return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_FAMILY));
        }

        Ok(LookupOptions {
            family,
            all,
            order,
            hints,
        })
    }
}

/// Shared promise `lookup` function so `dns.promises.lookup` and `dns/promises`
/// export the same Function reference, stored in private ctx userdata.
pub fn shared_promises_lookup<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    if let Some(cached) = ctx.userdata::<DnsPromisesLookupCache>() {
        return Ok(cached.lookup.clone());
    }

    let lookup = Function::new(ctx.clone(), rquickjs::prelude::Async(lookup_promise))?;
    let cache = DnsPromisesLookupCache {
        lookup: lookup.clone(),
    };
    let _ = ctx.store_userdata(cache);
    if let Some(cached) = ctx.userdata::<DnsPromisesLookupCache>() {
        return Ok(cached.lookup.clone());
    }
    Ok(lookup)
}

#[derive(rquickjs::JsLifetime)]
struct DnsPromisesLookupCache<'js> {
    lookup: Function<'js>,
}

#[cfg(test)]
mod tests {
    use lexe_test::{call_test, call_test_err, test_async_with, ModuleEvaluator};
    use lexe_utils::primordials::{BasePrimordials, Primordial};

    use crate::DnsModule;

    #[tokio::test]
    async fn test_lookup() {
        test_async_with(|ctx| {
            Box::pin(async move {
                BasePrimordials::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<DnsModule>(ctx.clone(), "dns")
                    .await
                    .unwrap();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { lookup } from 'dns';

                        export async function test(hostname) {
                            return new Promise((resolve, reject) => {
                                lookup(hostname, (err, address, family) => {
                                    if (err) reject(err);
                                    else resolve(`${address}:${family}`);
                                });
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test::<String, _>(&ctx, &module, ("www.example.com",)).await;

                assert!(result.ends_with(":4") || result.ends_with(":6"));
            })
        })
        .await
    }

    #[tokio::test]
    async fn test_lookup_v6() {
        test_async_with(|ctx| {
            Box::pin(async move {
                BasePrimordials::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<DnsModule>(ctx.clone(), "dns")
                    .await
                    .unwrap();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { lookup } from 'dns';

                        export async function test(hostname) {
                            return new Promise((resolve, reject) => {
                                lookup(hostname, 6, (err, address, family) => {
                                    if (err) reject(err);
                                    else resolve(`${address}:${family}`);
                                });
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test_err::<String, _>(&ctx, &module, ("www.example.com",)).await;

                match result {
                    Ok(result) => assert!(result.ends_with(":6")),
                    Err(err) => assert!(err.to_string().contains("No address found")),
                }
            })
        })
        .await
    }

    #[tokio::test]
    async fn test_lookup_all() {
        test_async_with(|ctx| {
            Box::pin(async move {
                BasePrimordials::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<DnsModule>(ctx.clone(), "dns")
                    .await
                    .unwrap();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { lookup } from 'dns';

                        export async function test(hostname) {
                            return new Promise((resolve, reject) => {
                                lookup(hostname, { all: true }, (err, addresses) => {
                                    if (err) reject(err);
                                    else resolve(addresses.map(addr => `${addr.address}:${addr.family}`));
                                });
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test::<Vec<String>, _>(&ctx, &module, ("www.example.com",)).await;

                assert!(!result.is_empty());
                assert!(result
                    .iter()
                    .all(|addr| addr.ends_with(":4") || addr.ends_with(":6")));
            })
        })
        .await
    }

    #[tokio::test]
    async fn test_lookup_order() {
        test_async_with(|ctx| {
            Box::pin(async move {
                BasePrimordials::init(&ctx).unwrap();
                ModuleEvaluator::eval_rust::<DnsModule>(ctx.clone(), "dns")
                    .await
                    .unwrap();
                let module = ModuleEvaluator::eval_js(
                    ctx.clone(),
                    "test",
                    r#"
                        import { lookup } from 'dns';

                        export async function test(hostname) {
                            return new Promise((resolve, reject) => {
                                lookup(hostname, { all: true, order: 'ipv6first' }, (err, addresses) => {
                                    if (err) reject(err);
                                    else resolve(addresses.map(addr => `${addr.address}:${addr.family}`));
                                });
                            });
                        }
                    "#,
                )
                .await
                .unwrap();

                let result = call_test::<Vec<String>, _>(&ctx, &module, ("www.example.com",)).await;

                assert!(!result.is_empty());
                let first_ipv4 = result.iter().position(|addr| addr.ends_with(":4")).unwrap();
                let last_ipv6 = result
                    .iter()
                    .rposition(|addr| addr.ends_with(":6"))
                    .unwrap_or(result.len().saturating_sub(1));
                assert!(last_ipv6 <= first_ipv4);
            })
        })
        .await
    }
}
