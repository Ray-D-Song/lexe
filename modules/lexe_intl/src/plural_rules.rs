// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{atom::PredefinedAtom, function::Opt, Array, Ctx, Exception, Object, Result, Value};

use crate::locale::{parse_bcp47_tag, resolve_locale_arg, supported_locales_of_impl};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PluralType {
    Cardinal,
    Ordinal,
}

impl PluralType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cardinal => "cardinal",
            Self::Ordinal => "ordinal",
        }
    }
}

#[derive(Clone)]
pub(crate) struct PluralOperands {
    pub n: f64,
    pub i: u64,
    pub v: usize,
    pub f: u64,
    pub t: u64,
}

fn round_significant_digits(abs_val: f64, min_sig: usize, max_sig: usize) -> (u64, String) {
    if abs_val == 0.0 {
        let frac = if min_sig > 1 {
            "0".repeat(min_sig - 1)
        } else {
            String::new()
        };
        return (0, frac);
    }
    let exp = abs_val.abs().log10().floor() as isize;
    let frac_places = (max_sig as isize - 1 - exp).max(0) as usize;
    let rounded = format!("{abs_val:.frac_places$}");
    let (int_str, raw_frac) = rounded.split_once('.').unwrap_or((&rounded, ""));
    let sig_in_int = if int_str == "0" { 0 } else { int_str.len() };
    let min_frac_for_sig = min_sig.saturating_sub(sig_in_int);

    let trimmed = raw_frac.trim_end_matches('0');
    let frac = if trimmed.len() >= min_frac_for_sig {
        trimmed.to_string()
    } else {
        let mut s = trimmed.to_string();
        while s.len() < min_frac_for_sig {
            s.push('0');
        }
        s
    };
    let int_val = int_str.parse::<u64>().unwrap_or(u64::MAX);
    (int_val, frac)
}

fn compute_operands(
    value: f64,
    min_frac: usize,
    max_frac: usize,
    min_sig: Option<usize>,
    max_sig: Option<usize>,
) -> Option<PluralOperands> {
    if !value.is_finite() {
        return None;
    }
    let abs_val = value.abs();
    let (i, frac_str) = if min_sig.is_some() || max_sig.is_some() {
        round_significant_digits(abs_val, min_sig.unwrap_or(1), max_sig.unwrap_or(21))
    } else {
        let formatted = format!("{abs_val:.max_frac$}");
        let (int_part, frac_part) = formatted.split_once('.').unwrap_or((&formatted, ""));
        let mut keep = frac_part.len();
        while keep > min_frac && frac_part.as_bytes()[keep - 1] == b'0' {
            keep -= 1;
        }
        let int_val = int_part.parse::<u64>().unwrap_or(u64::MAX);
        (int_val, frac_part[..keep].to_string())
    };

    let v = frac_str.len();
    let no_trailing = frac_str.trim_end_matches('0');
    let f = if frac_str.is_empty() {
        0
    } else {
        frac_str.parse::<u64>().unwrap_or(0)
    };
    let t = if no_trailing.is_empty() {
        0
    } else {
        no_trailing.parse::<u64>().unwrap_or(0)
    };
    let n = format!(
        "{i}{}",
        if frac_str.is_empty() {
            String::new()
        } else {
            format!(".{frac_str}")
        }
    )
    .parse::<f64>()
    .unwrap_or(abs_val);

    Some(PluralOperands { n, i, v, f, t })
}

pub(crate) fn select_category_for_lang(
    lang: &str,
    plural_type: PluralType,
    op: &PluralOperands,
) -> &'static str {
    let n = op.n;
    let i = op.i;
    let v = op.v;
    let f = op.f;
    let t = op.t;

    match (lang, plural_type) {
        ("zh" | "ja" | "ko" | "vi" | "th" | "id" | "tr", PluralType::Cardinal) => "other",
        ("zh" | "ja" | "ko" | "vi" | "th" | "id", PluralType::Ordinal) => "other",
        ("fr" | "pt" | "hi" | "bn", PluralType::Cardinal) => {
            if i == 0 || i == 1 {
                "one"
            } else {
                "other"
            }
        },
        ("fr", PluralType::Ordinal) => {
            if (n - 1.0).abs() < f64::EPSILON && f == 0 {
                "one"
            } else {
                "other"
            }
        },
        ("es" | "it", PluralType::Cardinal) => {
            if (n - 1.0).abs() < f64::EPSILON && f == 0 {
                "one"
            } else {
                "other"
            }
        },
        ("ru" | "uk", PluralType::Cardinal) => {
            let mod10 = i % 10;
            let mod100 = i % 100;
            if v == 0 && mod10 == 1 && mod100 != 11 {
                "one"
            } else if v == 0 && (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
                "few"
            } else if (v == 0 && mod10 == 0)
                || (v == 0 && (5..=9).contains(&mod10))
                || (v == 0 && (11..=14).contains(&mod100))
            {
                "many"
            } else {
                "other"
            }
        },
        ("pl", PluralType::Cardinal) => {
            let mod10 = i % 10;
            let mod100 = i % 100;
            if i == 1 && v == 0 {
                "one"
            } else if v == 0 && (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
                "few"
            } else if v == 0
                && ((i != 1 && mod10 <= 1)
                    || (5..=9).contains(&mod10)
                    || (12..=14).contains(&mod100))
            {
                "many"
            } else {
                "other"
            }
        },
        ("cs" | "sk", PluralType::Cardinal) => {
            if i == 1 && v == 0 {
                "one"
            } else if (2..=4).contains(&i) && v == 0 {
                "few"
            } else if v != 0 {
                "many"
            } else {
                "other"
            }
        },
        ("ar", PluralType::Cardinal) => {
            let mod100 = n % 100.0;
            if n == 0.0 {
                "zero"
            } else if n == 1.0 {
                "one"
            } else if n == 2.0 {
                "two"
            } else if t == 0 && (3.0..=10.0).contains(&mod100) {
                "few"
            } else if t == 0 && (11.0..=99.0).contains(&mod100) {
                "many"
            } else {
                "other"
            }
        },
        ("sv", PluralType::Ordinal) => {
            let mod10 = n as u64 % 10;
            let mod100 = n as u64 % 100;
            if v == 0 && (mod10 == 1 || mod10 == 2) && mod100 != 11 && mod100 != 12 {
                "one"
            } else {
                "other"
            }
        },
        (_, PluralType::Ordinal) => {
            // Default & English ordinal: 1st, 2nd, 3rd
            if v == 0 {
                let mod10 = i % 10;
                let mod100 = i % 100;
                if mod10 == 1 && mod100 != 11 {
                    "one"
                } else if mod10 == 2 && mod100 != 12 {
                    "two"
                } else if mod10 == 3 && mod100 != 13 {
                    "few"
                } else {
                    "other"
                }
            } else {
                "other"
            }
        },
        (_, PluralType::Cardinal) => {
            // Default & Germanic (en, de, nl, da, nb, fi, el, he, hu, ro)
            if i == 1 && v == 0 {
                "one"
            } else {
                "other"
            }
        },
    }
}

fn categories_for_lang(lang: &str, plural_type: PluralType) -> &'static [&'static str] {
    match (lang, plural_type) {
        ("zh" | "ja" | "ko" | "vi" | "th" | "id" | "tr", PluralType::Cardinal) => &["other"],
        ("zh" | "ja" | "ko" | "vi" | "th" | "id", PluralType::Ordinal) => &["other"],
        ("fr" | "sv", PluralType::Ordinal) => &["one", "other"],
        ("ru" | "uk" | "pl" | "cs" | "sk", PluralType::Cardinal) => {
            &["one", "few", "many", "other"]
        },
        ("ar", PluralType::Cardinal) => &["zero", "one", "two", "few", "many", "other"],
        (_, PluralType::Ordinal) => &["one", "two", "few", "other"],
        (_, PluralType::Cardinal) => &["one", "other"],
    }
}

pub(crate) fn select_cardinal_for_locale(locale: &str, value: f64) -> &'static str {
    let lang = parse_bcp47_tag(locale)
        .map(|p| p.language)
        .unwrap_or_else(|_| "en".to_string());
    let Some(op) = compute_operands(value, 0, 3, None, None) else {
        return "other";
    };
    select_category_for_lang(&lang, PluralType::Cardinal, &op)
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct PluralRules {
    #[qjs(skip_trace)]
    locale: String,
    #[qjs(skip_trace)]
    language: String,
    #[qjs(skip_trace)]
    plural_type: PluralType,
    #[qjs(skip_trace)]
    minimum_integer_digits: usize,
    #[qjs(skip_trace)]
    minimum_fraction_digits: usize,
    #[qjs(skip_trace)]
    maximum_fraction_digits: usize,
    #[qjs(skip_trace)]
    minimum_significant_digits: Option<usize>,
    #[qjs(skip_trace)]
    maximum_significant_digits: Option<usize>,
    #[qjs(skip_trace)]
    rounding_mode: String,
    #[qjs(skip_trace)]
    trailing_zero_display: String,
}

fn read_digit_option(
    ctx: &Ctx<'_>,
    opts: &Object<'_>,
    name: &str,
    min: usize,
    max: usize,
) -> Result<Option<usize>> {
    let val: Value<'_> = opts.get(name)?;
    if val.is_undefined() {
        return Ok(None);
    }
    let num = val
        .as_number()
        .ok_or_else(|| Exception::throw_range(ctx, &format!("Invalid {name} option")))?;
    if !num.is_finite() || num < min as f64 || num > max as f64 {
        return Err(Exception::throw_range(
            ctx,
            &format!("{name} must be between {min} and {max}"),
        ));
    }
    Ok(Some(num.floor() as usize))
}

#[rquickjs::methods(rename_all = "camelCase")]
impl PluralRules {
    #[qjs(constructor)]
    pub fn new<'js>(
        ctx: Ctx<'js>,
        locales: Opt<Value<'js>>,
        options: Opt<Object<'js>>,
    ) -> Result<Self> {
        let locale = resolve_locale_arg(&ctx, locales.into_inner())?;
        let language = parse_bcp47_tag(&locale)
            .map(|p| p.language)
            .unwrap_or_else(|_| "en".to_string());

        let mut plural_type = PluralType::Cardinal;
        let mut minimum_integer_digits = 1usize;
        let mut min_frac_opt = None;
        let mut max_frac_opt = None;
        let mut min_sig_opt = None;
        let mut max_sig_opt = None;
        let mut rounding_mode = "halfExpand".to_string();
        let mut trailing_zero_display = "auto".to_string();

        if let Some(opts) = options.into_inner() {
            if let Some(lm) = opts.get::<_, Option<String>>("localeMatcher")? {
                if lm != "lookup" && lm != "best fit" {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid localeMatcher option for Intl.PluralRules",
                    ));
                }
            }
            if let Some(t) = opts.get::<_, Option<String>>("type")? {
                plural_type = match t.as_str() {
                    "cardinal" => PluralType::Cardinal,
                    "ordinal" => PluralType::Ordinal,
                    _ => {
                        return Err(Exception::throw_range(
                            &ctx,
                            "Invalid type option for Intl.PluralRules",
                        ))
                    },
                };
            }
            if let Some(v) = read_digit_option(&ctx, &opts, "minimumIntegerDigits", 1, 21)? {
                minimum_integer_digits = v;
            }
            min_frac_opt = read_digit_option(&ctx, &opts, "minimumFractionDigits", 0, 20)?;
            max_frac_opt = read_digit_option(&ctx, &opts, "maximumFractionDigits", 0, 20)?;
            min_sig_opt = read_digit_option(&ctx, &opts, "minimumSignificantDigits", 1, 21)?;
            max_sig_opt = read_digit_option(&ctx, &opts, "maximumSignificantDigits", 1, 21)?;

            if let Some(rm) = opts.get::<_, Option<String>>("roundingMode")? {
                rounding_mode = rm;
            }
            if let Some(tz) = opts.get::<_, Option<String>>("trailingZeroDisplay")? {
                if tz != "auto" && tz != "stripIfInteger" {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid trailingZeroDisplay option",
                    ));
                }
                trailing_zero_display = tz;
            }
        }

        if let (Some(min_s), Some(max_s)) = (min_sig_opt, max_sig_opt) {
            if min_s > max_s {
                return Err(Exception::throw_range(
                    &ctx,
                    "minimumSignificantDigits cannot exceed maximumSignificantDigits",
                ));
            }
        }
        let minimum_fraction_digits = min_frac_opt.unwrap_or(0);
        let maximum_fraction_digits =
            max_frac_opt.unwrap_or_else(|| minimum_fraction_digits.max(3));
        if minimum_fraction_digits > maximum_fraction_digits {
            return Err(Exception::throw_range(
                &ctx,
                "minimumFractionDigits cannot exceed maximumFractionDigits",
            ));
        }

        Ok(Self {
            locale,
            language,
            plural_type,
            minimum_integer_digits,
            minimum_fraction_digits,
            maximum_fraction_digits,
            minimum_significant_digits: min_sig_opt,
            maximum_significant_digits: max_sig_opt,
            rounding_mode,
            trailing_zero_display,
        })
    }

    #[qjs(static, rename = "supportedLocalesOf")]
    pub fn supported_locales_of<'js>(
        ctx: Ctx<'js>,
        locales: Opt<Value<'js>>,
        options: Opt<Object<'js>>,
    ) -> Result<Array<'js>> {
        supported_locales_of_impl(&ctx, locales.into_inner(), options.into_inner())
    }

    pub fn select(&self, value: Opt<Value<'_>>) -> Result<&'static str> {
        let num = match value.into_inner() {
            Some(v) if v.is_number() => v.as_number().unwrap_or(f64::NAN),
            Some(v) => v.as_number().unwrap_or_else(|| {
                v.as_string()
                    .and_then(|s| s.to_string().ok())
                    .and_then(|s| s.trim().parse::<f64>().ok())
                    .unwrap_or(f64::NAN)
            }),
            None => f64::NAN,
        };
        let Some(op) = compute_operands(
            num,
            self.minimum_fraction_digits,
            self.maximum_fraction_digits,
            self.minimum_significant_digits,
            self.maximum_significant_digits,
        ) else {
            return Ok("other");
        };
        Ok(select_category_for_lang(
            &self.language,
            self.plural_type,
            &op,
        ))
    }

    #[qjs(rename = "selectRange")]
    pub fn select_range(
        &self,
        ctx: Ctx<'_>,
        start: Opt<Value<'_>>,
        end: Opt<Value<'_>>,
    ) -> Result<&'static str> {
        let Some(s_val) = start.into_inner() else {
            return Err(Exception::throw_type(
                &ctx,
                "start is required for selectRange",
            ));
        };
        let Some(e_val) = end.into_inner() else {
            return Err(Exception::throw_type(
                &ctx,
                "end is required for selectRange",
            ));
        };
        if s_val.is_undefined() || e_val.is_undefined() {
            return Err(Exception::throw_type(
                &ctx,
                "start and end must not be undefined",
            ));
        }
        let s_num = s_val.as_number().unwrap_or(f64::NAN);
        let e_num = e_val.as_number().unwrap_or(f64::NAN);
        if !s_num.is_finite() || !e_num.is_finite() {
            return Err(Exception::throw_range(
                &ctx,
                "start and end must be finite numbers",
            ));
        }
        let end_cat = match compute_operands(
            e_num,
            self.minimum_fraction_digits,
            self.maximum_fraction_digits,
            self.minimum_significant_digits,
            self.maximum_significant_digits,
        ) {
            Some(op) => select_category_for_lang(&self.language, self.plural_type, &op),
            None => "other",
        };
        if s_num == e_num {
            return Ok(end_cat);
        }
        Ok(match (self.language.as_str(), end_cat) {
            ("en" | "de" | "nl" | "sv" | "es" | "pt" | "fr", _) => "other",
            _ => end_cat,
        })
    }

    #[qjs(rename = "resolvedOptions")]
    pub fn resolved_options<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx.clone())?;
        obj.set("locale", self.locale.as_str())?;
        obj.set("type", self.plural_type.as_str())?;
        obj.set("minimumIntegerDigits", self.minimum_integer_digits)?;
        obj.set("minimumFractionDigits", self.minimum_fraction_digits)?;
        obj.set("maximumFractionDigits", self.maximum_fraction_digits)?;
        if let Some(min_s) = self.minimum_significant_digits {
            obj.set("minimumSignificantDigits", min_s)?;
        }
        if let Some(max_s) = self.maximum_significant_digits {
            obj.set("maximumSignificantDigits", max_s)?;
        }
        let cats = Array::new(ctx)?;
        for (i, c) in categories_for_lang(&self.language, self.plural_type)
            .iter()
            .enumerate()
        {
            cats.set(i, *c)?;
        }
        obj.set("pluralCategories", cats)?;
        obj.set("roundingIncrement", 1)?;
        obj.set("roundingMode", self.rounding_mode.as_str())?;
        obj.set("trailingZeroDisplay", self.trailing_zero_display.as_str())?;
        Ok(obj)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Intl.PluralRules"
    }
}
