// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{cell::RefCell, rc::Rc};

use rquickjs::{
    atom::PredefinedAtom,
    class::{Trace, Tracer},
    function::Opt,
    prelude::List,
    Array, Class, Coerced, Ctx, Exception, FromJs, Function, IntoJs, JsLifetime, Object, Promise,
    Result, Value,
};

use super::{
    headers::{Headers, HeadersGuard},
    request::Request,
    response::Response,
};

#[derive(Clone)]
struct CacheEntry {
    request_url: String,
    request_method: String,
    request_headers: Vec<(String, String)>,
    response_url: String,
    response_status: u16,
    response_status_text: Option<String>,
    response_headers: Vec<(String, String)>,
    response_body: Option<Vec<u8>>,
}

type CacheEntries = Rc<RefCell<Vec<CacheEntry>>>;

#[derive(Default)]
struct CacheQueryOptions {
    ignore_search: bool,
    ignore_method: bool,
    ignore_vary: bool,
    cache_name: Option<String>,
}

impl CacheQueryOptions {
    fn from_opt<'js>(opt: Opt<Value<'js>>) -> Result<Self> {
        let Some(val) = opt.0 else {
            return Ok(Self::default());
        };
        if val.is_undefined() || val.is_null() {
            return Ok(Self::default());
        }
        let Some(obj) = val.as_object() else {
            return Ok(Self::default());
        };
        Ok(Self {
            ignore_search: obj.get::<_, Option<bool>>("ignoreSearch")?.unwrap_or(false),
            ignore_method: obj.get::<_, Option<bool>>("ignoreMethod")?.unwrap_or(false),
            ignore_vary: obj.get::<_, Option<bool>>("ignoreVary")?.unwrap_or(false),
            cache_name: obj.get::<_, Option<String>>("cacheName")?,
        })
    }
}

#[rquickjs::class]
#[derive(Clone, JsLifetime)]
pub struct Cache {
    entries: CacheEntries,
}

impl Trace<'_> for Cache {
    fn trace<'a>(&self, _tracer: Tracer<'a, '_>) {}
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Cache {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>) -> Result<Self> {
        Err(Exception::throw_type(&ctx, "Illegal constructor"))
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(Cache)
    }

    #[qjs(rename = "match")]
    pub async fn match_one<'js>(
        &self,
        ctx: Ctx<'js>,
        request: Value<'js>,
        options: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let opts = CacheQueryOptions::from_opt(options)?;
        let snapshot = normalize_request_for_lookup(&ctx, request)?;
        let found = {
            let entries = self.entries.borrow();
            entries
                .iter()
                .find(|entry| entry_matches(&snapshot, entry, &opts))
                .cloned()
        };
        match found {
            Some(entry) => build_cached_response(&ctx, &entry)?.into_js(&ctx),
            None => Ok(Value::new_undefined(ctx)),
        }
    }

    pub async fn match_all<'js>(
        &self,
        ctx: Ctx<'js>,
        request: Opt<Value<'js>>,
        options: Opt<Value<'js>>,
    ) -> Result<Array<'js>> {
        let opts = CacheQueryOptions::from_opt(options)?;
        let out = Array::new(ctx.clone())?;
        let matches: Vec<CacheEntry> = match request.0 {
            Some(req) if !req.is_undefined() => {
                let snapshot = normalize_request_for_lookup(&ctx, req)?;
                self.entries
                    .borrow()
                    .iter()
                    .filter(|entry| entry_matches(&snapshot, entry, &opts))
                    .cloned()
                    .collect()
            },
            _ => self.entries.borrow().clone(),
        };
        for (idx, entry) in matches.iter().enumerate() {
            out.set(idx, build_cached_response(&ctx, entry)?)?;
        }
        Ok(out)
    }

    pub async fn add<'js>(&self, ctx: Ctx<'js>, request: Value<'js>) -> Result<()> {
        let req_cls = coerce_request_class(&ctx, request)?;
        validate_cache_request_for_write(&ctx, &req_cls)?;
        let req_snapshot = request_snapshot_from_class(&ctx, &req_cls)?;
        let res_cls = call_global_fetch(&ctx, req_cls).await?;
        put_fetched_response(&ctx, &self.entries, req_snapshot, res_cls).await
    }

    pub async fn add_all<'js>(&self, ctx: Ctx<'js>, requests: Value<'js>) -> Result<()> {
        let arr = Array::from_js(&ctx, requests)?;
        let mut req_classes = Vec::with_capacity(arr.len());
        let mut req_snapshots = Vec::with_capacity(arr.len());
        for item in arr.iter::<Value<'js>>() {
            let val = item?;
            let req_cls = coerce_request_class(&ctx, val)?;
            validate_cache_request_for_write(&ctx, &req_cls)?;
            let snap = request_snapshot_from_class(&ctx, &req_cls)?;
            req_classes.push(req_cls);
            req_snapshots.push(snap);
        }

        // Spec rejects if duplicate matching requests are passed to addAll.
        let exact_opts = CacheQueryOptions::default();
        for i in 0..req_snapshots.len() {
            for j in (i + 1)..req_snapshots.len() {
                if urls_equal(&req_snapshots[i].url, &req_snapshots[j].url, false)
                    && req_snapshots[i].method == req_snapshots[j].method
                {
                    let dummy_entry = CacheEntry {
                        request_url: req_snapshots[j].url.clone(),
                        request_method: req_snapshots[j].method.clone(),
                        request_headers: req_snapshots[j].headers.clone(),
                        response_url: String::new(),
                        response_status: 200,
                        response_status_text: None,
                        response_headers: Vec::new(),
                        response_body: None,
                    };
                    if entry_matches(&req_snapshots[i], &dummy_entry, &exact_opts) {
                        return Err(Exception::throw_type(
                            &ctx,
                            "Cache.addAll received duplicate requests",
                        ));
                    }
                }
            }
        }

        let mut prepared = Vec::with_capacity(req_classes.len());
        for (req_cls, req_snap) in req_classes.into_iter().zip(req_snapshots) {
            let res_cls = call_global_fetch(&ctx, req_cls).await?;
            let entry = prepare_cache_entry(&ctx, req_snap, res_cls, true).await?;
            prepared.push(entry);
        }

        let mut entries = self.entries.borrow_mut();
        for entry in prepared {
            let snap = RequestSnapshot {
                url: entry.request_url.clone(),
                method: entry.request_method.clone(),
                headers: entry.request_headers.clone(),
            };
            entries.retain(|existing| !entry_matches(&snap, existing, &exact_opts));
            entries.push(entry);
        }
        Ok(())
    }

    pub async fn put<'js>(
        &self,
        ctx: Ctx<'js>,
        request: Value<'js>,
        response: Value<'js>,
    ) -> Result<()> {
        let req_cls = coerce_request_class(&ctx, request)?;
        validate_cache_request_for_write(&ctx, &req_cls)?;
        let res_cls = Class::<Response<'js>>::from_value(&response).map_err(|_| {
            Exception::throw_type(
                &ctx,
                "Failed to execute 'put' on 'Cache': parameter 2 is not of type 'Response'",
            )
        })?;
        let req_snapshot = request_snapshot_from_class(&ctx, &req_cls)?;
        let entry = prepare_cache_entry(&ctx, req_snapshot, res_cls, false).await?;
        let snap = RequestSnapshot {
            url: entry.request_url.clone(),
            method: entry.request_method.clone(),
            headers: entry.request_headers.clone(),
        };
        let exact_opts = CacheQueryOptions::default();
        let mut entries = self.entries.borrow_mut();
        entries.retain(|existing| !entry_matches(&snap, existing, &exact_opts));
        entries.push(entry);
        Ok(())
    }

    pub async fn delete<'js>(
        &self,
        ctx: Ctx<'js>,
        request: Value<'js>,
        options: Opt<Value<'js>>,
    ) -> Result<bool> {
        let opts = CacheQueryOptions::from_opt(options)?;
        let snapshot = normalize_request_for_lookup(&ctx, request)?;
        let mut entries = self.entries.borrow_mut();
        let before = entries.len();
        entries.retain(|entry| !entry_matches(&snapshot, entry, &opts));
        Ok(entries.len() != before)
    }

    pub async fn keys<'js>(
        &self,
        ctx: Ctx<'js>,
        request: Opt<Value<'js>>,
        options: Opt<Value<'js>>,
    ) -> Result<Array<'js>> {
        let opts = CacheQueryOptions::from_opt(options)?;
        let out = Array::new(ctx.clone())?;
        let matches: Vec<CacheEntry> = match request.0 {
            Some(req) if !req.is_undefined() => {
                let snapshot = normalize_request_for_lookup(&ctx, req)?;
                self.entries
                    .borrow()
                    .iter()
                    .filter(|entry| entry_matches(&snapshot, entry, &opts))
                    .cloned()
                    .collect()
            },
            _ => self.entries.borrow().clone(),
        };
        for (idx, entry) in matches.iter().enumerate() {
            out.set(idx, build_cached_request(&ctx, entry)?)?;
        }
        Ok(out)
    }
}

#[rquickjs::class]
#[derive(Clone, JsLifetime)]
pub struct CacheStorage {
    /// Ordered list of `(name, cache_entries)` preserves insertion order for `keys()` and `match()`.
    caches: Rc<RefCell<Vec<(String, CacheEntries)>>>,
}

impl Trace<'_> for CacheStorage {
    fn trace<'a>(&self, _tracer: Tracer<'a, '_>) {}
}

#[rquickjs::methods(rename_all = "camelCase")]
impl CacheStorage {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>) -> Result<Self> {
        Err(Exception::throw_type(&ctx, "Illegal constructor"))
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(CacheStorage)
    }

    pub async fn open<'js>(
        &self,
        ctx: Ctx<'js>,
        cache_name: Coerced<String>,
    ) -> Result<Class<'js, Cache>> {
        let name = cache_name.0;
        let entries = {
            let mut caches = self.caches.borrow_mut();
            if let Some((_, entries)) = caches.iter().find(|(n, _)| *n == name) {
                entries.clone()
            } else {
                let entries = Rc::new(RefCell::new(Vec::new()));
                caches.push((name, entries.clone()));
                entries
            }
        };
        Class::instance(ctx, Cache { entries })
    }

    pub async fn has(&self, cache_name: Coerced<String>) -> Result<bool> {
        let name = cache_name.0;
        Ok(self.caches.borrow().iter().any(|(n, _)| *n == name))
    }

    pub async fn delete(&self, cache_name: Coerced<String>) -> Result<bool> {
        let name = cache_name.0;
        let mut caches = self.caches.borrow_mut();
        let before = caches.len();
        caches.retain(|(n, _)| *n != name);
        Ok(caches.len() != before)
    }

    pub async fn keys<'js>(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        let out = Array::new(ctx)?;
        for (idx, (name, _)) in self.caches.borrow().iter().enumerate() {
            out.set(idx, name.as_str())?;
        }
        Ok(out)
    }

    #[qjs(rename = "match")]
    pub async fn match_one<'js>(
        &self,
        ctx: Ctx<'js>,
        request: Value<'js>,
        options: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let opts = CacheQueryOptions::from_opt(options)?;
        let snapshot = normalize_request_for_lookup(&ctx, request)?;

        let found = {
            let caches = self.caches.borrow();
            if let Some(cache_name) = opts.cache_name.as_deref() {
                caches
                    .iter()
                    .find(|(n, _)| n == cache_name)
                    .and_then(|(_, entries)| {
                        entries
                            .borrow()
                            .iter()
                            .find(|entry| entry_matches(&snapshot, entry, &opts))
                            .cloned()
                    })
            } else {
                let mut hit = None;
                for (_, entries) in caches.iter() {
                    if let Some(entry) = entries
                        .borrow()
                        .iter()
                        .find(|entry| entry_matches(&snapshot, entry, &opts))
                        .cloned()
                    {
                        hit = Some(entry);
                        break;
                    }
                }
                hit
            }
        };

        match found {
            Some(entry) => build_cached_response(&ctx, &entry)?.into_js(&ctx),
            None => Ok(Value::new_undefined(ctx)),
        }
    }
}

pub(crate) fn init_cache_api<'js>(ctx: &Ctx<'js>, globals: &Object<'js>) -> Result<()> {
    Class::<Cache>::define(globals)?;
    Class::<CacheStorage>::define(globals)?;
    let storage = Class::instance(
        ctx.clone(),
        CacheStorage {
            caches: Rc::new(RefCell::new(Vec::new())),
        },
    )?;
    globals.set("caches", storage)?;
    Ok(())
}

struct RequestSnapshot {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
}

fn coerce_request_class<'js>(
    ctx: &Ctx<'js>,
    request: Value<'js>,
) -> Result<Class<'js, Request<'js>>> {
    if let Ok(req_cls) = Class::<Request<'js>>::from_value(&request) {
        return Ok(req_cls);
    }
    let req = Request::new(ctx.clone(), request, Opt(None))?;
    Class::instance(ctx.clone(), req)
}

fn request_snapshot_from_class<'js>(
    ctx: &Ctx<'js>,
    req_cls: &Class<'js, Request<'js>>,
) -> Result<RequestSnapshot> {
    let req = req_cls.borrow();
    let url = req.url_string();
    let method = req.method_string();
    let headers = req
        .headers_vec(ctx)?
        .into_iter()
        .collect::<Vec<(String, String)>>();
    Ok(RequestSnapshot {
        url,
        method,
        headers,
    })
}

fn normalize_request_for_lookup<'js>(
    ctx: &Ctx<'js>,
    request: Value<'js>,
) -> Result<RequestSnapshot> {
    let req_cls = coerce_request_class(ctx, request)?;
    request_snapshot_from_class(ctx, &req_cls)
}

fn validate_cache_request_for_write<'js>(
    ctx: &Ctx<'js>,
    req_cls: &Class<'js, Request<'js>>,
) -> Result<()> {
    let req = req_cls.borrow();
    let method = req.method_string();
    if method != "GET" {
        return Err(Exception::throw_type(
            ctx,
            &format!("Failed to execute cache write: Request method '{method}' is unsupported"),
        ));
    }
    let url = req.url_string();
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        let scheme = url.split(':').next().unwrap_or("");
        return Err(Exception::throw_type(
            ctx,
            &format!("Failed to execute cache write: Request scheme '{scheme}' is unsupported"),
        ));
    }
    Ok(())
}

async fn call_global_fetch<'js>(
    ctx: &Ctx<'js>,
    req_cls: Class<'js, Request<'js>>,
) -> Result<Class<'js, Response<'js>>> {
    let fetch_fn: Function<'js> = ctx.globals().get("fetch")?;
    let promise: Promise<'js> = fetch_fn.call((req_cls,))?;
    let res_val: Value<'js> = promise.into_future().await?;
    Class::<Response<'js>>::from_value(&res_val).map_err(|_| {
        Exception::throw_type(ctx, "Cache.add expected fetch() to resolve to a Response")
    })
}

async fn put_fetched_response<'js>(
    ctx: &Ctx<'js>,
    entries: &Rc<RefCell<Vec<CacheEntry>>>,
    req_snapshot: RequestSnapshot,
    res_cls: Class<'js, Response<'js>>,
) -> Result<()> {
    let entry = prepare_cache_entry(ctx, req_snapshot, res_cls, true).await?;
    let snap = RequestSnapshot {
        url: entry.request_url.clone(),
        method: entry.request_method.clone(),
        headers: entry.request_headers.clone(),
    };
    let exact_opts = CacheQueryOptions::default();
    let mut list = entries.borrow_mut();
    list.retain(|existing| !entry_matches(&snap, existing, &exact_opts));
    list.push(entry);
    Ok(())
}

async fn prepare_cache_entry<'js>(
    ctx: &Ctx<'js>,
    req_snapshot: RequestSnapshot,
    res_cls: Class<'js, Response<'js>>,
    require_ok: bool,
) -> Result<CacheEntry> {
    let (status, status_text, response_url, response_headers) = {
        let res = res_cls.borrow();
        let status = res.status() as u16;
        if require_ok && !(200..300).contains(&status) {
            return Err(Exception::throw_type(
                ctx,
                &format!("Request failed with status {status}"),
            ));
        }
        if status == 206 {
            return Err(Exception::throw_type(
                ctx,
                "Partial response (status code 206) is unsupported",
            ));
        }
        if status == 0 {
            return Err(Exception::throw_type(
                ctx,
                "Cannot cache network error response",
            ));
        }
        let headers = res.headers_vec(ctx)?;
        for (name, val) in &headers {
            if name.eq_ignore_ascii_case("vary") && val.split(',').any(|t| t.trim() == "*") {
                return Err(Exception::throw_type(ctx, "Vary header contains *"));
            }
        }
        (status, res.status_text_opt(), res.url(), headers)
    };

    let body_bytes = Response::consume_body_bytes(&res_cls, ctx).await?;

    Ok(CacheEntry {
        request_url: req_snapshot.url,
        request_method: req_snapshot.method,
        request_headers: req_snapshot.headers,
        response_url,
        response_status: status,
        response_status_text: status_text,
        response_headers,
        response_body: body_bytes,
    })
}

fn strip_fragment(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

fn strip_query_and_fragment(url: &str) -> &str {
    let without_frag = strip_fragment(url);
    without_frag.split('?').next().unwrap_or(without_frag)
}

fn urls_equal(a: &str, b: &str, ignore_search: bool) -> bool {
    if ignore_search {
        strip_query_and_fragment(a) == strip_query_and_fragment(b)
    } else {
        strip_fragment(a) == strip_fragment(b)
    }
}

fn find_header_values(headers: &[(String, String)], target: &str) -> Vec<String> {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(target))
        .map(|(_, v)| v.clone())
        .collect()
}

fn entry_matches(req: &RequestSnapshot, entry: &CacheEntry, opts: &CacheQueryOptions) -> bool {
    if !opts.ignore_method && req.method != entry.request_method {
        return false;
    }
    if !urls_equal(&req.url, &entry.request_url, opts.ignore_search) {
        return false;
    }
    if opts.ignore_vary {
        return true;
    }
    for (_, val) in entry
        .response_headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("vary"))
    {
        for field in val.split(',').map(str::trim).filter(|f| !f.is_empty()) {
            if field == "*" {
                return false;
            }
            let mut req_vals = find_header_values(&req.headers, field);
            let mut cached_vals = find_header_values(&entry.request_headers, field);
            req_vals.sort();
            cached_vals.sort();
            if req_vals != cached_vals {
                return false;
            }
        }
    }
    true
}

fn build_cached_response<'js>(ctx: &Ctx<'js>, entry: &CacheEntry) -> Result<Response<'js>> {
    let headers = Headers::from_pairs(entry.response_headers.clone(), HeadersGuard::Response);
    Response::from_buffered_bytes(
        ctx.clone(),
        entry.response_body.as_deref().unwrap_or(&[]),
        entry.response_status,
        entry.response_status_text.clone(),
        entry.response_url.clone(),
        headers,
        false,
        entry.request_method.clone(),
        None,
        entry.response_body.is_none(),
    )
}

fn build_cached_request<'js>(ctx: &Ctx<'js>, entry: &CacheEntry) -> Result<Request<'js>> {
    let init = Object::new(ctx.clone())?;
    init.set("method", entry.request_method.as_str())?;
    let hdr_arr = Array::new(ctx.clone())?;
    for (idx, (k, v)) in entry.request_headers.iter().enumerate() {
        hdr_arr.set(idx, List((k.as_str(), v.as_str())))?;
    }
    init.set("headers", hdr_arr)?;
    Request::new(
        ctx.clone(),
        entry.request_url.clone().into_js(ctx)?,
        Opt(Some(init.into_value())),
    )
}
