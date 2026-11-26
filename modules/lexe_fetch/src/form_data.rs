// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    io::Write,
    sync::{Arc, Mutex},
};

use lexe_buffer::{Blob, File};
use lexe_utils::{class::IteratorDef, object::map_to_entries, result::ResultExt};
use rand::RngExt;
use rquickjs::{
    atom::PredefinedAtom, class::Trace, prelude::Opt, qjs, Array, Class, Ctx, Exception, Function,
    IntoJs, JsLifetime, Result, Value,
};

#[derive(Clone)]
enum FormValue {
    Text(String),
    File(File),
}

impl<'js> IntoJs<'js> for FormValue {
    fn into_js(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        match self {
            FormValue::Text(s) => s.into_js(ctx),
            FormValue::File(f) => f.clone().into_js(ctx),
        }
    }
}

#[derive(Clone, Trace, JsLifetime, Default)]
#[rquickjs::class]
pub struct FormData {
    #[qjs(skip_trace)]
    entries: Arc<Mutex<Vec<(String, FormValue)>>>,
}

impl<'js> IteratorDef<'js> for FormData {
    fn js_entries(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        let entries = self.entries.lock().or_throw(&ctx)?;
        map_to_entries(&ctx, entries.clone())
    }
}

fn value_to_usv_string<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<String> {
    let raw = unsafe { qjs::JS_ToString(ctx.as_raw().as_ptr(), value.as_raw()) };
    let js_val = unsafe { Value::from_raw(ctx.clone(), raw) };
    if js_val.is_exception() {
        return Err(rquickjs::Error::Exception);
    }
    let js_str = js_val
        .into_string()
        .ok_or_else(|| Exception::throw_type(ctx, "Expected string"))?;
    let mut len = std::mem::MaybeUninit::uninit();
    let ptr = unsafe {
        qjs::JS_ToCStringLen2(
            ctx.as_raw().as_ptr(),
            len.as_mut_ptr(),
            js_str.as_raw(),
            true,
        )
    };
    if ptr.is_null() {
        return Err(rquickjs::Error::Exception);
    }
    let len = unsafe { len.assume_init() } as usize;
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    let units = cesu8_to_utf16(bytes);
    unsafe { qjs::JS_FreeCString(ctx.as_raw().as_ptr(), ptr) };
    Ok(char::decode_utf16(units)
        .map(|r| r.unwrap_or('\u{FFFD}'))
        .collect())
}

fn cesu8_to_utf16(bytes: &[u8]) -> Vec<u16> {
    if bytes.is_ascii() {
        return bytes.iter().map(|&b| b as u16).collect();
    }
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let get = |n: usize| bytes.get(i + n).map_or(0, |b| (b & 0x3f) as u32);
        match b0 {
            0x00..=0x7f => {
                units.push(b0 as u16);
                i += 1;
            },
            0xc0..=0xdf => {
                units.push((((b0 & 0x1f) as u32) << 6 | get(1)) as u16);
                i += 2;
            },
            0xe0..=0xef => {
                units.push((((b0 & 0x0f) as u32) << 12 | get(1) << 6 | get(2)) as u16);
                i += 3;
            },
            _ => {
                let cp = ((b0 & 0x07) as u32) << 18 | get(1) << 12 | get(2) << 6 | get(3);
                let cp = cp - 0x10000;
                units.push(0xd800 | (cp >> 10) as u16);
                units.push(0xdc00 | (cp & 0x3ff) as u16);
                i += 4;
            },
        }
    }
    units
}

fn build_form_value<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    filename: Opt<Value<'js>>,
    op_name: &str,
) -> Result<FormValue> {
    if let Ok(f) = Class::<File>::from_value(&value) {
        let file = f.borrow().to_owned();
        if let Some(fname_val) = filename.0 {
            if !fname_val.is_undefined() {
                let fname = value_to_usv_string(ctx, &fname_val)?;
                return Ok(FormValue::File(File::from_blob(file.get_blob(), fname)));
            }
        }
        return Ok(FormValue::File(file));
    }
    if let Ok(b) = Class::<Blob>::from_value(&value) {
        let blob = b.borrow().to_owned();
        let fname = match filename.0 {
            Some(v) if !v.is_undefined() => value_to_usv_string(ctx, &v)?,
            _ => "blob".to_string(),
        };
        return Ok(FormValue::File(File::from_blob(blob, fname)));
    }
    if let Some(fname_val) = filename.0 {
        if !fname_val.is_undefined() {
            return Err(Exception::throw_type(
                ctx,
                &format!("Failed to execute '{op_name}' on 'FormData': parameter 2 is not of type 'Blob'."),
            ));
        }
    }
    let text = value_to_usv_string(ctx, &value)?;
    Ok(FormValue::Text(text))
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> FormData {
    #[qjs(constructor)]
    pub fn new(_form: Opt<Value<'js>>, _submitter: Opt<Value<'js>>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn append(
        &self,
        ctx: Ctx<'js>,
        name: Opt<Value<'js>>,
        value: Opt<Value<'js>>,
        filename: Opt<Value<'js>>,
    ) -> Result<()> {
        let (Some(name_val), Some(val)) = (name.0, value.0) else {
            return Err(Exception::throw_type(
                &ctx,
                "Failed to execute 'append' on 'FormData': 2 arguments required.",
            ));
        };
        let name = value_to_usv_string(&ctx, &name_val)?;
        let entry = build_form_value(&ctx, val, filename, "append")?;
        let mut entries = self.entries.lock().or_throw(&ctx)?;
        entries.push((name, entry));
        Ok(())
    }

    pub fn get(&self, ctx: Ctx<'js>, name: String) -> Result<Option<Value<'js>>> {
        let entries = self.entries.lock().or_throw(&ctx)?;
        for (k, v) in entries.iter() {
            if *k == name {
                return Ok(v.clone().into_js(&ctx).ok());
            }
        }
        Ok(None)
    }

    pub fn get_all(&self, ctx: Ctx<'js>, name: String) -> Result<Vec<Value<'js>>> {
        let entries = self.entries.lock().or_throw(&ctx)?;

        Ok(entries
            .iter()
            .filter(|(k, _)| *k == name)
            .filter_map(|(_, v)| v.clone().into_js(&ctx).ok())
            .collect())
    }

    pub fn has(&self, ctx: Ctx<'js>, name: String) -> Result<bool> {
        let entries = self.entries.lock().or_throw(&ctx)?;

        Ok(entries.iter().any(|(n, _)| n == &name))
    }

    pub fn set(
        &self,
        ctx: Ctx<'js>,
        name: Opt<Value<'js>>,
        value: Opt<Value<'js>>,
        filename: Opt<Value<'js>>,
    ) -> Result<()> {
        let (Some(name_val), Some(val)) = (name.0, value.0) else {
            return Err(Exception::throw_type(
                &ctx,
                "Failed to execute 'set' on 'FormData': 2 arguments required.",
            ));
        };
        let name = value_to_usv_string(&ctx, &name_val)?;
        let entry = build_form_value(&ctx, val, filename, "set")?;
        let mut entries = self.entries.lock().or_throw(&ctx)?;
        if let Some(pos) = entries.iter().position(|(k, _)| *k == name) {
            entries[pos] = (name.clone(), entry);
            let mut idx = 0;
            entries.retain(|(k, _)| {
                let keep = idx == pos || k != &name;
                idx += 1;
                keep
            });
        } else {
            entries.push((name, entry));
        }
        Ok(())
    }

    pub fn delete(&self, ctx: Ctx<'js>, name: String) -> Result<()> {
        let mut entries = self.entries.lock().or_throw(&ctx)?;

        entries.retain(|(k, _)| *k != name);
        Ok(())
    }

    pub fn keys(&self, ctx: Ctx<'js>) -> Result<Vec<String>> {
        let entries = self.entries.lock().or_throw(&ctx)?;

        Ok(entries.iter().map(|(k, _)| k.clone()).collect())
    }

    pub fn values(&self, ctx: Ctx<'js>) -> Result<Vec<Value<'js>>> {
        let ctx2 = ctx.clone();
        let entries = self.entries.lock().or_throw(&ctx)?;

        Ok(entries
            .iter()
            .filter_map(|(_, v)| v.clone().into_js(&ctx2).ok())
            .collect())
    }

    pub fn entries(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.js_iterator(ctx)
    }

    #[qjs(rename = PredefinedAtom::SymbolIterator)]
    pub fn iterator(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.js_iterator(ctx)
    }

    pub fn for_each(&self, ctx: Ctx<'js>, callback: Function<'js>) -> Result<()> {
        let entries = self.entries.lock().or_throw(&ctx)?;

        for (name, value) in entries.iter() {
            let val = value.clone().into_js(&ctx)?;
            () = callback.call((val, name.clone()))?;
        }

        Ok(())
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(FormData)
    }
}

fn escape_multipart_field(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\r' => out.push_str("%0D"),
            '\n' => out.push_str("%0A"),
            '"' => out.push_str("%22"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape_multipart_field(input: &str) -> String {
    input
        .replace("%22", "\"")
        .replace("%0D", "\r")
        .replace("%0A", "\n")
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

impl FormData {
    #[allow(private_interfaces)]
    pub fn iter<'js>(&self, ctx: &Ctx<'js>) -> Result<impl Iterator<Item = (String, FormValue)>> {
        let entries = self.entries.lock().or_throw(ctx)?;
        let entries = entries.clone();

        Ok(entries.into_iter())
    }

    pub fn from_multipart_bytes<'js>(
        ctx: &Ctx<'js>,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<Self> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }

        let Some(boundary) = extract_boundary(content_type) else {
            return Ok(Self::default());
        };
        let delim = ["--", &boundary].concat().into_bytes();

        let mut entries = Vec::new();
        let Some(first) = find_subslice(&bytes, &delim) else {
            return Ok(Self::default());
        };
        let mut pos = first + delim.len();

        while pos < bytes.len() {
            // End delimiter `--boundary--`
            if bytes[pos..].starts_with(b"--") {
                break;
            }
            if bytes[pos..].starts_with(b"\r\n") {
                pos += 2;
            } else if bytes[pos..].starts_with(b"\n") {
                pos += 1;
            }

            let (header_end, body_start_offset) =
                if let Some(i) = find_subslice(&bytes[pos..], b"\r\n\r\n") {
                    (pos + i, 4)
                } else if let Some(i) = find_subslice(&bytes[pos..], b"\n\n") {
                    (pos + i, 2)
                } else {
                    break;
                };

            let header_bytes = &bytes[pos..header_end];
            let header_str = String::from_utf8_lossy(header_bytes);
            let mut name: Option<String> = None;
            let mut filename: Option<String> = None;
            let mut mime_type: Option<String> = None;

            for line in header_str.lines() {
                let line = line.trim_end_matches('\r');
                if line.to_ascii_lowercase().starts_with("content-disposition") {
                    for seg in line.split(';') {
                        let seg = seg.trim();
                        if let Some(n) = seg.strip_prefix("name=") {
                            name = Some(unescape_multipart_field(n.trim_matches('"')));
                        } else if let Some(f) = seg.strip_prefix("filename=") {
                            filename = Some(unescape_multipart_field(f.trim_matches('"')));
                        }
                    }
                } else if line.to_ascii_lowercase().starts_with("content-type:") {
                    if let Some(ct) = line.split_once(':').map(|(_, v)| v.trim()) {
                        mime_type = Some(ct.to_string());
                    }
                }
            }

            let body_start = header_end + body_start_offset;
            let next_delim = find_subslice(&bytes[body_start..], &delim);
            let body_end = match next_delim {
                Some(rel) => {
                    let raw_end = body_start + rel;
                    if bytes[..raw_end].ends_with(b"\r\n") {
                        raw_end - 2
                    } else if bytes[..raw_end].ends_with(b"\n") {
                        raw_end - 1
                    } else {
                        raw_end
                    }
                },
                None => bytes.len(),
            };

            if let Some(entry_name) = name {
                let payload = bytes[body_start..body_end].to_vec();
                if let Some(fname) = filename {
                    let file = File::from_bytes(ctx, payload, fname, mime_type)?;
                    entries.push((entry_name, FormValue::File(file)));
                } else {
                    let text = String::from_utf8_lossy(&payload).into_owned();
                    entries.push((entry_name, FormValue::Text(text)));
                }
            }

            match next_delim {
                Some(rel) => pos = body_start + rel + delim.len(),
                None => break,
            }
        }

        Ok(Self {
            entries: Arc::new(Mutex::new(entries)),
        })
    }

    pub fn to_multipart_bytes<'js>(&self, ctx: &Ctx<'js>) -> Result<(Vec<u8>, String)> {
        let boundary = generate_boundary();
        let mut body = Vec::new();
        let entries = self.entries.lock().or_throw(ctx)?;

        for (name, value) in entries.iter() {
            let esc_name = escape_multipart_field(name);
            match value {
                FormValue::Text(text) => {
                    write!(
                        body,
                        "--{boundary}\r\nContent-Disposition: form-data; name=\"{esc_name}\"\r\n\r\n{text}\r\n"
                    )?;
                },
                FormValue::File(file) => {
                    let esc_filename = escape_multipart_field(&file.name());
                    let raw_ct = file.mime_type();
                    let content_type = if raw_ct.is_empty() {
                        "application/octet-stream"
                    } else {
                        raw_ct.as_str()
                    };
                    let bytes = file.get_blob().get_bytes();
                    write!(
                        body,
                        "--{boundary}\r\nContent-Disposition: form-data; name=\"{esc_name}\"; filename=\"{esc_filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
                    )?;
                    body.extend_from_slice(&bytes);
                    body.extend_from_slice(b"\r\n");
                },
            }
        }

        write!(body, "--{boundary}--\r\n")?;

        Ok((body, boundary))
    }
}

fn extract_boundary(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|part| {
        let part = part.trim();
        part.find("boundary=").map(|idx| {
            part[(idx + "boundary=".len())..]
                .trim()
                .trim_matches('"')
                .into()
        })
    })
}

fn generate_boundary() -> String {
    let rand_string: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(24)
        .map(char::from)
        .collect();

    ["----WebKitFormBoundary", &rand_string].concat()
}
