// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::env;

use once_cell::sync::Lazy;

pub mod module;
pub mod module_builder;
pub mod package;

pub use self::modules::*;

mod modules {
    #[cfg(feature = "abort")]
    pub use lexe_abort as abort;
    #[cfg(feature = "assert")]
    pub use lexe_assert as assert;
    #[cfg(feature = "async-hooks")]
    pub use lexe_async_hooks as async_hooks;
    #[cfg(feature = "buffer")]
    pub use lexe_buffer as buffer;
    #[cfg(feature = "child-process")]
    pub use lexe_child_process as child_process;
    #[cfg(feature = "console")]
    pub use lexe_console as console;
    #[cfg(feature = "constants")]
    pub use lexe_constants as constants;
    #[cfg(feature = "crypto")]
    pub use lexe_crypto as crypto;
    #[cfg(feature = "dgram")]
    pub use lexe_dgram as dgram;
    #[cfg(feature = "diagnostics-channel")]
    pub use lexe_diagnostics_channel as diagnostics_channel;
    #[cfg(feature = "dns")]
    pub use lexe_dns as dns;
    #[cfg(feature = "events")]
    pub use lexe_events as events;
    #[cfg(feature = "exceptions")]
    pub use lexe_exceptions as exceptions;
    #[cfg(feature = "fetch")]
    pub use lexe_fetch as fetch;
    #[cfg(feature = "ffi")]
    pub use lexe_ffi as ffi;
    #[cfg(feature = "fs")]
    pub use lexe_fs as fs;
    #[cfg(feature = "http")]
    pub use lexe_http as http;
    #[cfg(feature = "https")]
    pub use lexe_http as https;
    #[cfg(feature = "http2")]
    pub use lexe_http2 as http2;
    #[cfg(feature = "inspector")]
    pub use lexe_inspector as inspector;
    #[cfg(feature = "intl")]
    pub use lexe_intl as intl;
    #[cfg(feature = "navigator")]
    pub use lexe_navigator as navigator;
    #[cfg(feature = "net")]
    pub use lexe_net as net;
    #[cfg(feature = "os")]
    pub use lexe_os as os;
    #[cfg(feature = "path")]
    pub use lexe_path as path;
    #[cfg(feature = "perf-hooks")]
    pub use lexe_perf_hooks as perf_hooks;
    #[cfg(feature = "process")]
    pub use lexe_process as process;
    #[cfg(feature = "querystring")]
    pub use lexe_querystring as querystring;
    #[cfg(feature = "sea")]
    pub use lexe_sea as sea;
    #[cfg(feature = "sqlite")]
    pub use lexe_sqlite as sqlite;
    #[cfg(feature = "stream-web")]
    pub use lexe_stream_web as stream_web;
    #[cfg(feature = "string-decoder")]
    pub use lexe_string_decoder as string_decoder;
    #[cfg(feature = "temporal")]
    pub use lexe_temporal as temporal;
    #[cfg(feature = "timers")]
    pub use lexe_timers as timers;
    #[cfg(feature = "tls")]
    pub use lexe_tls as tls;
    #[cfg(feature = "tty")]
    pub use lexe_tty as tty;
    #[cfg(feature = "url")]
    pub use lexe_url as url;
    #[cfg(feature = "util")]
    pub use lexe_util as util;
    #[cfg(feature = "v8")]
    pub use lexe_v8 as v8;
    #[cfg(feature = "vm")]
    pub use lexe_vm as vm;
    #[cfg(feature = "webassembly")]
    pub use lexe_webassembly as webassembly;
    #[cfg(feature = "worker-threads")]
    pub use lexe_worker_threads as worker_threads;
    #[cfg(feature = "zlib")]
    pub use lexe_zlib as zlib;
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// added when .cjs files are imported
pub const CJS_IMPORT_PREFIX: &str = "__cjs:";
// added to force CJS imports in loader
pub const CJS_LOADER_PREFIX: &str = "__cjsm:";

pub const ENV_LEXE_PLATFORM: &str = "LEXE_PLATFORM";

pub static LEXE_PLATFORM: Lazy<String> = Lazy::new(|| {
    env::var(ENV_LEXE_PLATFORM)
        .ok()
        .filter(|platform| platform == "node")
        .unwrap_or_else(|| "browser".to_string())
});
