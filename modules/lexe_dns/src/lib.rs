// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use lexe_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::{Async, Func},
    Class, Ctx, Function, Object, Result, Value,
};

use crate::lookup::{
    get_default_result_order, lookup, lookup_service_callback, lookup_service_promise,
    set_default_result_order, shared_promises_lookup, DNS_ADDRCONFIG, DNS_ALL, DNS_V4MAPPED,
};
use crate::resolver::{default_resolver_state, NativeResolver};

mod lookup;
mod resolver;
mod wire;

const DNS_ERROR_CONSTANTS: [(&str, &str); 24] = [
    ("NODATA", "ENODATA"),
    ("FORMERR", "EFORMERR"),
    ("SERVFAIL", "ESERVFAIL"),
    ("NOTFOUND", "ENOTFOUND"),
    ("NOTIMP", "ENOTIMP"),
    ("REFUSED", "EREFUSED"),
    ("BADQUERY", "EBADQUERY"),
    ("BADNAME", "EBADNAME"),
    ("BADFAMILY", "EBADFAMILY"),
    ("BADRESP", "EBADRESP"),
    ("CONNREFUSED", "ECONNREFUSED"),
    ("TIMEOUT", "ETIMEOUT"),
    ("EOF", "EOF"),
    ("FILE", "EFILE"),
    ("NOMEM", "ENOMEM"),
    ("DESTRUCTION", "EDESTRUCTION"),
    ("BADSTR", "EBADSTR"),
    ("BADFLAGS", "EBADFLAGS"),
    ("NONAME", "ENONAME"),
    ("BADHINTS", "EBADHINTS"),
    ("NOTINITIALIZED", "ENOTINITIALIZED"),
    ("LOADIPHLPAPI", "ELOADIPHLPAPI"),
    ("ADDRGETNETWORKPARAMS", "EADDRGETNETWORKPARAMS"),
    ("CANCELLED", "ECANCELLED"),
];

const DNS_JS_FACTORY: &str = r#"(function (
  NativeResolver,
  defaultHandle,
  lookupPromise,
  lookupServicePromise,
  getDefaultResultOrder,
  setDefaultResultOrder
) {
  "use strict";

  const VALID_RRTYPES = new Set([
    "A", "AAAA", "ANY", "CAA", "CNAME", "MX", "NAPTR",
    "NS", "PTR", "SOA", "SRV", "TLSA", "TXT",
  ]);

  function makeCodedError(Ctor, code, msg) {
    const err = new Ctor(msg);
    err.code = code;
    return err;
  }

  function validateHostname(hostname) {
    if (typeof hostname !== "string") {
      throw makeCodedError(TypeError, "ERR_INVALID_ARG_TYPE", 'The "hostname" argument must be of type string');
    }
  }

  function validateRrtype(rrtype) {
    if (typeof rrtype !== "string") {
      throw makeCodedError(TypeError, "ERR_INVALID_ARG_TYPE", 'The "rrtype" argument must be of type string');
    }
    if (!VALID_RRTYPES.has(rrtype)) {
      throw makeCodedError(TypeError, "ERR_INVALID_ARG_VALUE", `The argument 'rrtype' is invalid. Received '${rrtype}'`);
    }
  }

  function validateCallback(cb) {
    if (typeof cb !== "function") {
      throw makeCodedError(TypeError, "ERR_INVALID_ARG_TYPE", 'The "callback" argument must be of type function');
    }
  }

  function parseResolverOptions(options) {
    if (options === undefined || options === null) {
      return { timeout: -1, tries: 4 };
    }
    if (typeof options !== "object") {
      throw makeCodedError(TypeError, "ERR_INVALID_ARG_TYPE", 'The "options" argument must be of type object');
    }
    const timeout = typeof options.timeout === "number" ? Math.trunc(options.timeout) : -1;
    const tries = typeof options.tries === "number" && options.tries > 0 ? Math.trunc(options.tries) : 4;
    return { timeout, tries };
  }

  class ResolverBase {
    constructor(options) {
      if (options instanceof NativeResolver) {
        this._handle = options;
      } else {
        const parsed = parseResolverOptions(options);
        this._handle = new NativeResolver(parsed.timeout, parsed.tries);
      }
    }

    cancel() {
      this._handle.cancel();
    }

    getServers() {
      return this._handle.getServers();
    }

    setServers(servers) {
      this._handle.setServers(servers);
    }

    setLocalAddress(ipv4, ipv6) {
      this._handle.setLocalAddress(ipv4, ipv6);
    }
  }

  class PromisesResolver extends ResolverBase {
    resolve(hostname, rrtype = "A") {
      validateHostname(hostname);
      validateRrtype(rrtype);
      return this._handle.query(hostname, rrtype, false);
    }

    resolve4(hostname, options) {
      validateHostname(hostname);
      const ttl = Boolean(options && typeof options === "object" && options.ttl);
      return this._handle.query(hostname, "A", ttl);
    }

    resolve6(hostname, options) {
      validateHostname(hostname);
      const ttl = Boolean(options && typeof options === "object" && options.ttl);
      return this._handle.query(hostname, "AAAA", ttl);
    }

    resolveAny(hostname) { validateHostname(hostname); return this._handle.query(hostname, "ANY", false); }
    resolveCaa(hostname) { validateHostname(hostname); return this._handle.query(hostname, "CAA", false); }
    resolveCname(hostname) { validateHostname(hostname); return this._handle.query(hostname, "CNAME", false); }
    resolveMx(hostname) { validateHostname(hostname); return this._handle.query(hostname, "MX", false); }
    resolveNaptr(hostname) { validateHostname(hostname); return this._handle.query(hostname, "NAPTR", false); }
    resolveNs(hostname) { validateHostname(hostname); return this._handle.query(hostname, "NS", false); }
    resolvePtr(hostname) { validateHostname(hostname); return this._handle.query(hostname, "PTR", false); }
    resolveSoa(hostname) { validateHostname(hostname); return this._handle.query(hostname, "SOA", false); }
    resolveSrv(hostname) { validateHostname(hostname); return this._handle.query(hostname, "SRV", false); }
    resolveTlsa(hostname) { validateHostname(hostname); return this._handle.query(hostname, "TLSA", false); }
    resolveTxt(hostname) { validateHostname(hostname); return this._handle.query(hostname, "TXT", false); }

    reverse(ip) {
      this._handle.validateReverseIp(ip);
      return this._handle.reverse(ip);
    }
  }

  class CallbackResolver extends ResolverBase {
    resolve(hostname, rrtypeOrCb, cb) {
      let rrtype = rrtypeOrCb;
      if (typeof rrtypeOrCb === "function") {
        cb = rrtypeOrCb;
        rrtype = "A";
      }
      validateHostname(hostname);
      validateRrtype(rrtype);
      validateCallback(cb);
      this._handle.query(hostname, rrtype, false).then(
        (res) => cb(null, res),
        (err) => cb(err)
      );
    }

    resolve4(hostname, optionsOrCb, cb) {
      let ttl = false;
      if (typeof optionsOrCb === "function") {
        cb = optionsOrCb;
      } else if (optionsOrCb && typeof optionsOrCb === "object") {
        ttl = Boolean(optionsOrCb.ttl);
      }
      validateHostname(hostname);
      validateCallback(cb);
      this._handle.query(hostname, "A", ttl).then(
        (res) => cb(null, res),
        (err) => cb(err)
      );
    }

    resolve6(hostname, optionsOrCb, cb) {
      let ttl = false;
      if (typeof optionsOrCb === "function") {
        cb = optionsOrCb;
      } else if (optionsOrCb && typeof optionsOrCb === "object") {
        ttl = Boolean(optionsOrCb.ttl);
      }
      validateHostname(hostname);
      validateCallback(cb);
      this._handle.query(hostname, "AAAA", ttl).then(
        (res) => cb(null, res),
        (err) => cb(err)
      );
    }

    reverse(ip, cb) {
      const checked = this._handle.validateReverseIp(ip);
      validateCallback(cb);
      this._handle.reverse(checked).then(
        (res) => cb(null, res),
        (err) => cb(err)
      );
    }
  }

  for (const [method, rrtype] of [
    ["resolveAny", "ANY"],
    ["resolveCaa", "CAA"],
    ["resolveCname", "CNAME"],
    ["resolveMx", "MX"],
    ["resolveNaptr", "NAPTR"],
    ["resolveNs", "NS"],
    ["resolvePtr", "PTR"],
    ["resolveSoa", "SOA"],
    ["resolveSrv", "SRV"],
    ["resolveTlsa", "TLSA"],
    ["resolveTxt", "TXT"],
  ]) {
    CallbackResolver.prototype[method] = function (hostname, cb) {
      validateHostname(hostname);
      validateCallback(cb);
      this._handle.query(hostname, rrtype, false).then(
        (res) => cb(null, res),
        (err) => cb(err)
      );
    };
  }

  // Name constructors nicely for diagnostics without leaking to globalThis
  Object.defineProperty(PromisesResolver, "name", { value: "Resolver" });
  Object.defineProperty(CallbackResolver, "name", { value: "Resolver" });

  const defaultPromiseResolver = new PromisesResolver(defaultHandle);
  const defaultCallbackResolver = new CallbackResolver(defaultHandle);

  const promises = {
    Resolver: PromisesResolver,
    lookup: lookupPromise,
    lookupService: lookupServicePromise,
    getDefaultResultOrder,
    setDefaultResultOrder,
    getServers: () => defaultPromiseResolver.getServers(),
    setServers: (servers) => defaultPromiseResolver.setServers(servers),
    resolve: (hostname, rrtype) => defaultPromiseResolver.resolve(hostname, rrtype),
    resolve4: (hostname, opts) => defaultPromiseResolver.resolve4(hostname, opts),
    resolve6: (hostname, opts) => defaultPromiseResolver.resolve6(hostname, opts),
    resolveAny: (hostname) => defaultPromiseResolver.resolveAny(hostname),
    resolveCaa: (hostname) => defaultPromiseResolver.resolveCaa(hostname),
    resolveCname: (hostname) => defaultPromiseResolver.resolveCname(hostname),
    resolveMx: (hostname) => defaultPromiseResolver.resolveMx(hostname),
    resolveNaptr: (hostname) => defaultPromiseResolver.resolveNaptr(hostname),
    resolveNs: (hostname) => defaultPromiseResolver.resolveNs(hostname),
    resolvePtr: (hostname) => defaultPromiseResolver.resolvePtr(hostname),
    resolveSoa: (hostname) => defaultPromiseResolver.resolveSoa(hostname),
    resolveSrv: (hostname) => defaultPromiseResolver.resolveSrv(hostname),
    resolveTlsa: (hostname) => defaultPromiseResolver.resolveTlsa(hostname),
    resolveTxt: (hostname) => defaultPromiseResolver.resolveTxt(hostname),
    reverse: (ip) => defaultPromiseResolver.reverse(ip),
  };

  const callbacks = {
    Resolver: CallbackResolver,
    getServers: () => defaultCallbackResolver.getServers(),
    setServers: (servers) => defaultCallbackResolver.setServers(servers),
    resolve: (hostname, rrtypeOrCb, cb) => defaultCallbackResolver.resolve(hostname, rrtypeOrCb, cb),
    resolve4: (hostname, optsOrCb, cb) => defaultCallbackResolver.resolve4(hostname, optsOrCb, cb),
    resolve6: (hostname, optsOrCb, cb) => defaultCallbackResolver.resolve6(hostname, optsOrCb, cb),
    resolveAny: (hostname, cb) => defaultCallbackResolver.resolveAny(hostname, cb),
    resolveCaa: (hostname, cb) => defaultCallbackResolver.resolveCaa(hostname, cb),
    resolveCname: (hostname, cb) => defaultCallbackResolver.resolveCname(hostname, cb),
    resolveMx: (hostname, cb) => defaultCallbackResolver.resolveMx(hostname, cb),
    resolveNaptr: (hostname, cb) => defaultCallbackResolver.resolveNaptr(hostname, cb),
    resolveNs: (hostname, cb) => defaultCallbackResolver.resolveNs(hostname, cb),
    resolvePtr: (hostname, cb) => defaultCallbackResolver.resolvePtr(hostname, cb),
    resolveSoa: (hostname, cb) => defaultCallbackResolver.resolveSoa(hostname, cb),
    resolveSrv: (hostname, cb) => defaultCallbackResolver.resolveSrv(hostname, cb),
    resolveTlsa: (hostname, cb) => defaultCallbackResolver.resolveTlsa(hostname, cb),
    resolveTxt: (hostname, cb) => defaultCallbackResolver.resolveTxt(hostname, cb),
    reverse: (ip, cb) => defaultCallbackResolver.reverse(ip, cb),
  };

  return { promises, callbacks };
})"#;

#[derive(rquickjs::JsLifetime)]
struct DnsRuntimeCache<'js> {
    promises: Object<'js>,
    callbacks: Object<'js>,
}

fn shared_dns_objects<'js>(ctx: &Ctx<'js>) -> Result<(Object<'js>, Object<'js>)> {
    if let Some(cached) = ctx.userdata::<DnsRuntimeCache>() {
        return Ok((cached.promises.clone(), cached.callbacks.clone()));
    }

    let native_resolver_ctor = Class::<NativeResolver>::create_constructor(ctx)?
        .expect("NativeResolver must have constructor");

    let default_handle = Class::instance(
        ctx.clone(),
        NativeResolver::from_state(default_resolver_state()),
    )?;
    let promises_lookup = shared_promises_lookup(ctx)?;
    let promises_lookup_service = Function::new(ctx.clone(), Async(lookup_service_promise))?;
    let get_order_fn = Function::new(ctx.clone(), get_default_result_order)?;
    let set_order_fn = Function::new(ctx.clone(), set_default_result_order)?;

    let factory: Function = ctx.eval(DNS_JS_FACTORY)?;
    let bundle: Object = factory.call((
        native_resolver_ctor,
        default_handle,
        promises_lookup,
        promises_lookup_service,
        get_order_fn,
        set_order_fn,
    ))?;

    let promises: Object = bundle.get("promises")?;
    for (k, v) in DNS_ERROR_CONSTANTS {
        promises.set(k, v)?;
    }
    let callbacks: Object = bundle.get("callbacks")?;

    let _ = ctx.store_userdata(DnsRuntimeCache {
        promises: promises.clone(),
        callbacks: callbacks.clone(),
    });
    if let Some(cached) = ctx.userdata::<DnsRuntimeCache>() {
        return Ok((cached.promises.clone(), cached.callbacks.clone()));
    }
    Ok((promises, callbacks))
}

pub struct DnsModule;

impl ModuleDef for DnsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        for (k, _) in DNS_ERROR_CONSTANTS {
            declare.declare(k)?;
        }
        declare.declare("ADDRCONFIG")?;
        declare.declare("V4MAPPED")?;
        declare.declare("ALL")?;

        declare.declare("lookup")?;
        declare.declare("lookupService")?;
        declare.declare("getDefaultResultOrder")?;
        declare.declare("setDefaultResultOrder")?;
        declare.declare("getServers")?;
        declare.declare("setServers")?;
        declare.declare("Resolver")?;
        declare.declare("resolve")?;
        declare.declare("resolve4")?;
        declare.declare("resolve6")?;
        declare.declare("resolveAny")?;
        declare.declare("resolveCaa")?;
        declare.declare("resolveCname")?;
        declare.declare("resolveMx")?;
        declare.declare("resolveNaptr")?;
        declare.declare("resolveNs")?;
        declare.declare("resolvePtr")?;
        declare.declare("resolveSoa")?;
        declare.declare("resolveSrv")?;
        declare.declare("resolveTlsa")?;
        declare.declare("resolveTxt")?;
        declare.declare("reverse")?;
        declare.declare("promises")?;

        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let (promises, callbacks) = shared_dns_objects(ctx)?;

        export_default(ctx, exports, |default| {
            for (k, v) in DNS_ERROR_CONSTANTS {
                default.set(k, v)?;
            }
            default.set("ADDRCONFIG", DNS_ADDRCONFIG)?;
            default.set("V4MAPPED", DNS_V4MAPPED)?;
            default.set("ALL", DNS_ALL)?;

            default.set("lookup", Func::from(lookup))?;
            default.set("lookupService", Func::from(lookup_service_callback))?;
            default.set(
                "getDefaultResultOrder",
                Func::from(get_default_result_order),
            )?;
            default.set(
                "setDefaultResultOrder",
                Func::from(set_default_result_order),
            )?;

            for key in [
                "Resolver",
                "getServers",
                "setServers",
                "resolve",
                "resolve4",
                "resolve6",
                "resolveAny",
                "resolveCaa",
                "resolveCname",
                "resolveMx",
                "resolveNaptr",
                "resolveNs",
                "resolvePtr",
                "resolveSoa",
                "resolveSrv",
                "resolveTlsa",
                "resolveTxt",
                "reverse",
            ] {
                default.set(key, callbacks.get::<_, Value>(key)?)?;
            }

            default.set("promises", promises)?;
            Ok(())
        })?;

        Ok(())
    }
}

impl From<DnsModule> for ModuleInfo<DnsModule> {
    fn from(val: DnsModule) -> Self {
        ModuleInfo {
            name: "dns",
            module: val,
        }
    }
}

pub struct DnsPromisesModule;

impl ModuleDef for DnsPromisesModule {
    fn declare(declare: &Declarations) -> Result<()> {
        for (k, _) in DNS_ERROR_CONSTANTS {
            declare.declare(k)?;
        }
        declare.declare("lookup")?;
        declare.declare("lookupService")?;
        declare.declare("getDefaultResultOrder")?;
        declare.declare("setDefaultResultOrder")?;
        declare.declare("getServers")?;
        declare.declare("setServers")?;
        declare.declare("Resolver")?;
        declare.declare("resolve")?;
        declare.declare("resolve4")?;
        declare.declare("resolve6")?;
        declare.declare("resolveAny")?;
        declare.declare("resolveCaa")?;
        declare.declare("resolveCname")?;
        declare.declare("resolveMx")?;
        declare.declare("resolveNaptr")?;
        declare.declare("resolveNs")?;
        declare.declare("resolvePtr")?;
        declare.declare("resolveSoa")?;
        declare.declare("resolveSrv")?;
        declare.declare("resolveTlsa")?;
        declare.declare("resolveTxt")?;
        declare.declare("reverse")?;

        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let (promises, _) = shared_dns_objects(ctx)?;

        export_default(ctx, exports, |default| {
            for key in promises.keys::<String>() {
                let key = key?;
                default.set(&key, promises.get::<_, Value>(&key)?)?;
            }
            Ok(())
        })?;

        Ok(())
    }
}

impl From<DnsPromisesModule> for ModuleInfo<DnsPromisesModule> {
    fn from(val: DnsPromisesModule) -> Self {
        ModuleInfo {
            name: "dns/promises",
            module: val,
        }
    }
}
