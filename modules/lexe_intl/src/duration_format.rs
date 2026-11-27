// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{atom::PredefinedAtom, function::Opt, Array, Ctx, Exception, Object, Result, Value};

use crate::locale::{parse_bcp47_tag, resolve_locale_arg, supported_locales_of_impl};
use crate::plural_rules::select_cardinal_for_locale;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DurationStyle {
    Long,
    Short,
    Narrow,
    Digital,
}

impl DurationStyle {
    fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
            Self::Narrow => "narrow",
            Self::Digital => "digital",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnitStyle {
    Long,
    Short,
    Narrow,
    Numeric,
    TwoDigit,
}

impl UnitStyle {
    fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
            Self::Narrow => "narrow",
            Self::Numeric => "numeric",
            Self::TwoDigit => "2-digit",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnitDisplay {
    Auto,
    Always,
}

impl UnitDisplay {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
        }
    }
}

#[derive(Clone, Copy)]
struct UnitConfig {
    style: UnitStyle,
    display: UnitDisplay,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DurationUnit {
    Years,
    Months,
    Weeks,
    Days,
    Hours,
    Minutes,
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

impl DurationUnit {
    const ALL: [Self; 10] = [
        Self::Years,
        Self::Months,
        Self::Weeks,
        Self::Days,
        Self::Hours,
        Self::Minutes,
        Self::Seconds,
        Self::Milliseconds,
        Self::Microseconds,
        Self::Nanoseconds,
    ];

    fn prop_name(self) -> &'static str {
        match self {
            Self::Years => "years",
            Self::Months => "months",
            Self::Weeks => "weeks",
            Self::Days => "days",
            Self::Hours => "hours",
            Self::Minutes => "minutes",
            Self::Seconds => "seconds",
            Self::Milliseconds => "milliseconds",
            Self::Microseconds => "microseconds",
            Self::Nanoseconds => "nanoseconds",
        }
    }

    fn display_prop_name(self) -> &'static str {
        match self {
            Self::Years => "yearsDisplay",
            Self::Months => "monthsDisplay",
            Self::Weeks => "weeksDisplay",
            Self::Days => "daysDisplay",
            Self::Hours => "hoursDisplay",
            Self::Minutes => "minutesDisplay",
            Self::Seconds => "secondsDisplay",
            Self::Milliseconds => "millisecondsDisplay",
            Self::Microseconds => "microsecondsDisplay",
            Self::Nanoseconds => "nanosecondsDisplay",
        }
    }

    fn singular_name(self) -> &'static str {
        match self {
            Self::Years => "year",
            Self::Months => "month",
            Self::Weeks => "week",
            Self::Days => "day",
            Self::Hours => "hour",
            Self::Minutes => "minute",
            Self::Seconds => "second",
            Self::Milliseconds => "millisecond",
            Self::Microseconds => "microsecond",
            Self::Nanoseconds => "nanosecond",
        }
    }
}

fn parse_unit_option(
    ctx: &Ctx<'_>,
    opts: Option<&Object<'_>>,
    unit: DurationUnit,
    base_style: DurationStyle,
    prev_style: Option<UnitStyle>,
) -> Result<UnitConfig> {
    let default_style = match base_style {
        DurationStyle::Long => UnitStyle::Long,
        DurationStyle::Short => UnitStyle::Short,
        DurationStyle::Narrow => UnitStyle::Narrow,
        DurationStyle::Digital => match unit {
            DurationUnit::Years
            | DurationUnit::Months
            | DurationUnit::Weeks
            | DurationUnit::Days => UnitStyle::Short,
            DurationUnit::Hours
            | DurationUnit::Minutes
            | DurationUnit::Seconds
            | DurationUnit::Milliseconds
            | DurationUnit::Microseconds
            | DurationUnit::Nanoseconds => UnitStyle::Numeric,
        },
    };

    let mut style = default_style;
    let mut explicit_style = false;
    if let Some(obj) = opts {
        if let Some(s) = obj.get::<_, Option<String>>(unit.prop_name())? {
            explicit_style = true;
            style = match (unit, s.as_str()) {
                (_, "long") => UnitStyle::Long,
                (_, "short") => UnitStyle::Short,
                (_, "narrow") => UnitStyle::Narrow,
                (
                    DurationUnit::Hours
                    | DurationUnit::Minutes
                    | DurationUnit::Seconds
                    | DurationUnit::Milliseconds
                    | DurationUnit::Microseconds
                    | DurationUnit::Nanoseconds,
                    "numeric",
                ) => UnitStyle::Numeric,
                (
                    DurationUnit::Hours | DurationUnit::Minutes | DurationUnit::Seconds,
                    "2-digit",
                ) => UnitStyle::TwoDigit,
                _ => {
                    return Err(Exception::throw_range(
                        ctx,
                        &format!("Invalid style `{s}` for `{}`", unit.prop_name()),
                    ))
                },
            };
        }
    }

    if !explicit_style
        && matches!(prev_style, Some(UnitStyle::Numeric | UnitStyle::TwoDigit))
        && matches!(unit, DurationUnit::Minutes | DurationUnit::Seconds)
    {
        style = UnitStyle::TwoDigit;
    }

    let default_display = if style == UnitStyle::Numeric || style == UnitStyle::TwoDigit {
        match unit {
            DurationUnit::Minutes | DurationUnit::Seconds
                if base_style == DurationStyle::Digital =>
            {
                UnitDisplay::Always
            },
            _ => UnitDisplay::Auto,
        }
    } else {
        UnitDisplay::Auto
    };

    let mut display = default_display;
    if let Some(obj) = opts {
        if let Some(d) = obj.get::<_, Option<String>>(unit.display_prop_name())? {
            display = match d.as_str() {
                "auto" => UnitDisplay::Auto,
                "always" => UnitDisplay::Always,
                _ => {
                    return Err(Exception::throw_range(
                        ctx,
                        &format!("Invalid display `{d}` for `{}`", unit.display_prop_name()),
                    ))
                },
            };
        }
    }

    Ok(UnitConfig { style, display })
}

fn unit_label(lang: &str, unit: DurationUnit, style: UnitStyle, singular: bool) -> &'static str {
    if lang == "zh" {
        return match (unit, style) {
            (DurationUnit::Years, _) => "年",
            (DurationUnit::Months, _) => "个月",
            (DurationUnit::Weeks, _) => "周",
            (DurationUnit::Days, _) => "天",
            (DurationUnit::Hours, UnitStyle::Narrow) => "小时",
            (DurationUnit::Hours, _) => "小时",
            (DurationUnit::Minutes, UnitStyle::Narrow) => "分",
            (DurationUnit::Minutes, _) => "分钟",
            (DurationUnit::Seconds, UnitStyle::Narrow) => "秒",
            (DurationUnit::Seconds, _) => "秒钟",
            (DurationUnit::Milliseconds, _) => "毫秒",
            (DurationUnit::Microseconds, _) => "微秒",
            (DurationUnit::Nanoseconds, _) => "纳秒",
        };
    }

    match (unit, style, singular) {
        (DurationUnit::Years, UnitStyle::Long, true) => " year",
        (DurationUnit::Years, UnitStyle::Long, false) => " years",
        (DurationUnit::Years, UnitStyle::Short, true) => " yr",
        (DurationUnit::Years, UnitStyle::Short, false) => " yrs",
        (DurationUnit::Years, _, _) => "y",

        (DurationUnit::Months, UnitStyle::Long, true) => " month",
        (DurationUnit::Months, UnitStyle::Long, false) => " months",
        (DurationUnit::Months, UnitStyle::Short, true) => " mth",
        (DurationUnit::Months, UnitStyle::Short, false) => " mths",
        (DurationUnit::Months, _, _) => "m",

        (DurationUnit::Weeks, UnitStyle::Long, true) => " week",
        (DurationUnit::Weeks, UnitStyle::Long, false) => " weeks",
        (DurationUnit::Weeks, UnitStyle::Short, true) => " wk",
        (DurationUnit::Weeks, UnitStyle::Short, false) => " wks",
        (DurationUnit::Weeks, _, _) => "w",

        (DurationUnit::Days, UnitStyle::Long, true) => " day",
        (DurationUnit::Days, UnitStyle::Long, false) => " days",
        (DurationUnit::Days, UnitStyle::Short, true) => " day",
        (DurationUnit::Days, UnitStyle::Short, false) => " days",
        (DurationUnit::Days, _, _) => "d",

        (DurationUnit::Hours, UnitStyle::Long, true) => " hour",
        (DurationUnit::Hours, UnitStyle::Long, false) => " hours",
        (DurationUnit::Hours, UnitStyle::Short, true) => " hr",
        (DurationUnit::Hours, UnitStyle::Short, false) => " hrs",
        (DurationUnit::Hours, _, _) => "h",

        (DurationUnit::Minutes, UnitStyle::Long, true) => " minute",
        (DurationUnit::Minutes, UnitStyle::Long, false) => " minutes",
        (DurationUnit::Minutes, UnitStyle::Short, _) => " min",
        (DurationUnit::Minutes, _, _) => "m",

        (DurationUnit::Seconds, UnitStyle::Long, true) => " second",
        (DurationUnit::Seconds, UnitStyle::Long, false) => " seconds",
        (DurationUnit::Seconds, UnitStyle::Short, _) => " sec",
        (DurationUnit::Seconds, _, _) => "s",

        (DurationUnit::Milliseconds, UnitStyle::Long, true) => " millisecond",
        (DurationUnit::Milliseconds, UnitStyle::Long, false) => " milliseconds",
        (DurationUnit::Milliseconds, UnitStyle::Short, _) => " ms",
        (DurationUnit::Milliseconds, _, _) => "ms",

        (DurationUnit::Microseconds, UnitStyle::Long, true) => " microsecond",
        (DurationUnit::Microseconds, UnitStyle::Long, false) => " microseconds",
        (DurationUnit::Microseconds, _, _) => " μs",

        (DurationUnit::Nanoseconds, UnitStyle::Long, true) => " nanosecond",
        (DurationUnit::Nanoseconds, UnitStyle::Long, false) => " nanoseconds",
        (DurationUnit::Nanoseconds, _, _) => " ns",
    }
}

type DurationPart = (&'static str, String, Option<&'static str>);

fn push_grouped_int(
    parts: &mut Vec<DurationPart>,
    val: i64,
    unit_name: &'static str,
    pad_two: bool,
    grouping: bool,
) {
    let s = if pad_two && val < 10 {
        format!("0{val}")
    } else {
        val.to_string()
    };
    if !grouping || s.len() <= 3 {
        parts.push(("integer", s, Some(unit_name)));
        return;
    }
    let rem = s.len() % 3;
    let first = if rem == 0 { 3 } else { rem };
    parts.push(("integer", s[..first].to_string(), Some(unit_name)));
    for chunk in s.as_bytes()[first..].chunks(3) {
        parts.push(("group", ",".to_string(), Some(unit_name)));
        parts.push((
            "integer",
            std::str::from_utf8(chunk).unwrap_or("").to_string(),
            Some(unit_name),
        ));
    }
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct DurationFormat {
    #[qjs(skip_trace)]
    locale: String,
    #[qjs(skip_trace)]
    language: String,
    #[qjs(skip_trace)]
    style: DurationStyle,
    #[qjs(skip_trace)]
    configs: [UnitConfig; 10],
    #[qjs(skip_trace)]
    fractional_digits: Option<usize>,
}

impl DurationFormat {
    fn extract_duration(&self, ctx: &Ctx<'_>, val: &Value<'_>) -> Result<[f64; 10]> {
        let obj = val
            .as_object()
            .ok_or_else(|| Exception::throw_type(ctx, "Duration argument must be an object"))?;
        let mut values = [0.0f64; 10];
        let mut any_present = false;
        let mut sign: i8 = 0;

        for (i, unit) in DurationUnit::ALL.iter().enumerate() {
            let prop: Value<'_> = obj.get(unit.prop_name())?;
            if prop.is_undefined() {
                continue;
            }
            any_present = true;
            let num = prop.as_number().ok_or_else(|| {
                Exception::throw_type(
                    ctx,
                    &format!("Duration field `{}` must be a number", unit.prop_name()),
                )
            })?;
            if !num.is_finite() || num.fract() != 0.0 {
                return Err(Exception::throw_range(
                    ctx,
                    &format!(
                        "Duration field `{}` must be a finite integer",
                        unit.prop_name()
                    ),
                ));
            }
            if num != 0.0 {
                let s = if num < 0.0 { -1 } else { 1 };
                if sign != 0 && sign != s {
                    return Err(Exception::throw_range(
                        ctx,
                        "Duration fields must not have mixed signs",
                    ));
                }
                sign = s;
            }
            values[i] = num;
        }

        if !any_present {
            return Err(Exception::throw_type(
                ctx,
                "Duration object must contain at least one duration unit field",
            ));
        }

        Ok(values)
    }

    fn build_parts(
        &self,
        ctx: &Ctx<'_>,
        val: &Value<'_>,
    ) -> Result<Vec<(&'static str, String, Option<&'static str>)>> {
        let mut vals = self.extract_duration(ctx, val)?;

        // Determine if digital fractional collapse applies: seconds numeric/2-digit with sub-second units
        let sec_cfg = self.configs[6];
        let ms_cfg = self.configs[7];
        let us_cfg = self.configs[8];
        let ns_cfg = self.configs[9];

        let collapse_subseconds = matches!(sec_cfg.style, UnitStyle::Numeric | UnitStyle::TwoDigit)
            && ms_cfg.style == UnitStyle::Numeric
            && us_cfg.style == UnitStyle::Numeric
            && ns_cfg.style == UnitStyle::Numeric
            && (vals[7] != 0.0
                || vals[8] != 0.0
                || vals[9] != 0.0
                || self.fractional_digits.unwrap_or(0) > 0);

        let mut folded_frac_ns: i128 = 0;
        if collapse_subseconds {
            folded_frac_ns =
                (vals[7] as i128) * 1_000_000 + (vals[8] as i128) * 1_000 + (vals[9] as i128);
            vals[7] = 0.0;
            vals[8] = 0.0;
            vals[9] = 0.0;
        }

        let is_negative = vals.iter().any(|&v| v < 0.0) || folded_frac_ns < 0;
        let mut segments: Vec<(DurationUnit, UnitConfig, Vec<DurationPart>)> = Vec::new();

        for (idx, unit) in DurationUnit::ALL.iter().enumerate() {
            if collapse_subseconds && idx >= 7 {
                continue;
            }
            let cfg = self.configs[idx];
            let v = vals[idx].abs();
            let has_frac_seconds = idx == 6 && collapse_subseconds && folded_frac_ns != 0;

            if v == 0.0 && !has_frac_seconds && cfg.display == UnitDisplay::Auto {
                // In digital mode, if hours are displayed, minutes/seconds are typically shown too
                let force_digital_sub =
                    matches!(unit, DurationUnit::Minutes | DurationUnit::Seconds)
                        && matches!(cfg.style, UnitStyle::Numeric | UnitStyle::TwoDigit)
                        && segments.iter().any(|(u, c, _)| {
                            *u == DurationUnit::Hours
                                && matches!(c.style, UnitStyle::Numeric | UnitStyle::TwoDigit)
                        });
                if !force_digital_sub {
                    continue;
                }
            }

            let unit_name = unit.singular_name();
            let mut unit_parts = Vec::new();
            let mut int_val = v as i64;

            if idx == 6 && collapse_subseconds {
                let extra_secs = (folded_frac_ns.abs() / 1_000_000_000) as i64;
                int_val += extra_secs;
            }

            let pad_two = cfg.style == UnitStyle::TwoDigit
                || (matches!(unit, DurationUnit::Minutes | DurationUnit::Seconds)
                    && matches!(cfg.style, UnitStyle::Numeric)
                    && segments.iter().any(|(u, c, _)| {
                        *u == DurationUnit::Hours
                            && matches!(c.style, UnitStyle::Numeric | UnitStyle::TwoDigit)
                    }));

            push_grouped_int(
                &mut unit_parts,
                int_val,
                unit_name,
                pad_two,
                cfg.style != UnitStyle::TwoDigit && cfg.style != UnitStyle::Numeric,
            );

            if idx == 6 && collapse_subseconds {
                let rem_ns = (folded_frac_ns.abs() % 1_000_000_000) as u64;
                let max_digits = self.fractional_digits.unwrap_or(9);
                if max_digits > 0 && (rem_ns > 0 || self.fractional_digits.is_some()) {
                    let full = format!("{rem_ns:09}");
                    let slice = &full[..max_digits.min(9)];
                    let final_frac = if self.fractional_digits.is_some() {
                        slice
                    } else {
                        slice.trim_end_matches('0')
                    };
                    if !final_frac.is_empty() {
                        unit_parts.push(("decimal", ".".to_string(), Some(unit_name)));
                        unit_parts.push(("fraction", final_frac.to_string(), Some(unit_name)));
                    }
                }
            }

            if matches!(
                cfg.style,
                UnitStyle::Long | UnitStyle::Short | UnitStyle::Narrow
            ) {
                let singular = select_cardinal_for_locale(&self.locale, int_val as f64) == "one";
                let label = unit_label(&self.language, *unit, cfg.style, singular);
                unit_parts.push(("unit", label.to_string(), Some(unit_name)));
            }

            segments.push((*unit, cfg, unit_parts));
        }

        let mut out: Vec<(&'static str, String, Option<&'static str>)> = Vec::new();
        let len = segments.len();
        for (idx, (unit, cfg, mut seg_parts)) in segments.into_iter().enumerate() {
            if idx == 0 && is_negative {
                out.push(("minusSign", "-".to_string(), Some(unit.singular_name())));
            } else if idx > 0 {
                let prev_numeric = matches!(cfg.style, UnitStyle::Numeric | UnitStyle::TwoDigit);
                let sep = if prev_numeric {
                    ":"
                } else if self.style == DurationStyle::Long && self.language == "en" {
                    if idx + 1 == len && len > 2 {
                        ", and "
                    } else if idx + 1 == len && len == 2 {
                        " and "
                    } else {
                        ", "
                    }
                } else if self.style == DurationStyle::Narrow && self.language == "zh" {
                    ""
                } else {
                    ", "
                };
                if !sep.is_empty() {
                    out.push(("literal", sep.to_string(), None));
                }
            }
            out.append(&mut seg_parts);
        }

        Ok(out)
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl DurationFormat {
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

        let opts_opt = options.into_inner();
        let mut style = DurationStyle::Short;
        let mut fractional_digits = None;

        if let Some(ref opts) = opts_opt {
            if let Some(lm) = opts.get::<_, Option<String>>("localeMatcher")? {
                if lm != "lookup" && lm != "best fit" {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid localeMatcher for Intl.DurationFormat",
                    ));
                }
            }
            if let Some(s) = opts.get::<_, Option<String>>("style")? {
                style = match s.as_str() {
                    "long" => DurationStyle::Long,
                    "short" => DurationStyle::Short,
                    "narrow" => DurationStyle::Narrow,
                    "digital" => DurationStyle::Digital,
                    _ => {
                        return Err(Exception::throw_range(
                            &ctx,
                            "Invalid style option for Intl.DurationFormat",
                        ))
                    },
                };
            }
            let fd_val: Value<'_> = opts.get("fractionalDigits")?;
            if !fd_val.is_undefined() {
                let fd = fd_val.as_number().ok_or_else(|| {
                    Exception::throw_range(&ctx, "Invalid fractionalDigits for Intl.DurationFormat")
                })?;
                if !fd.is_finite() || !(0.0..=9.0).contains(&fd) {
                    return Err(Exception::throw_range(
                        &ctx,
                        "fractionalDigits must be between 0 and 9",
                    ));
                }
                fractional_digits = Some(fd.floor() as usize);
            }
        }

        let mut configs = [UnitConfig {
            style: UnitStyle::Short,
            display: UnitDisplay::Auto,
        }; 10];
        let mut prev_style = None;
        for (i, unit) in DurationUnit::ALL.iter().enumerate() {
            let cfg = parse_unit_option(&ctx, opts_opt.as_ref(), *unit, style, prev_style)?;
            prev_style = Some(cfg.style);
            configs[i] = cfg;
        }

        Ok(Self {
            locale,
            language,
            style,
            configs,
            fractional_digits,
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

    pub fn format(&self, ctx: Ctx<'_>, duration: Value<'_>) -> Result<String> {
        let parts = self.build_parts(&ctx, &duration)?;
        let mut out = String::new();
        for (_, text, _) in parts {
            out.push_str(&text);
        }
        Ok(out)
    }

    #[qjs(rename = "formatToParts")]
    pub fn format_to_parts<'js>(&self, ctx: Ctx<'js>, duration: Value<'_>) -> Result<Array<'js>> {
        let parts = self.build_parts(&ctx, &duration)?;
        let arr = Array::new(ctx.clone())?;
        for (i, (ty, text, unit_opt)) in parts.into_iter().enumerate() {
            let obj = Object::new(ctx.clone())?;
            obj.set("type", ty)?;
            obj.set("value", text)?;
            if let Some(u) = unit_opt {
                obj.set("unit", u)?;
            }
            arr.set(i, obj)?;
        }
        Ok(arr)
    }

    #[qjs(rename = "resolvedOptions")]
    pub fn resolved_options<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx)?;
        obj.set("locale", self.locale.as_str())?;
        obj.set("numberingSystem", "latn")?;
        obj.set("style", self.style.as_str())?;
        for (i, unit) in DurationUnit::ALL.iter().enumerate() {
            obj.set(unit.prop_name(), self.configs[i].style.as_str())?;
            obj.set(unit.display_prop_name(), self.configs[i].display.as_str())?;
        }
        if let Some(fd) = self.fractional_digits {
            obj.set("fractionalDigits", fd)?;
        }
        Ok(obj)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Intl.DurationFormat"
    }
}
