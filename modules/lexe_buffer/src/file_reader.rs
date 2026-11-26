// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::sync::{Arc, RwLock};

use lexe_encoding::{bytes_to_b64_string, Encoder, IncrementalDecoder};
use lexe_events::{Emitter, EventEmitter, EventList, ProgressEvent};
use lexe_exceptions::{DOMException, DOMExceptionName};
use rquickjs::{
    atom::PredefinedAtom,
    class::{Trace, Tracer},
    function::Opt,
    object::Property,
    prelude::This,
    ArrayBuffer, Class, Ctx, Exception, Function, IntoJs, JsLifetime, Null, Object, Result, Value,
};

use super::blob::{usv_string_from_js, Blob};
use super::codec::units_to_js;
use super::file::File;

const EMPTY: u16 = 0;
const LOADING: u16 = 1;
const DONE: u16 = 2;

struct YieldOnce(bool);

impl std::future::Future for YieldOnce {
    type Output = ();
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        if self.0 {
            std::task::Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    }
}

#[derive(Clone, Copy)]
enum ReadKind {
    ArrayBuffer,
    BinaryString,
    DataUrl,
    Text,
}

#[derive(Clone, Copy)]
enum HandlerSlot {
    LoadStart,
    Progress,
    Load,
    Abort,
    Error,
    LoadEnd,
}

impl HandlerSlot {
    fn event_name(self) -> &'static str {
        match self {
            Self::LoadStart => "loadstart",
            Self::Progress => "progress",
            Self::Load => "load",
            Self::Abort => "abort",
            Self::Error => "error",
            Self::LoadEnd => "loadend",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::LoadStart => 0,
            Self::Progress => 1,
            Self::Load => 2,
            Self::Abort => 3,
            Self::Error => 4,
            Self::LoadEnd => 5,
        }
    }
}

fn extract_blob_data<'js>(
    ctx: &Ctx<'js>,
    blob_opt: Opt<Value<'js>>,
    op_name: &str,
    iface_name: &str,
) -> Result<(Vec<u8>, String)> {
    let Some(val) = blob_opt.0 else {
        return Err(Exception::throw_type(
            ctx,
            &format!("Failed to execute '{op_name}' on '{iface_name}': 1 argument required, but only 0 present."),
        ));
    };
    if let Ok(blob) = Class::<Blob>::from_value(&val) {
        let b = blob.borrow();
        return Ok((b.get_bytes(), b.mime_type()));
    }
    if let Ok(file) = Class::<File>::from_value(&val) {
        let f = file.borrow();
        return Ok((f.get_blob().get_bytes(), f.mime_type()));
    }
    Err(Exception::throw_type(
        ctx,
        &format!(
            "Failed to execute '{op_name}' on '{iface_name}': parameter 1 is not of type 'Blob'."
        ),
    ))
}

fn decode_text_bytes(bytes: &[u8], encoding_label: Option<&str>) -> String {
    // Encoding Standard §4.2 BOM sniff takes precedence over fallback encoding.
    let (encoder, slice) = if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        (Encoder::Utf8, rest)
    } else if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        (Encoder::Utf16be, rest)
    } else if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        (Encoder::Utf16le, rest)
    } else {
        let enc = encoding_label
            .and_then(|label| Encoder::from_str(label).ok())
            .filter(|e| !matches!(e, Encoder::Hex | Encoder::Base64))
            .unwrap_or(Encoder::Utf8);
        (enc, bytes)
    };

    let mut dec = IncrementalDecoder::new();
    dec.decode_chunk(&encoder, slice, true, false, true)
        .unwrap_or_else(|_| String::from_utf8_lossy(slice).into_owned())
}

fn format_data_url(bytes: &[u8], mime_type: &str) -> String {
    let media_type = if mime_type.is_empty() {
        "application/octet-stream"
    } else {
        mime_type
    };
    let b64 = bytes_to_b64_string(bytes);
    format!("data:{media_type};base64,{b64}")
}

fn produce_result<'js>(
    ctx: &Ctx<'js>,
    kind: ReadKind,
    bytes: &[u8],
    mime_type: &str,
    encoding: Option<&str>,
) -> Result<Value<'js>> {
    match kind {
        ReadKind::ArrayBuffer => {
            let ab = ArrayBuffer::new(ctx.clone(), bytes.to_vec())?;
            Ok(ab.into_value())
        },
        ReadKind::BinaryString => {
            let units: Vec<u16> = bytes.iter().map(|&b| b as u16).collect();
            units_to_js(ctx, &units)
        },
        ReadKind::DataUrl => format_data_url(bytes, mime_type).into_js(ctx),
        ReadKind::Text => decode_text_bytes(bytes, encoding).into_js(ctx),
    }
}

#[rquickjs::class]
#[derive(Clone)]
pub struct FileReader<'js> {
    emitter: EventEmitter<'js>,
    ready_state: u16,
    result: Option<Value<'js>>,
    error: Option<Value<'js>>,
    generation: u64,
    handlers: [Option<Function<'js>>; 6],
    wrappers: [Option<Function<'js>>; 6],
}

unsafe impl<'js> JsLifetime<'js> for FileReader<'js> {
    type Changed<'to> = FileReader<'to>;
}

impl<'js> Trace<'js> for FileReader<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.emitter.trace(tracer);
        if let Some(ref res) = self.result {
            tracer.mark(res);
        }
        if let Some(ref err) = self.error {
            tracer.mark(err);
        }
        for f in self.handlers.iter().flatten() {
            tracer.mark(f);
        }
        for f in self.wrappers.iter().flatten() {
            tracer.mark(f);
        }
    }
}

impl<'js> Emitter<'js> for FileReader<'js> {
    fn get_event_list(&self) -> Arc<RwLock<EventList<'js>>> {
        self.emitter.get_event_list()
    }
}

impl<'js> FileReader<'js> {
    fn get_handler(&self, ctx: &Ctx<'js>, slot: HandlerSlot) -> Result<Value<'js>> {
        match &self.handlers[slot.index()] {
            Some(f) => Ok(f.clone().into_value()),
            None => Null.into_js(ctx),
        }
    }

    fn set_handler(
        this: This<Class<'js, Self>>,
        ctx: &Ctx<'js>,
        slot: HandlerSlot,
        val: Value<'js>,
    ) -> Result<()> {
        let idx = slot.index();
        let new_fn = val.into_function();

        if let Some(func) = new_fn {
            let need_register = {
                let mut borrow = this.0.borrow_mut();
                borrow.handlers[idx] = Some(func);
                borrow.wrappers[idx].is_none()
            };
            if need_register {
                let wrapper = Function::new(
                    ctx.clone(),
                    move |_ctx: Ctx<'js>,
                          ev_this: This<Value<'js>>,
                          ev: Value<'js>|
                          -> Result<()> {
                        if let Ok(reader) = Class::<FileReader>::from_value(&ev_this.0) {
                            let handler = reader.borrow().handlers[idx].clone();
                            if let Some(h) = handler {
                                let _: Value = h.call((This(ev_this.0), ev))?;
                            }
                        }
                        Ok(())
                    },
                )?;
                this.0.borrow_mut().wrappers[idx] = Some(wrapper.clone());
                Self::add_event_listener_str(
                    this.0.clone(),
                    ctx,
                    slot.event_name(),
                    wrapper,
                    false,
                    false,
                )?;
            }
        } else {
            let old_wrapper = {
                let mut borrow = this.0.borrow_mut();
                borrow.handlers[idx] = None;
                borrow.wrappers[idx].take()
            };
            if let Some(wrapper) = old_wrapper {
                Self::remove_event_listener_str(this.0.clone(), ctx, slot.event_name(), wrapper)?;
            }
        }
        Ok(())
    }

    fn dispatch_progress_event(
        ctx: &Ctx<'js>,
        this: &Class<'js, Self>,
        event_type: &str,
        length_computable: bool,
        loaded: f64,
        total: f64,
    ) -> Result<()> {
        let ev = Class::instance(
            ctx.clone(),
            ProgressEvent::with_values(event_type, length_computable, loaded, total),
        )?;
        if let Some(this_obj) = this.as_object() {
            let dispatch: Function = this_obj.get("dispatchEvent")?;
            let _: Value = dispatch.call((This(this.clone().into_value()), ev))?;
        }
        Ok(())
    }

    fn start_read(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        blob: Opt<Value<'js>>,
        encoding: Opt<Value<'js>>,
        kind: ReadKind,
        op_name: &str,
    ) -> Result<()> {
        let (bytes, mime_type) = extract_blob_data(&ctx, blob, op_name, "FileReader")?;
        let encoding_str = match (kind, encoding.0) {
            (ReadKind::Text, Some(v)) if !v.is_undefined() => Some(usv_string_from_js(&ctx, &v)?),
            _ => None,
        };

        let gen = {
            let mut reader = this.0.borrow_mut();
            if reader.ready_state == LOADING {
                let dom_err = DOMException::new_with_name(
                    &ctx,
                    DOMExceptionName::InvalidStateError,
                    format!("Failed to execute '{op_name}' on 'FileReader': The object is already in an InvalidStateError state."),
                )?;
                return Err(ctx.throw(dom_err.into_js(&ctx)?));
            }
            reader.ready_state = LOADING;
            reader.result = None;
            reader.error = None;
            reader.generation = reader.generation.wrapping_add(1);
            reader.generation
        };

        let this_class = this.0;
        let spawn_ctx = ctx.clone();
        ctx.spawn(async move {
            YieldOnce(false).await;
            if this_class.borrow().generation != gen {
                return;
            }
            let total = bytes.len() as f64;
            if Self::dispatch_progress_event(&spawn_ctx, &this_class, "loadstart", true, 0.0, total)
                .is_err()
            {
                return;
            }
            if this_class.borrow().generation != gen {
                return;
            }

            YieldOnce(false).await;
            if this_class.borrow().generation != gen {
                return;
            }
            if Self::dispatch_progress_event(
                &spawn_ctx,
                &this_class,
                "progress",
                true,
                total,
                total,
            )
            .is_err()
            {
                return;
            }
            if this_class.borrow().generation != gen {
                return;
            }

            match produce_result(
                &spawn_ctx,
                kind,
                &bytes,
                &mime_type,
                encoding_str.as_deref(),
            ) {
                Ok(val) => {
                    {
                        let mut reader = this_class.borrow_mut();
                        if reader.generation != gen {
                            return;
                        }
                        reader.ready_state = DONE;
                        reader.result = Some(val);
                    }
                    let _ = Self::dispatch_progress_event(
                        &spawn_ctx,
                        &this_class,
                        "load",
                        true,
                        total,
                        total,
                    );
                },
                Err(err) => {
                    let dom_err = DOMException::new_with_name(
                        &spawn_ctx,
                        DOMExceptionName::NotReadableError,
                        err.to_string(),
                    )
                    .and_then(|e| e.into_js(&spawn_ctx));
                    {
                        let mut reader = this_class.borrow_mut();
                        if reader.generation != gen {
                            return;
                        }
                        reader.ready_state = DONE;
                        reader.result = None;
                        reader.error = dom_err.ok();
                    }
                    let _ = Self::dispatch_progress_event(
                        &spawn_ctx,
                        &this_class,
                        "error",
                        true,
                        0.0,
                        total,
                    );
                },
            }

            if this_class.borrow().generation == gen {
                let _ = Self::dispatch_progress_event(
                    &spawn_ctx,
                    &this_class,
                    "loadend",
                    true,
                    total,
                    total,
                );
            }
        });

        Ok(())
    }

    pub fn install_prototype(ctx: &Ctx<'js>, globals: &Object<'js>) -> Result<()> {
        Self::add_event_target_prototype(ctx)?;

        if let Some(proto) = Class::<Self>::prototype(ctx)? {
            for (name, val) in [("EMPTY", EMPTY), ("LOADING", LOADING), ("DONE", DONE)] {
                proto.prop(name, Property::from(val).enumerable())?;
            }
            if let Some(et_proto) = globals
                .get::<_, Option<Object>>("EventTarget")?
                .and_then(|ctor| ctor.get::<_, Option<Object>>("prototype").ok().flatten())
            {
                let _ = proto.set_prototype(Some(&et_proto));
            }
        }
        if let Some(ctor) = globals.get::<_, Option<Object>>("FileReader")? {
            for (name, val) in [("EMPTY", EMPTY), ("LOADING", LOADING), ("DONE", DONE)] {
                ctor.prop(name, Property::from(val).enumerable())?;
            }
            if let Some(et_ctor) = globals.get::<_, Option<Object>>("EventTarget")? {
                let _ = ctor.set_prototype(Some(&et_ctor));
            }
        }

        Ok(())
    }
}

impl<'js> Default for FileReader<'js> {
    fn default() -> Self {
        Self::new()
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> FileReader<'js> {
    #[qjs(constructor)]
    pub fn new() -> Self {
        Self {
            emitter: EventEmitter::new(),
            ready_state: EMPTY,
            result: None,
            error: None,
            generation: 0,
            handlers: [None, None, None, None, None, None],
            wrappers: [None, None, None, None, None, None],
        }
    }

    #[qjs(get)]
    pub fn ready_state(&self) -> u16 {
        self.ready_state
    }

    #[qjs(get)]
    pub fn result(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match &self.result {
            Some(v) => Ok(v.clone()),
            None => Null.into_js(&ctx),
        }
    }

    #[qjs(get)]
    pub fn error(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        match &self.error {
            Some(v) => Ok(v.clone()),
            None => Null.into_js(&ctx),
        }
    }

    #[qjs(get, rename = "onloadstart")]
    pub fn get_onloadstart(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.get_handler(&ctx, HandlerSlot::LoadStart)
    }

    #[qjs(set, rename = "onloadstart")]
    pub fn set_onloadstart(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        val: Value<'js>,
    ) -> Result<()> {
        Self::set_handler(this, &ctx, HandlerSlot::LoadStart, val)
    }

    #[qjs(get, rename = "onprogress")]
    pub fn get_onprogress(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.get_handler(&ctx, HandlerSlot::Progress)
    }

    #[qjs(set, rename = "onprogress")]
    pub fn set_onprogress(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        val: Value<'js>,
    ) -> Result<()> {
        Self::set_handler(this, &ctx, HandlerSlot::Progress, val)
    }

    #[qjs(get, rename = "onload")]
    pub fn get_onload(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.get_handler(&ctx, HandlerSlot::Load)
    }

    #[qjs(set, rename = "onload")]
    pub fn set_onload(this: This<Class<'js, Self>>, ctx: Ctx<'js>, val: Value<'js>) -> Result<()> {
        Self::set_handler(this, &ctx, HandlerSlot::Load, val)
    }

    #[qjs(get, rename = "onabort")]
    pub fn get_onabort(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.get_handler(&ctx, HandlerSlot::Abort)
    }

    #[qjs(set, rename = "onabort")]
    pub fn set_onabort(this: This<Class<'js, Self>>, ctx: Ctx<'js>, val: Value<'js>) -> Result<()> {
        Self::set_handler(this, &ctx, HandlerSlot::Abort, val)
    }

    #[qjs(get, rename = "onerror")]
    pub fn get_onerror(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.get_handler(&ctx, HandlerSlot::Error)
    }

    #[qjs(set, rename = "onerror")]
    pub fn set_onerror(this: This<Class<'js, Self>>, ctx: Ctx<'js>, val: Value<'js>) -> Result<()> {
        Self::set_handler(this, &ctx, HandlerSlot::Error, val)
    }

    #[qjs(get, rename = "onloadend")]
    pub fn get_onloadend(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.get_handler(&ctx, HandlerSlot::LoadEnd)
    }

    #[qjs(set, rename = "onloadend")]
    pub fn set_onloadend(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        val: Value<'js>,
    ) -> Result<()> {
        Self::set_handler(this, &ctx, HandlerSlot::LoadEnd, val)
    }

    pub fn read_as_array_buffer(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        blob: Opt<Value<'js>>,
    ) -> Result<()> {
        Self::start_read(
            this,
            ctx,
            blob,
            Opt(None),
            ReadKind::ArrayBuffer,
            "readAsArrayBuffer",
        )
    }

    pub fn read_as_binary_string(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        blob: Opt<Value<'js>>,
    ) -> Result<()> {
        Self::start_read(
            this,
            ctx,
            blob,
            Opt(None),
            ReadKind::BinaryString,
            "readAsBinaryString",
        )
    }

    #[qjs(rename = "readAsDataURL")]
    pub fn read_as_data_url(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        blob: Opt<Value<'js>>,
    ) -> Result<()> {
        Self::start_read(
            this,
            ctx,
            blob,
            Opt(None),
            ReadKind::DataUrl,
            "readAsDataURL",
        )
    }

    pub fn read_as_text(
        this: This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        blob: Opt<Value<'js>>,
        encoding: Opt<Value<'js>>,
    ) -> Result<()> {
        Self::start_read(this, ctx, blob, encoding, ReadKind::Text, "readAsText")
    }

    pub fn abort(this: This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<()> {
        let should_fire = {
            let mut reader = this.0.borrow_mut();
            if reader.ready_state != LOADING {
                reader.result = None;
                false
            } else {
                reader.ready_state = DONE;
                reader.result = None;
                reader.generation = reader.generation.wrapping_add(1);
                let abort_err = DOMException::new_with_name(
                    &ctx,
                    DOMExceptionName::AbortError,
                    "The operation was aborted.".to_string(),
                )?;
                reader.error = Some(abort_err.into_js(&ctx)?);
                true
            }
        };
        if should_fire {
            Self::dispatch_progress_event(&ctx, &this.0, "abort", false, 0.0, 0.0)?;
            Self::dispatch_progress_event(&ctx, &this.0, "loadend", false, 0.0, 0.0)?;
        }
        Ok(())
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(FileReader)
    }
}

#[rquickjs::class]
#[derive(Trace, Clone, rquickjs::JsLifetime)]
pub struct FileReaderSync {}

impl Default for FileReaderSync {
    fn default() -> Self {
        Self::new()
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl FileReaderSync {
    #[qjs(constructor)]
    pub fn new() -> Self {
        Self {}
    }

    pub fn read_as_array_buffer<'js>(
        &self,
        ctx: Ctx<'js>,
        blob: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let (bytes, mime_type) =
            extract_blob_data(&ctx, blob, "readAsArrayBuffer", "FileReaderSync")?;
        produce_result(&ctx, ReadKind::ArrayBuffer, &bytes, &mime_type, None)
    }

    pub fn read_as_binary_string<'js>(
        &self,
        ctx: Ctx<'js>,
        blob: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let (bytes, mime_type) =
            extract_blob_data(&ctx, blob, "readAsBinaryString", "FileReaderSync")?;
        produce_result(&ctx, ReadKind::BinaryString, &bytes, &mime_type, None)
    }

    #[qjs(rename = "readAsDataURL")]
    pub fn read_as_data_url<'js>(
        &self,
        ctx: Ctx<'js>,
        blob: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let (bytes, mime_type) = extract_blob_data(&ctx, blob, "readAsDataURL", "FileReaderSync")?;
        produce_result(&ctx, ReadKind::DataUrl, &bytes, &mime_type, None)
    }

    pub fn read_as_text<'js>(
        &self,
        ctx: Ctx<'js>,
        blob: Opt<Value<'js>>,
        encoding: Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let (bytes, mime_type) = extract_blob_data(&ctx, blob, "readAsText", "FileReaderSync")?;
        let encoding_str = match encoding.0 {
            Some(v) if !v.is_undefined() => Some(usv_string_from_js(&ctx, &v)?),
            _ => None,
        };
        produce_result(
            &ctx,
            ReadKind::Text,
            &bytes,
            &mime_type,
            encoding_str.as_deref(),
        )
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        stringify!(FileReaderSync)
    }
}
