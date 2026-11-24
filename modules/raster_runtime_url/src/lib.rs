// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::inherent_to_string)]
pub mod url_class;
pub mod url_search_params;

use raster_runtime_utils::{
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

// options are ignored
pub fn path_to_file_url<'js>(ctx: Ctx<'js>, path: String, _: Opt<Value>) -> Result<URL<'js>> {
    let url = Url::from_file_path(&path)
        .map_err(|_| Exception::throw_type(&ctx, &["Path is not absolute: ", &path].concat()))?;

    URL::from_url(ctx, url)
}

/// Convert a WHATWG `file:` URL into a platform filesystem path.
///
/// Handles percent-encoding and platform path conventions via [`Url::to_file_path`].
/// Rejects encoded path separators that Node's `fileURLToPath` also rejects.
pub fn file_path_from_url(ctx: &Ctx<'_>, url: &Url) -> Result<String> {
    if url.scheme() != "file" {
        return Err(Exception::throw_type(ctx, "The URL must be of scheme file"));
    }

    reject_encoded_path_separators(ctx, url)?;

    url.to_file_path()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|_| Exception::throw_type(ctx, "File URL path must be absolute"))
}

/// Node rejects percent-encoded `/` (`%2F`) on all platforms, and `%5C` (`\`) on Windows,
/// before decoding the path. See `getPathFromURLPosix` / `getPathFromURLWin32`.
fn reject_encoded_path_separators(ctx: &Ctx<'_>, url: &Url) -> Result<()> {
    let pathname = url.path();
    let bytes = pathname.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' {
            let second = bytes[i + 1];
            let third = bytes[i + 2] | 0x20;
            if second == b'2' && third == b'f' {
                #[cfg(windows)]
                {
                    return Err(Exception::throw_type(
                        ctx,
                        "File URL path must not include encoded \\ or / characters",
                    ));
                }
                #[cfg(not(windows))]
                {
                    return Err(Exception::throw_type(
                        ctx,
                        "File URL path must not include encoded / characters",
                    ));
                }
            }
            #[cfg(windows)]
            if second == b'5' && third == b'c' {
                return Err(Exception::throw_type(
                    ctx,
                    "File URL path must not include encoded \\ or / characters",
                ));
            }
        }
        i += 1;
    }
    Ok(())
}

// options are ignored
pub fn file_url_to_path<'js>(ctx: Ctx<'js>, url: Value<'js>) -> Result<String> {
    let parsed = if let Ok(url) = Class::<URL>::from_value(&url) {
        url.borrow().inner().clone()
    } else {
        let url_string = url.get::<Coerced<String>>()?.to_string();
        Url::parse(&url_string)
            .map_err(|err| Exception::throw_type(&ctx, &format!("Invalid URL: {err}")))?
    };

    file_path_from_url(&ctx, &parsed)
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
