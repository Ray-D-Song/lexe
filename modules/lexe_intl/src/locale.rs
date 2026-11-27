// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use rquickjs::{
    atom::PredefinedAtom, function::Opt, Array, Class, Ctx, Exception, Object, Result, Value,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedLocale {
    pub language: String,
    pub script: Option<String>,
    pub region: Option<String>,
    pub variants: Vec<String>,
    pub unicode_attrs: Vec<String>,
    pub unicode_keywords: BTreeMap<String, String>,
    pub rest_extensions: Vec<String>,
}

fn is_valid_language(s: &str) -> bool {
    let len = s.len();
    ((2..=3).contains(&len) || (5..=8).contains(&len)) && s.bytes().all(|b| b.is_ascii_alphabetic())
}

fn is_valid_script(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphabetic())
}

fn is_valid_region(s: &str) -> bool {
    (s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic()))
        || (s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit()))
}

fn is_valid_variant(s: &str) -> bool {
    let bytes = s.as_bytes();
    if (5..=8).contains(&bytes.len()) {
        return bytes.iter().all(|b| b.is_ascii_alphanumeric());
    }
    bytes.len() == 4
        && bytes[0].is_ascii_digit()
        && bytes[1..].iter().all(|b| b.is_ascii_alphanumeric())
}

fn normalize_language(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "iw" => "he".to_string(),
        "in" => "id".to_string(),
        "ji" => "yi".to_string(),
        "mo" => "ro".to_string(),
        other => other.to_string(),
    }
}

fn normalize_script(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

fn normalize_region(s: &str) -> String {
    s.to_ascii_uppercase()
}

fn is_singleton(s: &str) -> bool {
    s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() && s != "x" && s != "X"
}

fn is_alphanum_subtag(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn valid_unicode_type_seq(s: &str) -> bool {
    !s.is_empty() && s.split('-').all(|sub| is_alphanum_subtag(sub, 3, 8))
}

pub(crate) fn parse_bcp47_tag(raw: &str) -> std::result::Result<ParsedLocale, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('_') {
        return Err(format!("Incorrect locale information provided: {raw}"));
    }

    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("Incorrect locale information provided: {raw}"));
    }

    let mut idx = 0usize;

    // Language subtag
    let lang_sub = parts
        .get(idx)
        .ok_or_else(|| "Empty locale tag".to_string())?;
    if !is_valid_language(lang_sub) {
        return Err(format!("Invalid language subtag in locale: {raw}"));
    }
    let language = normalize_language(lang_sub);
    idx += 1;

    // Optional extlang (e.g., zh-cmn) -> fold only 1 3-alpha extlang if not followed by script/region?
    if let Some(&next) = parts.get(idx) {
        if next.len() == 3 && next.bytes().all(|b| b.is_ascii_alphabetic()) {
            idx += 1;
        }
    }

    // Optional script (4 alpha)
    let mut script = None;
    if let Some(&next) = parts.get(idx) {
        if is_valid_script(next) {
            script = Some(normalize_script(next));
            idx += 1;
        }
    }

    // Optional region (2 alpha or 3 digit)
    let mut region = None;
    if let Some(&next) = parts.get(idx) {
        if is_valid_region(next) {
            region = Some(normalize_region(next));
            idx += 1;
        }
    }

    // Optional variants before any singleton extension (-u-, -t-, -x-, etc.)
    let mut variants = Vec::new();
    while let Some(&next) = parts.get(idx) {
        if next.len() == 1 {
            break;
        }
        if !is_valid_variant(next) {
            return Err(format!("Invalid variant subtag `{next}` in locale: {raw}"));
        }
        let v_lower = next.to_ascii_lowercase();
        if variants.contains(&v_lower) {
            return Err(format!("Duplicate variant `{next}` in locale: {raw}"));
        }
        variants.push(v_lower);
        idx += 1;
    }
    variants.sort();

    // Parse extensions (-u-, -t-, etc.) and private use (-x-)
    let mut unicode_attrs = Vec::new();
    let mut unicode_keywords: BTreeMap<String, String> = BTreeMap::new();
    let mut rest_extensions = Vec::new();
    let mut seen_singletons: Vec<char> = Vec::new();

    while idx < parts.len() {
        let singleton = parts[idx];
        if singleton.len() != 1 || !singleton.as_bytes()[0].is_ascii_alphanumeric() {
            return Err(format!(
                "Invalid extension subtag `{singleton}` in locale: {raw}"
            ));
        }
        let s_ch = singleton.as_bytes()[0].to_ascii_lowercase() as char;
        idx += 1;

        if s_ch == 'x' {
            if idx >= parts.len() {
                return Err(format!("Incomplete private-use subtag in locale: {raw}"));
            }
            let mut priv_parts = vec!["x".to_string()];
            while idx < parts.len() {
                if !is_alphanum_subtag(parts[idx], 1, 8) {
                    return Err(format!("Invalid private-use subtag in locale: {raw}"));
                }
                priv_parts.push(parts[idx].to_ascii_lowercase());
                idx += 1;
            }
            rest_extensions.push(priv_parts.join("-"));
            break;
        }

        if !is_singleton(singleton) || seen_singletons.contains(&s_ch) {
            return Err(format!(
                "Duplicate or invalid singleton `{singleton}` in locale: {raw}"
            ));
        }
        seen_singletons.push(s_ch);

        let start_ext = idx;
        while idx < parts.len() && parts[idx].len() > 1 {
            if !is_alphanum_subtag(parts[idx], 2, 8) {
                return Err(format!(
                    "Invalid extension subtag `{}` in locale: {raw}",
                    parts[idx]
                ));
            }
            idx += 1;
        }
        if idx == start_ext {
            return Err(format!("Empty extension `-{s_ch}` in locale: {raw}"));
        }

        if s_ch == 'u' {
            let ext_slice = &parts[start_ext..idx];
            let mut k = 0usize;
            // Attributes have length 3..=8 and come before any 2-char keyword key
            while k < ext_slice.len() && ext_slice[k].len() >= 3 {
                let attr = ext_slice[k].to_ascii_lowercase();
                if !unicode_attrs.contains(&attr) {
                    unicode_attrs.push(attr);
                }
                k += 1;
            }
            unicode_attrs.sort();

            while k < ext_slice.len() {
                let key = ext_slice[k].to_ascii_lowercase();
                if key.len() != 2 {
                    return Err(format!(
                        "Invalid Unicode extension key `{key}` in locale: {raw}"
                    ));
                }
                k += 1;
                let mut vals = Vec::new();
                while k < ext_slice.len() && ext_slice[k].len() >= 3 {
                    vals.push(ext_slice[k].to_ascii_lowercase());
                    k += 1;
                }
                unicode_keywords
                    .entry(key)
                    .or_insert_with(|| vals.join("-"));
            }
        } else {
            let mut ext_parts = vec![s_ch.to_string()];
            for &sub in &parts[start_ext..idx] {
                ext_parts.push(sub.to_ascii_lowercase());
            }
            rest_extensions.push(ext_parts.join("-"));
        }
    }

    Ok(ParsedLocale {
        language,
        script,
        region,
        variants,
        unicode_attrs,
        unicode_keywords,
        rest_extensions,
    })
}

impl ParsedLocale {
    pub fn base_name(&self) -> String {
        let mut parts = vec![self.language.clone()];
        if let Some(ref s) = self.script {
            parts.push(s.clone());
        }
        if let Some(ref r) = self.region {
            parts.push(r.clone());
        }
        for v in &self.variants {
            parts.push(v.clone());
        }
        parts.join("-")
    }

    pub fn to_tag_string(&self) -> String {
        let mut out = self.base_name();
        if !self.unicode_attrs.is_empty() || !self.unicode_keywords.is_empty() {
            out.push_str("-u");
            for attr in &self.unicode_attrs {
                out.push('-');
                out.push_str(attr);
            }
            for (k, v) in &self.unicode_keywords {
                out.push('-');
                out.push_str(k);
                if !v.is_empty() && !(k == "kn" && v == "true") {
                    out.push('-');
                    out.push_str(v);
                }
            }
        }
        for ext in &self.rest_extensions {
            out.push('-');
            out.push_str(ext);
        }
        out
    }

    fn likely_subtags(
        lang: &str,
        script: Option<&str>,
        region: Option<&str>,
    ) -> (&'static str, &'static str, &'static str) {
        match (lang, script, region) {
            ("zh", Some("Hant"), _)
            | ("zh", _, Some("TW"))
            | ("zh", _, Some("HK"))
            | ("zh", _, Some("MO")) => {
                let reg = match region {
                    Some("HK") => "HK",
                    Some("MO") => "MO",
                    _ => "TW",
                };
                ("zh", "Hant", reg)
            },
            ("zh", _, _) => ("zh", "Hans", "CN"),
            ("en", _, Some("GB")) => ("en", "Latn", "GB"),
            ("en", _, Some("AU")) => ("en", "Latn", "AU"),
            ("en", _, _) => ("en", "Latn", "US"),
            ("ja", _, _) => ("ja", "Jpan", "JP"),
            ("ko", _, _) => ("ko", "Kore", "KR"),
            ("fr", _, Some("CA")) => ("fr", "Latn", "CA"),
            ("fr", _, _) => ("fr", "Latn", "FR"),
            ("de", _, Some("AT")) => ("de", "Latn", "AT"),
            ("de", _, Some("CH")) => ("de", "Latn", "CH"),
            ("de", _, _) => ("de", "Latn", "DE"),
            ("es", _, Some("MX")) => ("es", "Latn", "MX"),
            ("es", _, Some("419")) => ("es", "Latn", "419"),
            ("es", _, _) => ("es", "Latn", "ES"),
            ("pt", _, Some("BR")) => ("pt", "Latn", "BR"),
            ("pt", _, _) => ("pt", "Latn", "PT"),
            ("it", _, _) => ("it", "Latn", "IT"),
            ("nl", _, _) => ("nl", "Latn", "NL"),
            ("ru", _, _) => ("ru", "Cyrl", "RU"),
            ("uk", _, _) => ("uk", "Cyrl", "UA"),
            ("pl", _, _) => ("pl", "Latn", "PL"),
            ("ar", _, Some("SA")) => ("ar", "Arab", "SA"),
            ("ar", _, _) => ("ar", "Arab", "EG"),
            ("hi", _, _) => ("hi", "Deva", "IN"),
            ("bn", _, _) => ("bn", "Beng", "BD"),
            ("vi", _, _) => ("vi", "Latn", "VN"),
            ("th", _, _) => ("th", "Thai", "TH"),
            ("id", _, _) => ("id", "Latn", "ID"),
            ("tr", _, _) => ("tr", "Latn", "TR"),
            ("sv", _, _) => ("sv", "Latn", "SE"),
            ("da", _, _) => ("da", "Latn", "DK"),
            ("nb" | "no", _, _) => ("nb", "Latn", "NO"),
            ("nn", _, _) => ("nn", "Latn", "NO"),
            ("fi", _, _) => ("fi", "Latn", "FI"),
            ("cs", _, _) => ("cs", "Latn", "CZ"),
            ("el", _, _) => ("el", "Grek", "GR"),
            ("he", _, _) => ("he", "Hebr", "IL"),
            ("hu", _, _) => ("hu", "Latn", "HU"),
            ("ro", _, _) => ("ro", "Latn", "RO"),
            ("und", _, _) => ("en", "Latn", "US"),
            _ => ("", "", ""),
        }
    }

    pub fn maximize(&self) -> Self {
        let mut next = self.clone();
        let (l, s, r) = Self::likely_subtags(
            &self.language,
            self.script.as_deref(),
            self.region.as_deref(),
        );
        if !l.is_empty() && next.language == "und" {
            next.language = l.to_string();
        }
        if next.script.is_none() && !s.is_empty() {
            next.script = Some(s.to_string());
        }
        if next.region.is_none() && !r.is_empty() {
            next.region = Some(r.to_string());
        }
        next
    }

    pub fn minimize(&self) -> Self {
        let max = self.maximize();
        let mut candidate = self.clone();

        // Try lang only
        candidate.script = None;
        candidate.region = None;
        if candidate.maximize().base_name() == max.base_name() {
            return candidate;
        }

        // Try lang + region
        if max.region.is_some() {
            candidate.script = None;
            candidate.region = max.region.clone();
            if candidate.maximize().base_name() == max.base_name() {
                return candidate;
            }
        }

        // Try lang + script
        if max.script.is_some() {
            candidate.script = max.script.clone();
            candidate.region = None;
            if candidate.maximize().base_name() == max.base_name() {
                return candidate;
            }
        }

        max
    }
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct Locale {
    #[qjs(skip_trace)]
    parsed: ParsedLocale,
}

impl Locale {
    fn from_value_inner(ctx: &Ctx<'_>, val: Value<'_>) -> Result<ParsedLocale> {
        if let Some(obj) = val.as_object() {
            if let Some(cls) = Class::<Self>::from_object(obj) {
                return Ok(cls.borrow().parsed.clone());
            }
        }
        if let Some(s) = val.as_string() {
            let tag = s.to_string()?;
            return parse_bcp47_tag(&tag).map_err(|msg| Exception::throw_range(ctx, &msg));
        }
        Err(Exception::throw_type(
            ctx,
            "First argument to Intl.Locale constructor must be a string or Locale object",
        ))
    }

    fn apply_options(&mut self, ctx: &Ctx<'_>, opts: &Object<'_>) -> Result<()> {
        if let Some(v) = opts.get::<_, Option<String>>("language")? {
            if !is_valid_language(&v) {
                return Err(Exception::throw_range(
                    ctx,
                    "Invalid language option for Intl.Locale",
                ));
            }
            self.parsed.language = normalize_language(&v);
        }
        if let Some(v) = opts.get::<_, Option<String>>("script")? {
            if !is_valid_script(&v) {
                return Err(Exception::throw_range(
                    ctx,
                    "Invalid script option for Intl.Locale",
                ));
            }
            self.parsed.script = Some(normalize_script(&v));
        }
        if let Some(v) = opts.get::<_, Option<String>>("region")? {
            if !is_valid_region(&v) {
                return Err(Exception::throw_range(
                    ctx,
                    "Invalid region option for Intl.Locale",
                ));
            }
            self.parsed.region = Some(normalize_region(&v));
        }
        if let Some(v) = opts.get::<_, Option<String>>("calendar")? {
            let lower = v.to_ascii_lowercase();
            if !valid_unicode_type_seq(&lower) {
                return Err(Exception::throw_range(
                    ctx,
                    "Invalid calendar option for Intl.Locale",
                ));
            }
            self.parsed.unicode_keywords.insert("ca".to_string(), lower);
        }
        if let Some(v) = opts.get::<_, Option<String>>("collation")? {
            let lower = v.to_ascii_lowercase();
            if !valid_unicode_type_seq(&lower) {
                return Err(Exception::throw_range(
                    ctx,
                    "Invalid collation option for Intl.Locale",
                ));
            }
            self.parsed.unicode_keywords.insert("co".to_string(), lower);
        }
        if let Some(v) = opts.get::<_, Option<String>>("caseFirst")? {
            if !matches!(v.as_str(), "upper" | "lower" | "false") {
                return Err(Exception::throw_range(
                    ctx,
                    "Invalid caseFirst option for Intl.Locale",
                ));
            }
            self.parsed.unicode_keywords.insert("kf".to_string(), v);
        }
        if let Some(v) = opts.get::<_, Option<String>>("hourCycle")? {
            if !matches!(v.as_str(), "h11" | "h12" | "h23" | "h24") {
                return Err(Exception::throw_range(
                    ctx,
                    "Invalid hourCycle option for Intl.Locale",
                ));
            }
            self.parsed.unicode_keywords.insert("hc".to_string(), v);
        }
        if let Some(v) = opts.get::<_, Option<String>>("numberingSystem")? {
            let lower = v.to_ascii_lowercase();
            if !valid_unicode_type_seq(&lower) {
                return Err(Exception::throw_range(
                    ctx,
                    "Invalid numberingSystem option for Intl.Locale",
                ));
            }
            self.parsed.unicode_keywords.insert("nu".to_string(), lower);
        }
        if let Some(v) = opts.get::<_, Option<String>>("firstDayOfWeek")? {
            let mapped = match v.to_ascii_lowercase().as_str() {
                "mon" | "1" => "mon",
                "tue" | "2" => "tue",
                "wed" | "3" => "wed",
                "thu" | "4" => "thu",
                "fri" | "5" => "fri",
                "sat" | "6" => "sat",
                "sun" | "7" | "0" => "sun",
                _ => {
                    return Err(Exception::throw_range(
                        ctx,
                        "Invalid firstDayOfWeek option for Intl.Locale",
                    ))
                },
            };
            self.parsed
                .unicode_keywords
                .insert("fw".to_string(), mapped.to_string());
        }
        let kn_val: Value<'_> = opts.get("numeric")?;
        if !kn_val.is_undefined() {
            let is_true = kn_val.as_bool().unwrap_or(false)
                || kn_val
                    .as_string()
                    .and_then(|s| s.to_string().ok())
                    .is_some_and(|s| s == "true");
            self.parsed.unicode_keywords.insert(
                "kn".to_string(),
                if is_true { "true" } else { "false" }.to_string(),
            );
        }
        Ok(())
    }

    fn keyword_or_undefined<'js>(&self, ctx: &Ctx<'js>, key: &str) -> Result<Value<'js>> {
        match self.parsed.unicode_keywords.get(key) {
            Some(v) if !v.is_empty() => {
                Ok(rquickjs::String::from_str(ctx.clone(), v)?.into_value())
            },
            _ => Ok(Value::new_undefined(ctx.clone())),
        }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Locale {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, tag: Opt<Value<'_>>, options: Opt<Object<'_>>) -> Result<Self> {
        let Some(tag_val) = tag.into_inner() else {
            return Err(Exception::throw_type(
                &ctx,
                "Intl.Locale requires at least 1 argument",
            ));
        };
        let parsed = Self::from_value_inner(&ctx, tag_val)?;
        let mut locale = Self { parsed };
        if let Some(opts) = options.into_inner() {
            locale.apply_options(&ctx, &opts)?;
        }
        Ok(locale)
    }

    pub fn maximize(&self) -> Self {
        Self {
            parsed: self.parsed.maximize(),
        }
    }

    pub fn minimize(&self) -> Self {
        Self {
            parsed: self.parsed.minimize(),
        }
    }

    #[allow(clippy::inherent_to_string)]
    #[qjs(rename = PredefinedAtom::ToString)]
    pub fn to_string(&self) -> String {
        self.parsed.to_tag_string()
    }

    #[qjs(get, rename = "baseName")]
    pub fn base_name(&self) -> String {
        self.parsed.base_name()
    }

    #[qjs(get)]
    pub fn language(&self) -> String {
        self.parsed.language.clone()
    }

    #[qjs(get)]
    pub fn script(&self) -> Option<String> {
        self.parsed.script.clone()
    }

    #[qjs(get)]
    pub fn region(&self) -> Option<String> {
        self.parsed.region.clone()
    }

    #[qjs(get)]
    pub fn variants(&self) -> Option<String> {
        if self.parsed.variants.is_empty() {
            None
        } else {
            Some(self.parsed.variants.join("-"))
        }
    }

    #[qjs(get)]
    pub fn calendar<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.keyword_or_undefined(&ctx, "ca")
    }

    #[qjs(get, rename = "caseFirst")]
    pub fn case_first<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.keyword_or_undefined(&ctx, "kf")
    }

    #[qjs(get)]
    pub fn collation<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.keyword_or_undefined(&ctx, "co")
    }

    #[qjs(get, rename = "firstDayOfWeek")]
    pub fn first_day_of_week<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.keyword_or_undefined(&ctx, "fw")
    }

    #[qjs(get, rename = "hourCycle")]
    pub fn hour_cycle<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.keyword_or_undefined(&ctx, "hc")
    }

    #[qjs(get, rename = "numberingSystem")]
    pub fn numbering_system<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        self.keyword_or_undefined(&ctx, "nu")
    }

    #[qjs(get)]
    pub fn numeric(&self) -> bool {
        match self.parsed.unicode_keywords.get("kn") {
            Some(v) => v != "false",
            None => false,
        }
    }

    #[qjs(rename = "getCalendars")]
    pub fn get_calendars<'js>(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        let arr = Array::new(ctx)?;
        if let Some(ca) = self.parsed.unicode_keywords.get("ca") {
            arr.set(0, ca.as_str())?;
        } else {
            arr.set(0, "gregory")?;
        }
        Ok(arr)
    }

    #[qjs(rename = "getCollations")]
    pub fn get_collations<'js>(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        let arr = Array::new(ctx)?;
        if let Some(co) = self.parsed.unicode_keywords.get("co") {
            arr.set(0, co.as_str())?;
        } else {
            arr.set(0, "emoji")?;
            arr.set(1, "eor")?;
        }
        Ok(arr)
    }

    #[qjs(rename = "getHourCycles")]
    pub fn get_hour_cycles<'js>(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        let arr = Array::new(ctx)?;
        if let Some(hc) = self.parsed.unicode_keywords.get("hc") {
            arr.set(0, hc.as_str())?;
        } else if self.parsed.region.as_deref() == Some("US")
            || self.parsed.region.as_deref() == Some("AU")
            || self.parsed.region.as_deref() == Some("CA")
            || self.parsed.language == "en" && self.parsed.region.is_none()
        {
            arr.set(0, "h12")?;
        } else {
            arr.set(0, "h23")?;
        }
        Ok(arr)
    }

    #[qjs(rename = "getNumberingSystems")]
    pub fn get_numbering_systems<'js>(&self, ctx: Ctx<'js>) -> Result<Array<'js>> {
        let arr = Array::new(ctx)?;
        if let Some(nu) = self.parsed.unicode_keywords.get("nu") {
            arr.set(0, nu.as_str())?;
        } else {
            arr.set(0, "latn")?;
        }
        Ok(arr)
    }

    #[qjs(rename = "getTextInfo")]
    pub fn get_text_info<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx)?;
        let dir = if matches!(self.parsed.language.as_str(), "ar" | "he" | "fa" | "ur") {
            "rtl"
        } else {
            "ltr"
        };
        obj.set("direction", dir)?;
        Ok(obj)
    }

    #[qjs(rename = "getWeekInfo")]
    pub fn get_week_info<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx.clone())?;
        let first_day: i32 = match self.parsed.unicode_keywords.get("fw").map(String::as_str) {
            Some("mon") => 1,
            Some("tue") => 2,
            Some("wed") => 3,
            Some("thu") => 4,
            Some("fri") => 5,
            Some("sat") => 6,
            Some("sun") => 7,
            _ => match self.parsed.region.as_deref() {
                Some("US" | "CA" | "JP" | "BR" | "MX" | "KR" | "TW" | "HK" | "IL" | "ZA") => 7,
                Some("SA" | "AE" | "EG") => 6,
                _ => 1,
            },
        };
        let weekend = Array::new(ctx)?;
        if matches!(
            self.parsed.region.as_deref(),
            Some("SA" | "AE" | "EG" | "IL")
        ) {
            weekend.set(0, 5)?;
            weekend.set(1, 6)?;
        } else {
            weekend.set(0, 6)?;
            weekend.set(1, 7)?;
        }
        let minimal_days = if matches!(
            self.parsed.region.as_deref(),
            Some(
                "DE" | "FR"
                    | "ES"
                    | "IT"
                    | "NL"
                    | "BE"
                    | "CH"
                    | "AT"
                    | "SE"
                    | "NO"
                    | "DK"
                    | "FI"
                    | "PL"
                    | "PT"
                    | "GB"
            )
        ) {
            4
        } else {
            1
        };
        obj.set("firstDay", first_day)?;
        obj.set("weekend", weekend)?;
        obj.set("minimalDays", minimal_days)?;
        Ok(obj)
    }

    #[qjs(rename = "getTimeZones")]
    pub fn get_time_zones<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let zones: Option<&[&str]> = match self.parsed.region.as_deref() {
            Some("US") => Some(&[
                "America/New_York",
                "America/Chicago",
                "America/Denver",
                "America/Los_Angeles",
            ]),
            Some("GB") => Some(&["Europe/London"]),
            Some("CN") => Some(&["Asia/Shanghai", "Asia/Urumqi"]),
            Some("JP") => Some(&["Asia/Tokyo"]),
            Some("KR") => Some(&["Asia/Seoul"]),
            Some("DE") => Some(&["Europe/Berlin"]),
            Some("FR") => Some(&["Europe/Paris"]),
            _ => None,
        };
        match zones {
            Some(list) => {
                let arr = Array::new(ctx)?;
                for (i, z) in list.iter().enumerate() {
                    arr.set(i, *z)?;
                }
                Ok(arr.into_value())
            },
            None => Ok(Value::new_undefined(ctx)),
        }
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Intl.Locale"
    }
}

pub(crate) fn canonicalize_locale_list<'js>(
    ctx: &Ctx<'js>,
    locales: Option<Value<'js>>,
) -> Result<Vec<String>> {
    let Some(val) = locales else {
        return Ok(Vec::new());
    };
    if val.is_undefined() {
        return Ok(Vec::new());
    }
    if val.is_null() {
        return Err(Exception::throw_type(
            ctx,
            "Cannot convert null to locale list",
        ));
    }
    if let Some(s) = val.as_string() {
        let raw = s.to_string()?;
        let parsed = parse_bcp47_tag(&raw).map_err(|msg| Exception::throw_range(ctx, &msg))?;
        return Ok(vec![parsed.base_name()]);
    }
    if let Some(obj) = val.as_object() {
        if let Some(cls) = Class::<Locale>::from_object(obj) {
            return Ok(vec![cls.borrow().parsed.base_name()]);
        }
        let array_ctor: Object<'js> = ctx.globals().get("Array")?;
        let from_fn: rquickjs::Function<'js> = array_ctor.get("from")?;
        let arr: Array<'js> = from_fn.call((val,))?;
        let mut out = Vec::with_capacity(arr.len());
        for entry in arr.iter::<Value<'js>>() {
            let item = entry?;
            let parsed = if let Some(item_obj) = item.as_object() {
                if let Some(cls) = Class::<Locale>::from_object(item_obj) {
                    cls.borrow().parsed.clone()
                } else {
                    return Err(Exception::throw_type(
                        ctx,
                        "Language ID must be string or Intl.Locale",
                    ));
                }
            } else if let Some(s) = item.as_string() {
                let raw = s.to_string()?;
                parse_bcp47_tag(&raw).map_err(|msg| Exception::throw_range(ctx, &msg))?
            } else {
                return Err(Exception::throw_type(
                    ctx,
                    "Language ID must be string or Intl.Locale",
                ));
            };
            let canonical = parsed.base_name();
            if !out.contains(&canonical) {
                out.push(canonical);
            }
        }
        return Ok(out);
    }
    Err(Exception::throw_type(
        ctx,
        "Language ID must be string, Intl.Locale, or array",
    ))
}

pub(crate) fn resolve_locale_arg<'js>(
    ctx: &Ctx<'js>,
    locales: Option<Value<'js>>,
) -> Result<String> {
    let list = canonicalize_locale_list(ctx, locales)?;
    Ok(list
        .into_iter()
        .next()
        .unwrap_or_else(|| "en-US".to_string()))
}

pub(crate) fn supported_locales_of_impl<'js>(
    ctx: &Ctx<'js>,
    locales: Option<Value<'js>>,
    options: Option<Object<'js>>,
) -> Result<Array<'js>> {
    if let Some(opts) = options {
        if let Some(lm) = opts.get::<_, Option<String>>("localeMatcher")? {
            if lm != "lookup" && lm != "best fit" {
                return Err(Exception::throw_range(ctx, "Invalid localeMatcher option"));
            }
        }
    }
    let list = canonicalize_locale_list(ctx, locales)?;
    let arr = Array::new(ctx.clone())?;
    for (i, tag) in list.into_iter().enumerate() {
        arr.set(i, tag)?;
    }
    Ok(arr)
}
