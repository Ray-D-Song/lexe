// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use lexe_encoding::NodeEncoding;
use lexe_utils::bytes::ObjectBytes;
use rquickjs::{Array, Ctx, Error, Exception, Function, Object, Result, String as JsString, Value};

#[derive(Clone, Debug)]
pub(crate) struct HeaderEntry {
    pub(crate) lower: String,
    pub(crate) original: String,
    pub(crate) values: Vec<String>,
    pub(crate) is_array: bool,
}

#[derive(Clone, Default, Debug)]
pub(crate) struct HeaderStore {
    pub(crate) entries: Vec<HeaderEntry>,
}

impl HeaderStore {
    pub(crate) fn set(&mut self, name: &str, values: Vec<String>, is_array: bool) {
        let lower = name.to_ascii_lowercase();
        let original = name.to_string();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.lower == lower) {
            entry.original = original;
            entry.values = values;
            entry.is_array = is_array;
        } else {
            self.entries.push(HeaderEntry {
                lower,
                original,
                values,
                is_array,
            });
        }
    }

    pub(crate) fn append(&mut self, name: &str, mut values: Vec<String>, is_array: bool) {
        let lower = name.to_ascii_lowercase();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.lower == lower) {
            entry.original = name.to_string();
            entry.values.append(&mut values);
            entry.is_array = true;
        } else {
            let entry_is_array = is_array || values.len() > 1;
            self.entries.push(HeaderEntry {
                lower,
                original: name.to_string(),
                values,
                is_array: entry_is_array,
            });
        }
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        self.entries.iter().any(|e| e.lower == lower)
    }

    pub(crate) fn remove(&mut self, name: &str) {
        let lower = name.to_ascii_lowercase();
        self.entries.retain(|e| e.lower != lower);
    }

    pub(crate) fn get_js<'js>(&self, ctx: &Ctx<'js>, name: &str) -> Result<Value<'js>> {
        let lower = name.to_ascii_lowercase();
        let Some(entry) = self.entries.iter().find(|e| e.lower == lower) else {
            return Ok(Value::new_undefined(ctx.clone()));
        };
        if entry.lower == "set-cookie" || entry.is_array || entry.values.len() > 1 {
            let arr = Array::new(ctx.clone())?;
            for (idx, v) in entry.values.iter().enumerate() {
                arr.set(idx, v.as_str())?;
            }
            Ok(arr.into_value())
        } else {
            let single = entry.values.first().map(String::as_str).unwrap_or("");
            Ok(JsString::from_str(ctx.clone(), single)?.into_value())
        }
    }

    pub(crate) fn get_header_names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.lower.clone()).collect()
    }

    pub(crate) fn get_raw_header_names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.original.clone()).collect()
    }

    pub(crate) fn get_headers_object<'js>(&self, ctx: &Ctx<'js>) -> Result<Object<'js>> {
        let create_null: Function = ctx.eval("(function () { return Object.create(null); })")?;
        let result: Object = create_null.call(())?;
        for entry in &self.entries {
            if entry.lower == "set-cookie" || entry.is_array || entry.values.len() > 1 {
                let arr = Array::new(ctx.clone())?;
                for (i, v) in entry.values.iter().enumerate() {
                    arr.set(i, v.as_str())?;
                }
                result.set(entry.lower.as_str(), arr)?;
            } else {
                let val = entry.values.first().map(String::as_str).unwrap_or("");
                result.set(entry.lower.as_str(), val)?;
            }
        }
        Ok(result)
    }

    pub(crate) fn get_content_length(&self) -> Option<usize> {
        let entry = self.entries.iter().find(|e| e.lower == "content-length")?;
        if entry.values.len() != 1 {
            return None;
        }
        entry.values[0].trim().parse::<usize>().ok()
    }

    pub(crate) fn set_from_js<'js>(&mut self, ctx: &Ctx<'js>, headers: Value<'js>) -> Result<()> {
        let is_valid: bool = ctx
            .eval::<Function, _>(
                "(function (h) { return h !== null && typeof h === 'object' && typeof h.entries === 'function' && (h instanceof Map || (typeof Headers === 'function' && h instanceof Headers) || (typeof Symbol !== 'undefined' && typeof h[Symbol.iterator] === 'function')); })",
            )?
            .call((headers.clone(),))?;
        if !is_valid {
            return Err(throw_coded(
                ctx,
                ErrorKind::Type,
                "ERR_INVALID_ARG_TYPE",
                "The \"headers\" argument must be an instance of Headers or Map",
            ));
        }

        let pairs: Vec<(String, Value<'js>)> = {
            let obj = headers
                .as_object()
                .ok_or_else(|| Exception::throw_type(ctx, "Invalid headers"))?;
            let entries_fn: Function = obj.get("entries")?;
            let iter: Object = entries_fn.call((rquickjs::function::This(obj.clone()),))?;
            let next_fn: Function = iter.get("next")?;
            let mut collected = Vec::new();
            loop {
                let step: Object = next_fn.call((rquickjs::function::This(iter.clone()),))?;
                let done: bool = step.get("done")?;
                if done {
                    break;
                }
                let pair: Array = step.get("value")?;
                let key_val: Value = pair.get(0)?;
                let key = js_to_string(ctx, key_val)?;
                let val: Value = pair.get(1)?;
                collected.push((key, val));
            }
            collected
        };

        for (name, value) in pairs {
            validate_header_name_str(ctx, &name)?;
            let (values, is_array) = header_value_to_list_with_kind(ctx, &name, value)?;
            self.set(&name, values, is_array);
        }
        Ok(())
    }

    pub(crate) fn iter_pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().flat_map(|entry| {
            entry
                .values
                .iter()
                .map(move |v| (entry.lower.as_str(), v.as_str()))
        })
    }

    pub(crate) fn iter_raw_pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().flat_map(|entry| {
            entry
                .values
                .iter()
                .map(move |v| (entry.original.as_str(), v.as_str()))
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ErrorKind {
    Error,
    Type,
}

pub(crate) fn throw_coded(ctx: &Ctx<'_>, kind: ErrorKind, code: &str, message: &str) -> Error {
    let err = match kind {
        ErrorKind::Error => Exception::from_message(ctx.clone(), message),
        ErrorKind::Type => {
            if let Ok(ctor) = ctx.globals().get::<_, Function>("TypeError") {
                match ctor.call::<_, Value>((message,)) {
                    Ok(val) => {
                        if let Some(obj) = val.as_object() {
                            let _ = obj.set("code", code);
                        }
                        return ctx.throw(val);
                    },
                    Err(_) => Exception::from_message(ctx.clone(), message),
                }
            } else {
                return Exception::throw_type(ctx, message);
            }
        },
    };
    match err {
        Ok(e) => {
            let val = e.into_value();
            if let Some(obj) = val.as_object() {
                let _ = obj.set("code", code);
            }
            ctx.throw(val)
        },
        Err(e) => e,
    }
}

pub(crate) fn throw_headers_sent(ctx: &Ctx<'_>, action: &str) -> Error {
    let msg = format!("Cannot {action} headers after they are sent to the client");
    throw_coded(ctx, ErrorKind::Error, "ERR_HTTP_HEADERS_SENT", &msg)
}

pub(crate) fn throw_content_length_mismatch(
    ctx: &Ctx<'_>,
    actual: usize,
    expected: usize,
) -> Error {
    let msg = format!(
        "Response body's content-length of {actual} byte(s) does not match the content-length of {expected} byte(s) set in header"
    );
    throw_coded(
        ctx,
        ErrorKind::Error,
        "ERR_HTTP_CONTENT_LENGTH_MISMATCH",
        &msg,
    )
}

pub(crate) fn is_valid_header_token(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

pub(crate) fn validate_header_name_str(ctx: &Ctx<'_>, name: &str) -> Result<()> {
    if !is_valid_header_token(name) {
        return Err(throw_coded(
            ctx,
            ErrorKind::Type,
            "ERR_INVALID_HTTP_TOKEN",
            &format!("Header name must be a valid HTTP token [\"{name}\"]"),
        ));
    }
    Ok(())
}

pub(crate) fn validate_header_value_str(ctx: &Ctx<'_>, name: &str, value: &str) -> Result<()> {
    let invalid = value.bytes().any(|b| b == 0x7f || (b < 0x20 && b != b'\t'));
    if invalid {
        return Err(throw_coded(
            ctx,
            ErrorKind::Type,
            "ERR_INVALID_CHAR",
            &format!("Invalid character in header content [\"{name}\"]"),
        ));
    }
    Ok(())
}

pub(crate) fn validate_header_name<'js>(ctx: Ctx<'js>, name: Value<'js>) -> Result<()> {
    let Some(s) = name.as_string() else {
        return Err(throw_coded(
            &ctx,
            ErrorKind::Type,
            "ERR_INVALID_HTTP_TOKEN",
            "Header name must be a valid HTTP token",
        ));
    };
    let name = s.to_string()?;
    validate_header_name_str(&ctx, &name)
}

pub(crate) fn validate_header_value<'js>(
    ctx: Ctx<'js>,
    name: String,
    value: Value<'js>,
) -> Result<()> {
    let _ = header_value_to_list_with_kind(&ctx, &name, value)?;
    Ok(())
}

fn js_to_string<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
    if let Some(s) = value.as_string() {
        return s.to_string();
    }
    if let Some(n) = value.as_number() {
        if n.fract() == 0.0 && n.abs() < 1e15 {
            return Ok(format!("{}", n as i64));
        }
        return Ok(n.to_string());
    }
    if let Some(b) = value.as_bool() {
        return Ok(if b { "true" } else { "false" }.to_string());
    }
    ctx.eval::<Function, _>("String")?.call((value,))
}

pub(crate) fn header_value_to_list_with_kind<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    value: Value<'js>,
) -> Result<(Vec<String>, bool)> {
    if value.is_undefined() {
        return Err(throw_coded(
            ctx,
            ErrorKind::Type,
            "ERR_HTTP_INVALID_HEADER_VALUE",
            &format!("Invalid value \"undefined\" for header \"{name}\""),
        ));
    }
    if let Some(arr) = value.as_array() {
        let mut parts = Vec::new();
        for item in arr.iter::<Value>() {
            let item = item?;
            if item.is_undefined() {
                return Err(throw_coded(
                    ctx,
                    ErrorKind::Type,
                    "ERR_HTTP_INVALID_HEADER_VALUE",
                    &format!("Invalid value \"undefined\" for header \"{name}\""),
                ));
            }
            let s = if item.is_null() {
                "null".to_string()
            } else {
                js_to_string(ctx, item)?
            };
            validate_header_value_str(ctx, name, &s)?;
            parts.push(s);
        }
        return Ok((parts, true));
    }
    let s = if value.is_null() {
        "null".to_string()
    } else {
        js_to_string(ctx, value)?
    };
    validate_header_value_str(ctx, name, &s)?;
    Ok((vec![s], false))
}

pub(crate) fn parse_write_or_end_args<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    allow_cb_first: bool,
) -> Result<(Option<Vec<u8>>, Option<Function<'js>>)> {
    if args.is_empty() {
        return Ok((None, None));
    }
    let first = &args[0];
    if allow_cb_first && first.is_function() {
        return Ok((None, first.as_function().cloned()));
    }
    if first.is_undefined() || first.is_null() {
        let cb = args
            .get(1)
            .and_then(|v| v.as_function().cloned())
            .or_else(|| args.get(2).and_then(|v| v.as_function().cloned()));
        return Ok((None, cb));
    }

    let mut encoding: Option<String> = None;
    let mut callback: Option<Function<'js>> = None;

    if let Some(second) = args.get(1) {
        if let Some(cb) = second.as_function() {
            callback = Some(cb.clone());
        } else if let Some(s) = second.as_string() {
            encoding = Some(s.to_string()?);
        }
    }
    if callback.is_none() {
        if let Some(third) = args.get(2) {
            if let Some(cb) = third.as_function() {
                callback = Some(cb.clone());
            }
        }
    }

    let bytes = if let Some(s) = first.as_string() {
        let text = s.to_string()?;
        if let Some(enc_label) = encoding.as_deref() {
            let Some(enc) = NodeEncoding::parse(enc_label) else {
                return Err(throw_coded(
                    ctx,
                    ErrorKind::Type,
                    "ERR_UNKNOWN_ENCODING",
                    &format!("Unknown encoding: {enc_label}"),
                ));
            };
            let units: Vec<u16> = text.encode_utf16().collect();
            enc.encode_units(&units)
        } else {
            text.into_bytes()
        }
    } else {
        ObjectBytes::from(ctx, first)?.into_bytes(ctx)?
    };

    Ok((Some(bytes), callback))
}
