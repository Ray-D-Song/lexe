// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{atom::PredefinedAtom, function::Opt, Array, Ctx, Exception, Object, Result, Value};

use crate::parse_locale_value;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListStyle {
    Long,
    Short,
    Narrow,
}

impl ListStyle {
    fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
            Self::Narrow => "narrow",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListType {
    Conjunction,
    Disjunction,
    Unit,
}

impl ListType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Conjunction => "conjunction",
            Self::Disjunction => "disjunction",
            Self::Unit => "unit",
        }
    }
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct ListFormat {
    #[qjs(skip_trace)]
    locale: String,
    #[qjs(skip_trace)]
    style: ListStyle,
    #[qjs(skip_trace)]
    list_type: ListType,
}

impl ListFormat {
    fn build_parts(&self, items: Vec<String>) -> Vec<(&'static str, String)> {
        let len = items.len();
        if len == 0 {
            return Vec::new();
        }
        if len == 1 {
            return vec![("element", items.into_iter().next().unwrap())];
        }

        let pair_sep = match (self.list_type, self.style) {
            (ListType::Conjunction, ListStyle::Long) => " and ",
            (ListType::Conjunction, ListStyle::Short) => " & ",
            (ListType::Conjunction, ListStyle::Narrow) => ", ",
            (ListType::Disjunction, _) => " or ",
            (ListType::Unit, ListStyle::Long | ListStyle::Short) => ", ",
            (ListType::Unit, ListStyle::Narrow) => " ",
        };

        if len == 2 {
            let mut it = items.into_iter();
            let a = it.next().unwrap();
            let b = it.next().unwrap();
            return vec![
                ("element", a),
                ("literal", pair_sep.to_string()),
                ("element", b),
            ];
        }

        let mid_sep = match (self.list_type, self.style) {
            (ListType::Unit, ListStyle::Narrow) => " ",
            _ => ", ",
        };
        let end_sep = match (self.list_type, self.style) {
            (ListType::Conjunction, ListStyle::Long) => ", and ",
            (ListType::Conjunction, ListStyle::Short) => ", & ",
            (ListType::Conjunction, ListStyle::Narrow) => ", ",
            (ListType::Disjunction, ListStyle::Long | ListStyle::Short) => ", or ",
            (ListType::Disjunction, ListStyle::Narrow) => ", or ",
            (ListType::Unit, ListStyle::Long | ListStyle::Short) => ", ",
            (ListType::Unit, ListStyle::Narrow) => " ",
        };

        let mut parts = Vec::with_capacity(len * 2 - 1);
        for (idx, item) in items.into_iter().enumerate() {
            if idx > 0 {
                let sep = if idx + 1 == len { end_sep } else { mid_sep };
                parts.push(("literal", sep.to_string()));
            }
            parts.push(("element", item));
        }
        parts
    }

    fn collect_items<'js>(&self, ctx: &Ctx<'js>, list: Opt<Value<'js>>) -> Result<Vec<String>> {
        let Some(val) = list.0 else {
            return Ok(Vec::new());
        };
        if val.is_undefined() {
            return Ok(Vec::new());
        }
        let array_ctor: rquickjs::Object<'js> = ctx.globals().get("Array")?;
        let from_fn: rquickjs::Function<'js> = array_ctor.get("from")?;
        let arr: Array<'js> = from_fn.call((val,))?;
        let mut items = Vec::with_capacity(arr.len());
        for entry in arr.iter::<Value<'js>>() {
            let v = entry?;
            let s = v
                .as_string()
                .ok_or_else(|| {
                    Exception::throw_type(
                        ctx,
                        "Iterable yielded non-string value in Intl.ListFormat",
                    )
                })?
                .to_string()?;
            items.push(s);
        }
        Ok(items)
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl ListFormat {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, locales: Opt<Value<'_>>, options: Opt<Object<'_>>) -> Result<Self> {
        let locale = parse_locale_value(locales.into_inner())?;
        let mut style = ListStyle::Long;
        let mut list_type = ListType::Conjunction;

        if let Some(opts) = options.into_inner() {
            if let Ok(Some(s)) = opts.get::<_, Option<String>>("style") {
                style = match s.as_str() {
                    "long" => ListStyle::Long,
                    "short" => ListStyle::Short,
                    "narrow" => ListStyle::Narrow,
                    _ => {
                        return Err(Exception::throw_range(
                            &ctx,
                            "Invalid style for Intl.ListFormat",
                        ))
                    },
                };
            }
            if let Ok(Some(t)) = opts.get::<_, Option<String>>("type") {
                list_type = match t.as_str() {
                    "conjunction" => ListType::Conjunction,
                    "disjunction" => ListType::Disjunction,
                    "unit" => ListType::Unit,
                    _ => {
                        return Err(Exception::throw_range(
                            &ctx,
                            "Invalid type for Intl.ListFormat",
                        ))
                    },
                };
            }
        }

        Ok(Self {
            locale,
            style,
            list_type,
        })
    }

    pub fn format<'js>(&self, ctx: Ctx<'js>, list: Opt<Value<'js>>) -> Result<String> {
        let items = self.collect_items(&ctx, list)?;
        let parts = self.build_parts(items);
        let mut out = String::new();
        for (_, text) in parts {
            out.push_str(&text);
        }
        Ok(out)
    }

    #[qjs(rename = "formatToParts")]
    pub fn format_to_parts<'js>(&self, ctx: Ctx<'js>, list: Opt<Value<'js>>) -> Result<Array<'js>> {
        let items = self.collect_items(&ctx, list)?;
        let parts = self.build_parts(items);
        let arr = Array::new(ctx.clone())?;
        for (i, (ty, text)) in parts.into_iter().enumerate() {
            let obj = Object::new(ctx.clone())?;
            obj.set("type", ty)?;
            obj.set("value", text)?;
            arr.set(i, obj)?;
        }
        Ok(arr)
    }

    #[qjs(rename = "resolvedOptions")]
    pub fn resolved_options<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx)?;
        obj.set("locale", self.locale.as_str())?;
        obj.set("type", self.list_type.as_str())?;
        obj.set("style", self.style.as_str())?;
        Ok(obj)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Intl.ListFormat"
    }
}
