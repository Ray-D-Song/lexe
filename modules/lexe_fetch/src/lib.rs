// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
pub use self::security::{get_allow_list, get_deny_list, set_allow_list, set_deny_list};
use self::{form_data::FormData, headers::Headers, request::Request, response::Response};
use lexe_buffer::Blob;
use lexe_http::FETCH_HTTP_CLIENT;
use lexe_utils::{
    class::CustomInspectExtension,
    primordials::{BasePrimordials, Primordial},
    result::ResultExt,
};
use rquickjs::{prelude::Func, Class, Ctx, Result, Value};

pub(crate) mod body_helpers;
mod cache;
pub mod fetch;
pub mod form_data;
pub mod headers;
pub mod integrity;
pub mod request;
pub mod response;
mod security;
pub mod utils;

const MIME_TYPE_FORM_URLENCODED: &str = "application/x-www-form-urlencoded;charset=UTF-8";
const MIME_TYPE_TEXT: &str = "text/plain;charset=UTF-8";
const MIME_TYPE_JSON: &str = "application/json;charset=UTF-8";
const MIME_TYPE_FORM_DATA: &str = "multipart/form-data; boundary=";
const MIME_TYPE_OCTET_STREAM: &str = "application/octet-stream";

/// An internal, immutable capability used by consumers that need to verify a
/// native `Response` brand without trusting the mutable JS `Response`
/// constructor or prototype chain.
pub const RESPONSE_BRAND_CHECK_KEY: &str = "\0lexe:has_native_response_brand";

fn has_native_response_brand<'js>(value: Value<'js>) -> bool {
    Class::<Response>::from_value(&value).is_ok()
}

pub fn init(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();

    BasePrimordials::init(ctx)?;

    //init eagerly
    fetch::init(FETCH_HTTP_CLIENT.as_ref().or_throw(ctx)?.clone(), &globals)?;
    cache::init_cache_api(ctx, &globals)?;

    Class::<FormData>::define(&globals)?;

    Class::<Request>::define(&globals)?;
    Class::<Response>::define(&globals)?;
    Class::<Headers>::define_with_custom_inspect(&globals)?;
    globals.prop(
        RESPONSE_BRAND_CHECK_KEY,
        Func::from(has_native_response_brand),
    )?;

    Ok(())
}
