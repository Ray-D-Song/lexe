// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{atom::PredefinedAtom, function::Opt, Array, Ctx, Exception, Object, Result, Value};

use crate::parse_locale_value;

#[derive(Clone)]
struct NumberFormatOptions {
    locale: String,
    style: String,
    currency: Option<String>,
    currency_display: String,
    notation: String,
    compact_display: String,
    minimum_integer_digits: usize,
    minimum_fraction_digits: usize,
    maximum_fraction_digits: usize,
    use_grouping: bool,
}

fn currency_symbol(code: &str, display: &str) -> String {
    if display == "code" {
        return code.to_string();
    }
    match code {
        "USD" => "$".to_string(),
        "EUR" => "€".to_string(),
        "GBP" => "£".to_string(),
        "JPY" | "CNY" => "¥".to_string(),
        "KRW" => "₩".to_string(),
        "INR" => "₹".to_string(),
        "CHF" => "CHF".to_string(),
        other => other.to_string(),
    }
}

fn group_integer_digits(int_digits: &str) -> String {
    if int_digits.len() <= 3 {
        return int_digits.to_string();
    }
    let mut out = String::with_capacity(int_digits.len() + int_digits.len() / 3);
    let rem = int_digits.len() % 3;
    let first = if rem == 0 { 3 } else { rem };
    out.push_str(&int_digits[..first]);
    for chunk in int_digits.as_bytes()[first..].chunks(3) {
        out.push(',');
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
    }
    out
}

fn format_number_body(
    val: f64,
    min_frac: usize,
    max_frac: usize,
    min_int: usize,
    grouping: bool,
) -> String {
    let formatted = format!("{val:.max_frac$}");
    let (int_part, frac_part) = match formatted.split_once('.') {
        Some((i, f)) => (i, f),
        None => (formatted.as_str(), ""),
    };

    let trimmed_frac = if frac_part.len() > min_frac {
        let mut keep = frac_part.len();
        while keep > min_frac && frac_part.as_bytes()[keep - 1] == b'0' {
            keep -= 1;
        }
        &frac_part[..keep]
    } else {
        frac_part
    };

    let padded_int = if int_part.len() < min_int {
        format!("{}{int_part}", "0".repeat(min_int - int_part.len()))
    } else {
        int_part.to_string()
    };

    let grouped_int = if grouping {
        group_integer_digits(&padded_int)
    } else {
        padded_int
    };

    if trimmed_frac.is_empty() {
        grouped_int
    } else {
        format!("{grouped_int}.{trimmed_frac}")
    }
}

fn compact_scale(abs_val: f64, long: bool) -> (f64, &'static str) {
    if abs_val >= 1e12 {
        (abs_val / 1e12, if long { " trillion" } else { "T" })
    } else if abs_val >= 1e9 {
        (abs_val / 1e9, if long { " billion" } else { "B" })
    } else if abs_val >= 1e6 {
        (abs_val / 1e6, if long { " million" } else { "M" })
    } else if abs_val >= 1e3 {
        (abs_val / 1e3, if long { " thousand" } else { "K" })
    } else {
        (abs_val, "")
    }
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct NumberFormat {
    #[qjs(skip_trace)]
    opts: NumberFormatOptions,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl NumberFormat {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, locales: Opt<Value<'_>>, options: Opt<Object<'_>>) -> Result<Self> {
        let locale = parse_locale_value(locales.into_inner())?;
        let mut style = "decimal".to_string();
        let mut currency: Option<String> = None;
        let mut currency_display = "symbol".to_string();
        let mut notation = "standard".to_string();
        let mut compact_display = "short".to_string();
        let mut min_int = 1usize;
        let mut min_frac: Option<usize> = None;
        let mut max_frac: Option<usize> = None;
        let mut use_grouping = true;

        if let Some(obj) = options.into_inner() {
            if let Ok(Some(s)) = obj.get::<_, Option<String>>("style") {
                if !matches!(s.as_str(), "decimal" | "currency" | "percent" | "unit") {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid style for Intl.NumberFormat",
                    ));
                }
                style = s;
            }
            if let Ok(Some(c)) = obj.get::<_, Option<String>>("currency") {
                currency = Some(c.to_ascii_uppercase());
            }
            if let Ok(Some(cd)) = obj.get::<_, Option<String>>("currencyDisplay") {
                currency_display = cd;
            }
            if let Ok(Some(n)) = obj.get::<_, Option<String>>("notation") {
                notation = n;
            }
            if let Ok(Some(cd)) = obj.get::<_, Option<String>>("compactDisplay") {
                compact_display = cd;
            }
            if let Ok(Some(v)) = obj.get::<_, Option<usize>>("minimumIntegerDigits") {
                min_int = v.max(1);
            }
            if let Ok(Some(v)) = obj.get::<_, Option<usize>>("minimumFractionDigits") {
                min_frac = Some(v);
            }
            if let Ok(Some(v)) = obj.get::<_, Option<usize>>("maximumFractionDigits") {
                max_frac = Some(v);
            }
            if let Ok(Some(v)) = obj.get::<_, Option<bool>>("useGrouping") {
                use_grouping = v;
            }
        }

        if style == "currency" && currency.is_none() {
            return Err(Exception::throw_type(
                &ctx,
                "Currency code is required with currency style",
            ));
        }

        let (default_min_f, default_max_f) = match (style.as_str(), notation.as_str()) {
            ("currency", _) => (2, 2),
            ("percent", _) => (0, 0),
            (_, "compact") => (0, 0),
            _ => (0, 3),
        };
        let max_f = max_frac.unwrap_or_else(|| {
            let m = min_frac.unwrap_or(default_min_f);
            default_max_f.max(m)
        });
        let min_f = min_frac.unwrap_or_else(|| default_min_f.min(max_f));
        if max_f < min_f {
            return Err(Exception::throw_range(
                &ctx,
                "maximumFractionDigits cannot be less than minimumFractionDigits",
            ));
        }

        Ok(Self {
            opts: NumberFormatOptions {
                locale,
                style,
                currency,
                currency_display,
                notation,
                compact_display,
                minimum_integer_digits: min_int,
                minimum_fraction_digits: min_f,
                maximum_fraction_digits: max_f,
                use_grouping,
            },
        })
    }

    pub fn format(&self, value: Opt<Value<'_>>) -> Result<String> {
        let num = match value.0 {
            Some(v) if !v.is_undefined() => {
                if let Some(n) = v.as_number() {
                    n
                } else if let Some(b) = v.as_bool() {
                    if b {
                        1.0
                    } else {
                        0.0
                    }
                } else if let Some(s) = v.as_string() {
                    s.to_string()?.trim().parse::<f64>().unwrap_or(f64::NAN)
                } else {
                    f64::NAN
                }
            },
            _ => f64::NAN,
        };

        if num.is_nan() {
            return Ok("NaN".to_string());
        }
        let sign = if num.is_sign_negative() && num != 0.0 {
            "-"
        } else {
            ""
        };
        let mut mag = num.abs();
        if mag.is_infinite() {
            return Ok(format!("{sign}∞"));
        }

        if self.opts.style == "percent" {
            mag *= 100.0;
        }

        let (scaled, compact_suffix) = if self.opts.notation == "compact" {
            compact_scale(mag, self.opts.compact_display == "long")
        } else {
            (mag, "")
        };

        let body = format_number_body(
            scaled,
            self.opts.minimum_fraction_digits,
            self.opts.maximum_fraction_digits,
            self.opts.minimum_integer_digits,
            self.opts.use_grouping,
        );

        let result = match self.opts.style.as_str() {
            "currency" => {
                let code = self.opts.currency.as_deref().unwrap_or("USD");
                let sym = currency_symbol(code, &self.opts.currency_display);
                if sym.len() > 1 && sym.chars().all(|c| c.is_ascii_alphabetic()) {
                    format!("{sign}{sym}\u{00A0}{body}{compact_suffix}")
                } else {
                    format!("{sign}{sym}{body}{compact_suffix}")
                }
            },
            "percent" => format!("{sign}{body}{compact_suffix}%"),
            _ => format!("{sign}{body}{compact_suffix}"),
        };
        Ok(result)
    }

    #[qjs(rename = "formatToParts")]
    pub fn format_to_parts<'js>(
        &self,
        ctx: Ctx<'js>,
        value: Opt<Value<'js>>,
    ) -> Result<Array<'js>> {
        let formatted = self.format(value)?;
        let arr = Array::new(ctx.clone())?;
        let obj = Object::new(ctx)?;
        obj.set("type", "literal")?;
        obj.set("value", formatted)?;
        arr.set(0, obj)?;
        Ok(arr)
    }

    #[qjs(rename = "resolvedOptions")]
    pub fn resolved_options<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx)?;
        obj.set("locale", self.opts.locale.as_str())?;
        obj.set("numberingSystem", "latn")?;
        obj.set("style", self.opts.style.as_str())?;
        if let Some(ref c) = self.opts.currency {
            obj.set("currency", c.as_str())?;
            obj.set("currencyDisplay", self.opts.currency_display.as_str())?;
        }
        obj.set("minimumIntegerDigits", self.opts.minimum_integer_digits)?;
        obj.set("minimumFractionDigits", self.opts.minimum_fraction_digits)?;
        obj.set("maximumFractionDigits", self.opts.maximum_fraction_digits)?;
        obj.set("useGrouping", self.opts.use_grouping)?;
        obj.set("notation", self.opts.notation.as_str())?;
        Ok(obj)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Intl.NumberFormat"
    }
}
