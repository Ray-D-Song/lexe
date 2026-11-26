// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{
    atom::PredefinedAtom,
    function::{Opt, This},
    Class, Ctx, Exception, Object, Result, Value,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::parse_locale_value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Grapheme,
    Word,
    Sentence,
}

impl Granularity {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "grapheme" => Some(Self::Grapheme),
            "word" => Some(Self::Word),
            "sentence" => Some(Self::Sentence),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Grapheme => "grapheme",
            Self::Word => "word",
            Self::Sentence => "sentence",
        }
    }
}

#[derive(Clone)]
struct SegmentEntry {
    segment: String,
    utf16_index: usize,
    utf16_len: usize,
    is_word_like: Option<bool>,
}

fn build_segment_entries(input: &str, granularity: Granularity) -> (Vec<SegmentEntry>, usize) {
    let mut entries = Vec::new();
    let mut utf16_pos = 0usize;

    let mut push_piece = |piece: &str, is_word_like: Option<bool>| {
        let utf16_len = piece.encode_utf16().count();
        entries.push(SegmentEntry {
            segment: piece.to_string(),
            utf16_index: utf16_pos,
            utf16_len,
            is_word_like,
        });
        utf16_pos += utf16_len;
    };

    match granularity {
        Granularity::Grapheme => {
            for (_, g) in input.grapheme_indices(true) {
                push_piece(g, None);
            }
        },
        Granularity::Word => {
            for (_, w) in input.split_word_bound_indices() {
                let word_like = w.chars().any(char::is_alphanumeric);
                push_piece(w, Some(word_like));
            }
        },
        Granularity::Sentence => {
            for (_, s) in input.split_sentence_bound_indices() {
                push_piece(s, None);
            }
        },
    }

    (entries, utf16_pos)
}

fn entry_to_object<'js>(ctx: &Ctx<'js>, entry: &SegmentEntry, input: &str) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("segment", entry.segment.as_str())?;
    obj.set("index", entry.utf16_index)?;
    obj.set("input", input)?;
    if let Some(word_like) = entry.is_word_like {
        obj.set("isWordLike", word_like)?;
    }
    Ok(obj)
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct SegmentIterator {
    #[qjs(skip_trace)]
    input: String,
    #[qjs(skip_trace)]
    entries: Vec<SegmentEntry>,
    #[qjs(skip_trace)]
    cursor: usize,
}

#[rquickjs::methods]
impl SegmentIterator {
    pub fn next<'js>(&mut self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let out = Object::new(ctx.clone())?;
        if self.cursor < self.entries.len() {
            let entry = &self.entries[self.cursor];
            self.cursor += 1;
            out.set("value", entry_to_object(&ctx, entry, &self.input)?)?;
            out.set("done", false)?;
        } else {
            out.set("value", rquickjs::Undefined)?;
            out.set("done", true)?;
        }
        Ok(out)
    }

    #[qjs(rename = PredefinedAtom::SymbolIterator)]
    pub fn iterator<'js>(this: This<Object<'js>>) -> Object<'js> {
        this.0
    }
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct Segments {
    #[qjs(skip_trace)]
    input: String,
    #[qjs(skip_trace)]
    entries: Vec<SegmentEntry>,
    #[qjs(skip_trace)]
    total_utf16: usize,
}

#[rquickjs::methods]
impl Segments {
    pub fn containing<'js>(&self, ctx: Ctx<'js>, index: Opt<f64>) -> Result<Value<'js>> {
        let raw_idx = index.0.unwrap_or(0.0);
        if raw_idx.is_nan() || raw_idx < 0.0 || raw_idx >= (self.total_utf16 as f64) {
            return Ok(Value::new_undefined(ctx));
        }
        let target = raw_idx.trunc() as usize;
        for entry in &self.entries {
            if target >= entry.utf16_index && target < entry.utf16_index + entry.utf16_len {
                return Ok(entry_to_object(&ctx, entry, &self.input)?.into_value());
            }
        }
        Ok(Value::new_undefined(ctx))
    }

    #[qjs(rename = PredefinedAtom::SymbolIterator)]
    pub fn iterator<'js>(&self, ctx: Ctx<'js>) -> Result<Class<'js, SegmentIterator>> {
        Class::instance(
            ctx,
            SegmentIterator {
                input: self.input.clone(),
                entries: self.entries.clone(),
                cursor: 0,
            },
        )
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Segments"
    }
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct Segmenter {
    #[qjs(skip_trace)]
    locale: String,
    #[qjs(skip_trace)]
    granularity: Granularity,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Segmenter {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, locales: Opt<Value<'_>>, options: Opt<Object<'_>>) -> Result<Self> {
        let locale = parse_locale_value(locales.into_inner())?;
        let mut granularity = Granularity::Grapheme;

        if let Some(opts) = options.into_inner() {
            let g_val: Value<'_> = opts.get("granularity")?;
            if !g_val.is_undefined() {
                let g_str = g_val
                    .as_string()
                    .ok_or_else(|| {
                        Exception::throw_range(&ctx, "Invalid granularity for Intl.Segmenter")
                    })?
                    .to_string()?;
                granularity = Granularity::from_str(&g_str).ok_or_else(|| {
                    Exception::throw_range(
                        &ctx,
                        &format!("Value {g_str} out of range for Intl.Segmenter granularity"),
                    )
                })?;
            }
        }

        Ok(Self {
            locale,
            granularity,
        })
    }

    pub fn segment<'js>(&self, ctx: Ctx<'js>, input: String) -> Result<Class<'js, Segments>> {
        let (entries, total_utf16) = build_segment_entries(&input, self.granularity);
        Class::instance(
            ctx,
            Segments {
                input,
                entries,
                total_utf16,
            },
        )
    }

    #[qjs(rename = "resolvedOptions")]
    pub fn resolved_options<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx)?;
        obj.set("locale", self.locale.as_str())?;
        obj.set("granularity", self.granularity.as_str())?;
        Ok(obj)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Intl.Segmenter"
    }
}
