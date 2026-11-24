// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::inherent_to_string)]
pub mod url_class;
pub mod url_search_params;

use lexe_utils::{
    module::{export_default, ModuleInfo},
    primordials::{BasePrimordials, Primordial},
};
use rquickjs::{
    function::{Constructor, Func},
    module::{Declarations, Exports, ModuleDef},
    prelude::Opt,
    Class, Coerced, Ctx, Exception, Function, Object, Result, Value,
};
use url::{quirks, Url};

use self::url_class::{url_to_http_options, URL};
use self::url_search_params::URLSearchParams;

pub fn domain_to_unicode(domain: &str) -> String {
    quirks::domain_to_unicode(domain)
}

/// IDNA ToASCII without WHATWG host parsing, like Node's `internalBinding('encoding_binding').toASCII`:
/// numeric labels stay as they are and failures yield an empty string.
fn idna_to_ascii(domain: &str) -> String {
    idna::domain_to_ascii_cow(domain.as_bytes(), idna::AsciiDenyList::EMPTY)
        .map(|s| s.into_owned())
        .unwrap_or_default()
}

pub fn domain_to_ascii(domain: &str) -> String {
    quirks::domain_to_ascii(domain)
}

// Node's `pathToFileURL`. Anything that is not absolute is resolved against the
// current working directory first (Node does `path.resolve(path)`), so a
// relative input never fails — it only fails if the resolved path is unusable.
pub fn path_to_file_url<'js>(ctx: Ctx<'js>, path: String, _: Opt<Value<'js>>) -> Result<URL<'js>> {
    let resolved = if std::path::Path::new(&path).is_absolute() {
        std::path::PathBuf::from(&path)
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(&path),
            Err(_) => {
                return Err(throw_coded(
                    &ctx,
                    CodedError::Type,
                    "ERR_INVALID_ARG_VALUE",
                    &[
                        "The argument 'path' must be a string or Uint8Array. Received ",
                        &path,
                    ]
                    .concat(),
                ));
            },
        }
    };

    let mut url = Url::from_file_path(&resolved).map_err(|_| {
        throw_coded(
            &ctx,
            CodedError::Type,
            "ERR_INVALID_ARG_VALUE",
            &[
                "The argument 'path' must be a string or Uint8Array. Received ",
                &path,
            ]
            .concat(),
        )
    })?;

    // `Url::from_file_path` normalises away a trailing separator; Node keeps it,
    // which matters for `fileURLToPath(pathToFileURL("/tmp/a/"))`.
    if path.ends_with('/') || path.ends_with(std::path::MAIN_SEPARATOR) {
        let mut with_sep = url.path().to_string();
        if !with_sep.ends_with('/') {
            with_sep.push('/');
            url.set_path(&with_sep);
        }
    }

    URL::from_url(ctx, url)
}

/// Strict percent-decoding of a URL path.
///
/// Returns `None` for a malformed escape (`%zz`, `%2`, `%` at end), which is what
/// makes Node raise `URIError("URI malformed")`. The `url` crate leaves invalid
/// escapes untouched while parsing, so this is the first place they are noticed.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        if i + 2 >= bytes.len() {
            return None;
        }
        let hi = (bytes[i + 1] as char).to_digit(16)?;
        let lo = (bytes[i + 2] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 3;
    }
    String::from_utf8(out).ok()
}

/// Throw a `TypeError` carrying a Node `code` property (same helper shape used by
/// `lexe_net` and `lexe_sqlite`).
fn throw_coded(ctx: &Ctx<'_>, kind: CodedError, code: &str, message: &str) -> rquickjs::Error {
    let primordials = lexe_utils::primordials::BasePrimordials::get(ctx);
    let Ok(primordials) = primordials else {
        return Exception::throw_message(ctx, message);
    };
    let ctor = match kind {
        CodedError::Type => &primordials.constructor_type_error,
        CodedError::Uri => return throw_uri_error(ctx, message),
    };
    let Ok(err) = ctor.construct((message.to_string(),)) else {
        return Exception::throw_message(ctx, message);
    };
    let err: Object<'_> = err;
    let _ = err.set("code", code);
    ctx.throw(err.into_value())
}

#[derive(Clone, Copy)]
enum CodedError {
    Type,
    /// `URIError` with no `code` — what Node raises for malformed percent escapes.
    Uri,
}

fn throw_uri_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    // Primordials does not carry URIError, so reach for the global constructor.
    let ctor: Option<Constructor<'_>> = ctx.globals().get("URIError").ok();
    let Some(ctor) = ctor else {
        return Exception::throw_message(ctx, message);
    };
    match ctor.construct::<(String,), Object<'_>>((message.to_string(),)) {
        Ok(err) => ctx.throw(err.into_value()),
        Err(_) => Exception::throw_message(ctx, message),
    }
}

/// Options accepted by `fileURLToPath` — currently just `windows`.
fn read_windows_option(value: Option<Value<'_>>) -> bool {
    let Some(value) = value else {
        return cfg!(windows);
    };
    let Some(options) = value.as_object() else {
        return cfg!(windows);
    };
    match options.get::<_, Option<bool>>("windows") {
        Ok(Some(flag)) => flag,
        _ => cfg!(windows),
    }
}

/// Node-compatible `fileURLToPath(url[, options])`.
///
/// Error `code`s follow Node exactly, and that includes the one error that has
/// **no** code: a malformed percent escape raises `URIError("URI malformed")`
/// bare. The goal is parity with Node, not internal consistency.
pub fn file_url_to_path<'js>(
    ctx: Ctx<'js>,
    url: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<String> {
    let windows = read_windows_option(options.into_inner());

    let parsed = if let Ok(url) = Class::<URL>::from_value(&url) {
        let borrowed = url.borrow();
        let inner = borrowed.inner();
        if inner.scheme() != "file" {
            return Err(throw_coded(
                &ctx,
                CodedError::Type,
                "ERR_INVALID_URL_SCHEME",
                "The URL must be of scheme file",
            ));
        }
        inner.clone()
    } else {
        let url_string = url.get::<Coerced<String>>()?.to_string();
        match Url::parse(&url_string) {
            Ok(parsed) => {
                if parsed.scheme() != "file" {
                    return Err(throw_coded(
                        &ctx,
                        CodedError::Type,
                        "ERR_INVALID_URL_SCHEME",
                        "The URL must be of scheme file",
                    ));
                }
                parsed
            },
            Err(_) => {
                return Err(throw_coded(
                    &ctx,
                    CodedError::Type,
                    "ERR_INVALID_URL",
                    "Invalid URL",
                ));
            },
        }
    };

    file_path_from_url_opts(&ctx, &parsed, windows)
}

/// Shared by `fileURLToPath` and the `node:url` helpers above.
pub fn file_path_from_url(ctx: &Ctx<'_>, url: &Url) -> Result<String> {
    file_path_from_url_opts(ctx, url, cfg!(windows))
}

fn file_path_from_url_opts(ctx: &Ctx<'_>, url: &Url, windows: bool) -> Result<String> {
    if url.scheme() != "file" {
        return Err(throw_coded(
            ctx,
            CodedError::Type,
            "ERR_INVALID_URL_SCHEME",
            "The URL must be of scheme file",
        ));
    }

    // A non-empty, non-localhost host is an error. Node's message names the
    // current platform ("on linux"); we match the code and keep the wording.
    let host = url.host_str().unwrap_or("");
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return Err(throw_coded(
            ctx,
            CodedError::Type,
            "ERR_INVALID_FILE_URL_HOST",
            &format!(
                "File URL host must be \"localhost\" or empty on {}",
                std::env::consts::OS
            ),
        ));
    }

    // `%2F` (and `%5C` under Windows) must be rejected *before* decoding.
    reject_encoded_path_separators(ctx, url, windows)?;

    // Decoding itself: a malformed escape makes Node throw URIError, unchanged.
    let decoded = match percent_decode(url.path()) {
        Some(decoded) => decoded,
        // Node raises a bare `URIError("URI malformed")` here, with no `code`.
        None => return Err(throw_uri_error(ctx, "URI malformed")),
    };

    if windows {
        windows_path_from_url(ctx, &decoded)
    } else {
        Ok(decoded)
    }
}

/// Windows form: `/C:/x` becomes `C:\x`, and a path that is not driven or UNC
/// absolute is rejected. Node: `getPathFromURLWin32`.
fn windows_path_from_url(ctx: &Ctx<'_>, decoded: &str) -> Result<String> {
    let mut path = decoded.to_string();
    if path.starts_with('/') && path.len() >= 3 {
        let bytes = path.as_bytes();
        // "/C:/..." → "C:\..."
        if bytes[2] == b':' && bytes[1].is_ascii_alphabetic() {
            path.remove(0);
        }
    }
    if !(path.starts_with("\\\\")
        || (path.len() >= 3
            && path.as_bytes()[1] == b':'
            && path.as_bytes()[0].is_ascii_alphabetic()))
    {
        return Err(throw_coded(
            ctx,
            CodedError::Type,
            "ERR_INVALID_FILE_URL_PATH",
            "File URL path must be absolute",
        ));
    }
    Ok(path.replace('/', "\\"))
}

/// Node rejects percent-encoded `/` (`%2F`) on all platforms, and `%5C` (`\`) on Windows,
/// before decoding the path. See `getPathFromURLPosix` / `getPathFromURLWin32`.
fn reject_encoded_path_separators(ctx: &Ctx<'_>, url: &Url, windows: bool) -> Result<()> {
    let pathname = url.path();
    let bytes = pathname.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' {
            let second = bytes[i + 1];
            let third = bytes[i + 2] | 0x20;
            if second == b'2' && third == b'f' {
                let message = if windows {
                    "File URL path must not include encoded \\ or / characters"
                } else {
                    "File URL path must not include encoded / characters"
                };
                return Err(throw_coded(
                    ctx,
                    CodedError::Type,
                    "ERR_INVALID_FILE_URL_PATH",
                    message,
                ));
            }
            #[cfg(windows)]
            if second == b'5' && third == b'c' {
                return Err(throw_coded(
                    ctx,
                    CodedError::Type,
                    "ERR_INVALID_FILE_URL_PATH",
                    "File URL path must not include encoded \\ or / characters",
                ));
            }
        }
        i += 1;
    }
    Ok(())
}

pub fn url_format<'js>(url: Class<'js, URL<'js>>, options: Opt<Value<'js>>) -> Result<String> {
    let url = url.borrow();
    let mut string = url.protocol();
    string.push_str("//");

    let mut include_fragment = true;
    let mut unicode_encode = false;
    let mut include_auth = true;
    let mut include_search = true;

    // Parse options if provided
    if let Some(options) = options.into_inner() {
        if let Some(options) = options.as_object() {
            if let Ok(value) = options.get("unicode") {
                unicode_encode = value;
            }
            if let Ok(value) = options.get("auth") {
                include_auth = value;
            }
            if let Ok(value) = options.get("fragment") {
                include_fragment = value;
            }
            if let Ok(value) = options.get("search") {
                include_search = value
            }
        }
    }

    if include_auth {
        let username = url.username();
        let password = url.password();
        if !username.is_empty() {
            string.push_str(&username);
            if !password.is_empty() {
                string.push(':');
                string.push_str(&password);
            }
            string.push('@');
        }
    }

    if unicode_encode {
        string.push_str(&domain_to_unicode(&url.host()));
    } else {
        string.push_str(&url.host());
    }

    string.push_str(&url.pathname());

    if include_search {
        string.push_str(&url.search());
    }

    if include_fragment {
        string.push_str(&url.hash());
    }

    Ok(string)
}

// https://url.spec.whatwg.org/#cannot-be-a-base-url-path-state
pub fn convert_trailing_space(url: &mut Url) {
    if matches!(
        url.scheme(),
        "file" | "ftp" | "http" | "https" | "ws" | "wss"
    ) {
        return;
    }

    let path = url.path();
    let has_remaining = url.fragment().is_some() || url.query().is_some();

    #[allow(clippy::manual_strip)]
    if path.ends_with(' ') && has_remaining {
        let new_path = [&path[..path.len() - 1], "%20"].concat();
        url.set_path(&new_path);
    }
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();

    Class::<URLSearchParams>::define(&globals)?;
    Class::<URL>::define(&globals)?;

    let install: Function = ctx.eval(include_str!("object_url.js"))?;
    install.call::<_, ()>((globals.get::<_, Value>(stringify!(URL))?,))?;

    Ok(())
}

const LEGACY_EXPORTS: [&str; 6] = [
    "Url",
    "parse",
    "resolve",
    "resolveObject",
    "format",
    "fileURLToPathBuffer",
];

pub struct UrlModule;

impl ModuleDef for UrlModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare(stringify!(URL))?;
        declare.declare(stringify!(URLSearchParams))?;
        declare.declare("urlToHttpOptions")?;
        declare.declare("domainToUnicode")?;
        declare.declare("domainToASCII")?;
        declare.declare("fileURLToPath")?;
        declare.declare("pathToFileURL")?;
        for name in LEGACY_EXPORTS {
            declare.declare(name)?;
        }
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let globals = ctx.globals();
        BasePrimordials::init(ctx)?;
        let url: Constructor = globals.get(stringify!(URL))?;
        let url_search_params: Constructor = globals.get(stringify!(URLSearchParams))?;

        export_default(ctx, exports, |default| {
            default.set(stringify!(URL), url)?;
            default.set(stringify!(URLSearchParams), url_search_params)?;
            default.set("urlToHttpOptions", Func::from(url_to_http_options))?;
            default.set(
                "domainToUnicode",
                Func::from(|domain: String| domain_to_unicode(&domain)),
            )?;
            default.set(
                "domainToASCII",
                Func::from(|domain: String| domain_to_ascii(&domain)),
            )?;
            default.set("fileURLToPath", Func::from(file_url_to_path))?;
            default.set("pathToFileURL", Func::from(path_to_file_url))?;
            install_legacy(ctx, default)?;
            Ok(())
        })?;

        Ok(())
    }
}

fn install_legacy<'js>(ctx: &Ctx<'js>, target: &Object<'js>) -> Result<()> {
    let natives = Object::new(ctx.clone())?;
    natives.set(
        "domainToASCII",
        Func::from(|domain: String| domain_to_ascii(&domain)),
    )?;
    natives.set(
        "domainToUnicode",
        Func::from(|domain: String| domain_to_unicode(&domain)),
    )?;
    natives.set(
        "toASCII",
        Func::from(|domain: String| idna_to_ascii(&domain)),
    )?;
    let install: Function = ctx.eval(include_str!("legacy.js"))?;
    let legacy: Object = install.call((natives,))?;
    for name in LEGACY_EXPORTS {
        target.set(name, legacy.get::<_, Value>(name)?)?;
    }
    Ok(())
}

impl From<UrlModule> for ModuleInfo<UrlModule> {
    fn from(val: UrlModule) -> Self {
        ModuleInfo {
            name: "url",
            module: val,
        }
    }
}
