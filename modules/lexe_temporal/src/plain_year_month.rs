// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{cmp::Ordering, str::FromStr};

use jiff::{civil::Date, Span};
use lexe_utils::result::ResultExt;
use rquickjs::{
    atom::PredefinedAtom, class::Trace, prelude::Opt, Class, Ctx, Exception, JsLifetime, Object,
    Result, Value,
};

use crate::duration::Duration;
use crate::plain_date::PlainDate;
use crate::plain_date_time::PlainDateTime;
use crate::zoned_date_time::ZonedDateTime;

#[derive(Clone, JsLifetime, Trace)]
#[rquickjs::class]
pub(crate) struct PlainYearMonth {
    #[qjs(skip_trace)]
    inner: Date,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Overflow {
    Constrain,
    Reject,
}

fn parse_overflow(ctx: &Ctx<'_>, options: &Option<Value<'_>>) -> Result<Overflow> {
    let Some(val) = options else {
        return Ok(Overflow::Constrain);
    };
    if val.is_undefined() {
        return Ok(Overflow::Constrain);
    }
    let obj = val
        .as_object()
        .or_throw_type(ctx, "options must be an object")?;
    if let Some(ov) = obj.get::<_, Option<String>>("overflow")? {
        return match ov.as_str() {
            "constrain" => Ok(Overflow::Constrain),
            "reject" => Ok(Overflow::Reject),
            _ => Err(Exception::throw_range(
                ctx,
                &format!("Invalid overflow option: {ov}"),
            )),
        };
    }
    Ok(Overflow::Constrain)
}

fn parse_month_code(ctx: &Ctx<'_>, code: &str) -> Result<i8> {
    let bytes = code.as_bytes();
    if bytes.len() != 3 || bytes[0] != b'M' {
        return Err(Exception::throw_range(
            ctx,
            &format!("Invalid monthCode: {code}"),
        ));
    }
    let m: i8 = code[1..]
        .parse()
        .map_err(|_| Exception::throw_range(ctx, &format!("Invalid monthCode: {code}")))?;
    if !(1..=12).contains(&m) {
        return Err(Exception::throw_range(
            ctx,
            &format!("Invalid monthCode: {code}"),
        ));
    }
    Ok(m)
}

fn check_calendar_opt(ctx: &Ctx<'_>, obj: &Object<'_>) -> Result<()> {
    let cal_val: Value<'_> = obj.get("calendar")?;
    if cal_val.is_undefined() {
        return Ok(());
    }
    let cal = cal_val
        .as_string()
        .and_then(|s| s.to_string().ok())
        .or_throw_type(ctx, "calendar must be a string")?;
    if cal != "iso8601" {
        return Err(Exception::throw_range(
            ctx,
            &format!("Unsupported calendar `{cal}`"),
        ));
    }
    Ok(())
}

fn parse_iso_year_month(ctx: &Ctx<'_>, raw: &str) -> Result<Date> {
    // Strip optional bracketed annotations like `[u-ca=iso8601]` or `[UTC]`
    let mut main_part = raw;
    if let Some(bracket_idx) = raw.find('[') {
        let rest = &raw[bracket_idx..];
        if let Some(ca_pos) = rest.find("u-ca=") {
            let after = &rest[ca_pos + 5..];
            let cal_name = after.split(']').next().unwrap_or("");
            if !cal_name.is_empty() && cal_name != "iso8601" {
                return Err(Exception::throw_range(
                    ctx,
                    &format!("Unsupported calendar `{cal_name}`"),
                ));
            }
        }
        main_part = &raw[..bracket_idx];
    }

    // Try full ISO date/datetime first
    if let Ok(d) = Date::from_str(main_part) {
        return Date::new(d.year(), d.month(), 1).or_throw_range(ctx, "");
    }

    // Try `YYYY-MM` or `±YYYYYY-MM`
    let (neg, unsigned) = if let Some(s) = main_part.strip_prefix('-') {
        (true, s)
    } else if let Some(s) = main_part.strip_prefix('+') {
        (false, s)
    } else {
        (false, main_part)
    };

    let (y_str, m_str) = unsigned.split_once('-').ok_or_else(|| {
        Exception::throw_range(ctx, &format!("Invalid PlainYearMonth string: {raw}"))
    })?;

    if (y_str.len() != 4 && y_str.len() != 6)
        || m_str.len() != 2
        || !y_str.bytes().all(|b| b.is_ascii_digit())
        || !m_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(Exception::throw_range(
            ctx,
            &format!("Invalid PlainYearMonth string: {raw}"),
        ));
    }

    let mut year: i32 = y_str
        .parse()
        .map_err(|_| Exception::throw_range(ctx, &format!("Invalid year in: {raw}")))?;
    if neg {
        if year == 0 {
            return Err(Exception::throw_range(ctx, "Invalid negative zero year"));
        }
        year = -year;
    }
    let year_i16 =
        i16::try_from(year).map_err(|_| Exception::throw_range(ctx, "Year out of range"))?;
    let month: i8 = m_str
        .parse()
        .map_err(|_| Exception::throw_range(ctx, &format!("Invalid month in: {raw}")))?;

    Date::new(year_i16, month, 1).or_throw_range(ctx, "")
}

fn format_iso_year_month(date: Date, calendar_name: &str) -> String {
    let year = date.year();
    let ym = if year < 0 {
        format!("-{:06}-{:02}", year.unsigned_abs(), date.month())
    } else {
        format!("{year:04}-{:02}", date.month())
    };
    match calendar_name {
        "always" => format!("{ym}-01[u-ca=iso8601]"),
        "critical" => format!("{ym}-01[!u-ca=iso8601]"),
        _ => ym,
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl PlainYearMonth {
    #[qjs(constructor)]
    fn new(
        ctx: Ctx<'_>,
        iso_year: Value<'_>,
        iso_month: Value<'_>,
        calendar: Opt<Value<'_>>,
        reference_iso_day: Opt<Value<'_>>,
    ) -> Result<Self> {
        let y_num = iso_year
            .as_number()
            .or_throw_range(&ctx, "isoYear must be a number")?;
        let m_num = iso_month
            .as_number()
            .or_throw_range(&ctx, "isoMonth must be a number")?;
        if !y_num.is_finite() || !m_num.is_finite() {
            return Err(Exception::throw_range(
                &ctx,
                "isoYear and isoMonth must be finite numbers",
            ));
        }

        if let Some(cal_val) = calendar.0 {
            if !cal_val.is_undefined() {
                let cal = cal_val
                    .as_string()
                    .and_then(|s| s.to_string().ok())
                    .or_throw_type(&ctx, "calendar must be a string")?;
                if cal != "iso8601" {
                    return Err(Exception::throw_range(
                        &ctx,
                        &format!("Unsupported calendar `{cal}`"),
                    ));
                }
            }
        }

        let ref_day = if let Some(d_val) = reference_iso_day.0 {
            if d_val.is_undefined() {
                1i8
            } else {
                let d_num = d_val
                    .as_number()
                    .or_throw_range(&ctx, "referenceISODay must be a number")?;
                if !d_num.is_finite() || d_num < 1.0 || d_num > 31.0 {
                    return Err(Exception::throw_range(&ctx, "referenceISODay out of range"));
                }
                d_num.trunc() as i8
            }
        } else {
            1i8
        };

        let y = i16::try_from(y_num.trunc() as i64)
            .map_err(|_| Exception::throw_range(&ctx, "isoYear out of range"))?;
        let m = i8::try_from(m_num.trunc() as i64)
            .map_err(|_| Exception::throw_range(&ctx, "isoMonth out of range"))?;

        // Validate ref_day fits in `(y, m)`
        let validated = Date::new(y, m, ref_day).or_throw_range(&ctx, "")?;
        let normalized =
            Date::new(validated.year(), validated.month(), 1).or_throw_range(&ctx, "")?;
        Ok(Self { inner: normalized })
    }

    #[qjs(static)]
    fn compare(ctx: Ctx<'_>, one: Value<'_>, two: Value<'_>) -> Result<i8> {
        let a = Self::from(ctx.clone(), one, Opt(None))?;
        let b = Self::from(ctx, two, Opt(None))?;
        Ok(match a.inner.cmp(&b.inner) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        })
    }

    #[qjs(static)]
    fn from(ctx: Ctx<'_>, info: Value<'_>, options: Opt<Value<'_>>) -> Result<Self> {
        let overflow = parse_overflow(&ctx, &options.0)?;
        if let Some(obj) = info.as_object() {
            if let Some(cls) = Class::<Self>::from_object(obj) {
                return Ok(cls.borrow().clone());
            }
            if let Some(cls) = Class::<PlainDate>::from_object(obj) {
                let d = cls.borrow().clone().into_inner();
                return Ok(Self::from_year_month(d.year(), d.month()));
            }
            if let Some(cls) = Class::<PlainDateTime>::from_object(obj) {
                let dt = cls.borrow().clone().into_inner();
                return Ok(Self::from_year_month(dt.year(), dt.month()));
            }
            if let Some(cls) = Class::<ZonedDateTime>::from_object(obj) {
                let zdt = cls.borrow().clone().to_plain_date().into_inner();
                return Ok(Self::from_year_month(zdt.year(), zdt.month()));
            }
            return Self::from_object(&ctx, obj, overflow);
        }

        let str = info
            .as_string()
            .and_then(|s| s.to_string().ok())
            .or_throw_type(&ctx, "Cannot convert value to PlainYearMonth")?;
        let inner = parse_iso_year_month(&ctx, &str)?;
        Ok(Self { inner })
    }

    fn add(&self, ctx: Ctx<'_>, duration: Value<'_>) -> Result<Self> {
        let dur = Duration::from_value(&ctx, &duration)?;
        let span = dur.into_inner();
        Self::validate_year_month_span(&ctx, &span)?;
        let ym_span = Span::new()
            .years(span.get_years())
            .months(span.get_months());
        let date = self.inner.checked_add(ym_span).or_throw_range(&ctx, "")?;
        Ok(Self { inner: date })
    }

    fn subtract(&self, ctx: Ctx<'_>, duration: Value<'_>) -> Result<Self> {
        let dur = Duration::from_value(&ctx, &duration)?;
        let span = dur.into_inner();
        Self::validate_year_month_span(&ctx, &span)?;
        let ym_span = Span::new()
            .years(span.get_years())
            .months(span.get_months());
        let date = self.inner.checked_sub(ym_span).or_throw_range(&ctx, "")?;
        Ok(Self { inner: date })
    }

    fn until(&self, ctx: Ctx<'_>, other: Value<'_>, options: Opt<Value<'_>>) -> Result<Duration> {
        let other_ym = Self::from(ctx.clone(), other, Opt(None))?;
        self.diff_months(&ctx, &other_ym, false, &options.0)
    }

    fn since(&self, ctx: Ctx<'_>, other: Value<'_>, options: Opt<Value<'_>>) -> Result<Duration> {
        let other_ym = Self::from(ctx.clone(), other, Opt(None))?;
        self.diff_months(&ctx, &other_ym, true, &options.0)
    }

    fn equals(&self, ctx: Ctx<'_>, other: Value<'_>) -> Result<bool> {
        let other_ym = Self::from(ctx, other, Opt(None))?;
        Ok(self.inner == other_ym.inner)
    }

    fn with(&self, ctx: Ctx<'_>, item: Value<'_>, options: Opt<Value<'_>>) -> Result<Self> {
        let overflow = parse_overflow(&ctx, &options.0)?;
        let obj = item
            .as_object()
            .or_throw_type(&ctx, "with() argument must be an object")?;

        let y_val: Value<'_> = obj.get("year")?;
        let year = if y_val.is_undefined() {
            self.inner.year()
        } else {
            let n = y_val
                .as_number()
                .or_throw_type(&ctx, "year must be a number")?;
            if !n.is_finite() {
                return Err(Exception::throw_range(&ctx, "year must be finite"));
            }
            i16::try_from(n.trunc() as i64)
                .map_err(|_| Exception::throw_range(&ctx, "year out of range"))?
        };

        let m_val: Value<'_> = obj.get("month")?;
        let mc_val: Value<'_> = obj.get("monthCode")?;

        let month = match (m_val.is_undefined(), mc_val.is_undefined()) {
            (true, true) => self.inner.month(),
            (false, true) => {
                let n = m_val
                    .as_number()
                    .or_throw_type(&ctx, "month must be a number")?;
                Self::constrain_or_reject_month(&ctx, n, overflow)?
            },
            (true, false) => {
                let s = mc_val
                    .as_string()
                    .and_then(|s| s.to_string().ok())
                    .or_throw_type(&ctx, "monthCode must be a string")?;
                parse_month_code(&ctx, &s)?
            },
            (false, false) => {
                let n = m_val
                    .as_number()
                    .or_throw_type(&ctx, "month must be a number")?;
                let m1 = Self::constrain_or_reject_month(&ctx, n, overflow)?;
                let s = mc_val
                    .as_string()
                    .and_then(|s| s.to_string().ok())
                    .or_throw_type(&ctx, "monthCode must be a string")?;
                let m2 = parse_month_code(&ctx, &s)?;
                if m1 != m2 {
                    return Err(Exception::throw_range(
                        &ctx,
                        "month and monthCode do not match",
                    ));
                }
                m1
            },
        };

        let date = Date::new(year, month, 1).or_throw_range(&ctx, "")?;
        Ok(Self { inner: date })
    }

    #[qjs(rename = "toPlainDate")]
    fn to_plain_date(&self, ctx: Ctx<'_>, item: Value<'_>) -> Result<PlainDate> {
        let obj = item
            .as_object()
            .or_throw_type(&ctx, "toPlainDate argument must be an object with day")?;
        let day_val: Value<'_> = obj.get("day")?;
        if day_val.is_undefined() {
            return Err(Exception::throw_type(&ctx, "day property is required"));
        }
        let d_num = day_val
            .as_number()
            .or_throw_type(&ctx, "day must be a number")?;
        if !d_num.is_finite() {
            return Err(Exception::throw_range(&ctx, "day must be finite"));
        }
        let day = i8::try_from(d_num.trunc() as i64)
            .map_err(|_| Exception::throw_range(&ctx, "day out of range"))?;
        let date =
            Date::new(self.inner.year(), self.inner.month(), day).or_throw_range(&ctx, "")?;
        Ok(PlainDate::new_object(date))
    }

    #[allow(clippy::inherent_to_string)]
    #[qjs(rename = PredefinedAtom::ToString)]
    fn to_string(&self, ctx: Ctx<'_>, options: Opt<Value<'_>>) -> Result<String> {
        let mut cal_name = "auto".to_string();
        if let Some(val) = options.0 {
            if !val.is_undefined() {
                let obj = val
                    .as_object()
                    .or_throw_type(&ctx, "options must be an object")?;
                if let Some(c) = obj.get::<_, Option<String>>("calendarName")? {
                    if !matches!(c.as_str(), "auto" | "always" | "never" | "critical") {
                        return Err(Exception::throw_range(
                            &ctx,
                            &format!("Invalid calendarName option: {c}"),
                        ));
                    }
                    cal_name = c;
                }
            }
        }
        Ok(format_iso_year_month(self.inner, &cal_name))
    }

    #[qjs(rename = PredefinedAtom::ToJSON)]
    fn to_json(&self) -> String {
        format_iso_year_month(self.inner, "auto")
    }

    #[qjs(rename = "toLocaleString")]
    fn to_locale_string(&self) -> String {
        format_iso_year_month(self.inner, "auto")
    }

    #[qjs(rename = "valueOf")]
    fn value_of(&self, ctx: Ctx<'_>) -> Result<()> {
        Err(Exception::throw_type(
            &ctx,
            "can't convert PlainYearMonth to primitive type",
        ))
    }

    #[qjs(get, rename = "calendarId")]
    fn calendar_id(&self) -> &'static str {
        "iso8601"
    }

    #[qjs(get)]
    fn year(&self) -> i16 {
        self.inner.year()
    }

    #[qjs(get)]
    fn month(&self) -> i8 {
        self.inner.month()
    }

    #[qjs(get, rename = "monthCode")]
    fn month_code(&self) -> String {
        format!("M{:02}", self.inner.month())
    }

    #[qjs(get, rename = "daysInMonth")]
    fn days_in_month(&self) -> i8 {
        self.inner.days_in_month()
    }

    #[qjs(get, rename = "daysInYear")]
    fn days_in_year(&self) -> i16 {
        self.inner.days_in_year()
    }

    #[qjs(get, rename = "monthsInYear")]
    fn months_in_year(&self) -> i8 {
        12
    }

    #[qjs(get, rename = "inLeapYear")]
    fn in_leap_year(&self) -> bool {
        self.inner.in_leap_year()
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    fn to_string_tag(&self) -> &'static str {
        "Temporal.PlainYearMonth"
    }
}

impl PlainYearMonth {
    pub(crate) fn from_year_month(year: i16, month: i8) -> Self {
        let inner = Date::new(year, month, 1).unwrap_or(Date::MIN);
        Self { inner }
    }

    fn constrain_or_reject_month(ctx: &Ctx<'_>, raw: f64, overflow: Overflow) -> Result<i8> {
        if !raw.is_finite() {
            return Err(Exception::throw_range(ctx, "month must be finite"));
        }
        let m = raw.trunc() as i64;
        if overflow == Overflow::Constrain {
            if m < 1 {
                return Err(Exception::throw_range(ctx, "month must be positive"));
            }
            return Ok(m.min(12) as i8);
        }
        if !(1..=12).contains(&m) {
            return Err(Exception::throw_range(ctx, "month out of range"));
        }
        Ok(m as i8)
    }

    fn from_object(ctx: &Ctx<'_>, obj: &Object<'_>, overflow: Overflow) -> Result<Self> {
        check_calendar_opt(ctx, obj)?;
        let y_val: Value<'_> = obj.get("year")?;
        if y_val.is_undefined() {
            return Err(Exception::throw_type(
                ctx,
                "year is required for PlainYearMonth",
            ));
        }
        let y_num = y_val
            .as_number()
            .or_throw_type(ctx, "year must be a number")?;
        if !y_num.is_finite() {
            return Err(Exception::throw_range(ctx, "year must be finite"));
        }
        let year = i16::try_from(y_num.trunc() as i64)
            .map_err(|_| Exception::throw_range(ctx, "year out of range"))?;

        let m_val: Value<'_> = obj.get("month")?;
        let mc_val: Value<'_> = obj.get("monthCode")?;

        let month = match (m_val.is_undefined(), mc_val.is_undefined()) {
            (true, true) => {
                return Err(Exception::throw_type(
                    ctx,
                    "month or monthCode is required for PlainYearMonth",
                ))
            },
            (false, true) => {
                let n = m_val
                    .as_number()
                    .or_throw_type(ctx, "month must be a number")?;
                Self::constrain_or_reject_month(ctx, n, overflow)?
            },
            (true, false) => {
                let s = mc_val
                    .as_string()
                    .and_then(|s| s.to_string().ok())
                    .or_throw_type(ctx, "monthCode must be a string")?;
                parse_month_code(ctx, &s)?
            },
            (false, false) => {
                let n = m_val
                    .as_number()
                    .or_throw_type(ctx, "month must be a number")?;
                let m1 = Self::constrain_or_reject_month(ctx, n, overflow)?;
                let s = mc_val
                    .as_string()
                    .and_then(|s| s.to_string().ok())
                    .or_throw_type(ctx, "monthCode must be a string")?;
                let m2 = parse_month_code(ctx, &s)?;
                if m1 != m2 {
                    return Err(Exception::throw_range(
                        ctx,
                        "month and monthCode do not match",
                    ));
                }
                m1
            },
        };

        let inner = Date::new(year, month, 1).or_throw_range(ctx, "")?;
        Ok(Self { inner })
    }

    fn validate_year_month_span(ctx: &Ctx<'_>, span: &Span) -> Result<()> {
        if span.get_weeks() != 0
            || span.get_days() != 0
            || span.get_hours() != 0
            || span.get_minutes() != 0
            || span.get_seconds() != 0
            || span.get_milliseconds() != 0
            || span.get_microseconds() != 0
            || span.get_nanoseconds() != 0
        {
            return Err(Exception::throw_range(
                ctx,
                "PlainYearMonth does not support weeks, days, or time units",
            ));
        }
        Ok(())
    }

    fn diff_months(
        &self,
        ctx: &Ctx<'_>,
        other: &Self,
        reverse: bool,
        options: &Option<Value<'_>>,
    ) -> Result<Duration> {
        let mut largest_unit = "year".to_string();
        if let Some(val) = options {
            if !val.is_undefined() {
                let obj = val
                    .as_object()
                    .or_throw_type(ctx, "options must be an object")?;
                if let Some(lu) = obj.get::<_, Option<String>>("largestUnit")? {
                    if !matches!(lu.as_str(), "auto" | "year" | "years" | "month" | "months") {
                        return Err(Exception::throw_range(
                            ctx,
                            &format!("Invalid largestUnit for PlainYearMonth: {lu}"),
                        ));
                    }
                    largest_unit = lu;
                }
            }
        }

        let (from_d, to_d) = if reverse {
            (other.inner, self.inner)
        } else {
            (self.inner, other.inner)
        };
        let total_months = (to_d.year() as i32 - from_d.year() as i32) * 12
            + (to_d.month() as i32 - from_d.month() as i32);

        let span = if largest_unit.starts_with("month") {
            Span::new().months(total_months)
        } else {
            let years = (total_months / 12) as i16;
            let months = total_months % 12;
            Span::new().years(years).months(months)
        };
        Ok(Duration::new_object(span))
    }
}
