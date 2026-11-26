// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use rquickjs::{
    atom::PredefinedAtom, object::Property, Array, Ctx, Function, Object, Result, Value,
};

fn get_user_agent() -> &'static str {
    concat!("lexe ", env!("CARGO_PKG_VERSION"))
}

fn get_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "MacIntel"
    }
    #[cfg(target_os = "windows")]
    {
        "Win32"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "Linux x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "Linux aarch64"
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64")
    )))]
    {
        std::env::consts::OS
    }
}

fn get_language() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(raw) = std::env::var(var) {
            let trimmed = raw.trim();
            if trimmed.is_empty()
                || trimmed.eq_ignore_ascii_case("C")
                || trimmed.eq_ignore_ascii_case("POSIX")
                || trimmed.starts_with("C.")
            {
                continue;
            }
            let base = trimmed
                .split(['.', '@'])
                .next()
                .unwrap_or("")
                .replace('_', "-");
            if !base.is_empty() {
                return base;
            }
        }
    }
    "en-US".to_string()
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    let globals = ctx.globals();

    let navigator = Object::new(ctx.clone())?;
    let language = get_language();
    let concurrency = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    let languages = Array::new(ctx.clone())?;
    languages.set(0, language.as_str())?;
    let freeze: Function = ctx.eval("Object.freeze")?;
    let frozen_languages: Value = freeze.call((languages,))?;

    navigator.set("userAgent", get_user_agent())?;
    navigator.set("hardwareConcurrency", concurrency)?;
    navigator.set("language", language)?;
    navigator.set("languages", frozen_languages)?;
    navigator.set("platform", get_platform())?;
    navigator.prop(
        PredefinedAtom::SymbolToStringTag,
        Property::from("Navigator").configurable(),
    )?;

    globals.set("navigator", navigator)?;

    Ok(())
}
