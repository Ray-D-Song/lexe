// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::net::SocketAddr;
use std::result::Result as StdResult;

use either::Either;
use lexe_context::CtxExtension;
use lexe_hooking::{invoke_async_hook, register_finalization_registry, HookType};
use lexe_utils::{provider::ProviderType, result::ResultExt};
use rquickjs::{
    prelude::Opt, qjs, Ctx, Error, Exception, FromJs, Function, IntoJs, Null, Object, Result, Value,
};

const ERROR_MSG_OPTIONS_FAMILY: &str = "The argument 'family' must be one of: 0, 4, 6";
const ERROR_MSG_OPTIONS_ORDER: &str =
    "The argument 'order' must be one of: 'verbatim', 'ipv4first', 'ipv6first'";

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

    // SAFETY: Since it checks in advance whether it is an Function type, we can always get a pointer to the Function.
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
                        () =
                            cb.call((Exception::from_message(ctx.clone(), "No address found"),))?;
                    }
                }
                invoke_async_hook(&ctx, HookType::After, ProviderType::None, uid)?;
                Ok::<_, Error>(())
            },
            Err(err) => {
                invoke_async_hook(&ctx, HookType::Before, ProviderType::None, uid)?;
                () = cb.call((Exception::from_message(ctx.clone(), &err.to_string()),))?;
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
            None => Err(Exception::throw_message(&ctx, "No address found")),
        },
        Err(err) => Err(Exception::throw_message(&ctx, &err.to_string())),
    }
}

/// Shared DNS lookup core used by callback and Promise APIs.
pub async fn lookup_records(
    hostname: &str,
    options: LookupOptions,
) -> StdResult<Vec<LookupValue>, std::io::Error> {
    let mut addrs = tokio::net::lookup_host((hostname, 0))
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
    match options.order {
        LookupOrder::Verbatim => Ok(addrs),
        LookupOrder::Ipv4First => {
            addrs.sort_by_key(|a| a.family);
            Ok(addrs)
        },
        LookupOrder::Ipv6First => {
            addrs.sort_by_key(|b| std::cmp::Reverse(b.family));
            Ok(addrs)
        },
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
}

impl Default for LookupOptions {
    fn default() -> Self {
        Self {
            family: 0,
            all: false,
            order: LookupOrder::Verbatim,
        }
    }
}

impl<'js> FromJs<'js> for LookupOptions {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        let mut family = 0;
        let mut all = false;
        let mut order = LookupOrder::Verbatim;

        if let Some(v) = value.as_int() {
            if !matches!(v, 4 | 6 | 0) {
                return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_FAMILY));
            }
            family = v;
        } else if let Some(options) = value.as_object() {
            // Parse family
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

            // Parse all
            if let Ok(all_value) = options.get::<_, bool>("all") {
                all = all_value;
            }

            // Parse order
            if let Ok(order_value) = options.get::<_, String>("order") {
                match order_value.as_str() {
                    "verbatim" => order = LookupOrder::Verbatim,
                    "ipv4first" => order = LookupOrder::Ipv4First,
                    "ipv6first" => order = LookupOrder::Ipv6First,
                    _ => {
                        return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_ORDER));
                    },
                }
            }
        } else if value.is_null() || value.is_undefined() {
            // Use default options
        } else {
            return Err(Exception::throw_type(ctx, ERROR_MSG_OPTIONS_FAMILY));
        }

        Ok(LookupOptions { family, all, order })
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
    // Ignore AlreadyRegistered; prefer the stored instance for identity.
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

                // Not all systems support IPv6 resolution so we need to support it
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
                assert!(result.iter().all(|addr| addr.ends_with(":4") || addr.ends_with(":6")));
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
                let last_ipv6 = result.iter().rposition(|addr| addr.ends_with(":6")).unwrap_or(result.len().saturating_sub(1));
                assert!(last_ipv6 <= first_ipv4);
            })
        })
        .await
    }
}
