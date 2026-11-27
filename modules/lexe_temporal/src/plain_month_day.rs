// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::str::FromStr;

use jiff::civil::Date;
use lexe_utils::result::ResultExt;
use rquickjs::{
    atom::PredefinedAtom, class::Trace, prelude::Opt, Class, Ctx, Exception, JsLifetime, Object,
    Result, Value,
};

use crate::plain_date::PlainDate;
use crate::plain_date_time::PlainDateTime;
use crate::zoned_date_time::ZonedDateTime;

const REF_ISO_YEAR: i16 = 1972;

#[derive(Clone, JsLifetime, Trace)]
#[rquickjs::class]
pub(crate) struct PlainMonthDay {
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

fn days_in_month_iso(year: i16, month: i8) -> i8 {
    Date::new(year, month, 1)
        .map(|d| d.days_in_month())
        .unwrap_or(31)
}

fn parse_iso_month_day(ctx: &Ctx<'_>, raw: &str) -> Result<Date> {
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

    if let Ok(d) = Date::from_str(main_part) {
        return Date::new(REF_ISO_YEAR, d.month(), d.day()).or_throw_range(ctx, "");
    }

    // Parse `--MM-DD` or `--MMDD` or `MM-DD`
    let stripped = main_part.strip_prefix("--").unwrap_or(main_part);
    let (m_str, d_str) = if let Some((m, d)) = stripped.split_once('-') {
        (m, d)
    } else if stripped.len() == 4 && main_part.starts_with("--") {
        (&stripped[..2], &stripped[2..])
    } else {
        return Err(Exception::throw_range(
            ctx,
            &format!("Invalid PlainMonthDay string: {raw}"),
        ));
    };

    if m_str.len() != 2
        || d_str.len() != 2
        || !m_str.bytes().all(|b| b.is_ascii_digit())
        || !d_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(Exception::throw_range(
            ctx,
            &format!("Invalid PlainMonthDay string: {raw}"),
        ));
    }

    let month: i8 = m_str
        .parse()
        .map_err(|_| Exception::throw_range(ctx, &format!("Invalid month in: {raw}")))?;
    let day: i8 = d_str
        .parse()
        .map_err(|_| Exception::throw_range(ctx, &format!("Invalid day in: {raw}")))?;

    Date::new(REF_ISO_YEAR, month, day).or_throw_range(ctx, "")
}

fn format_iso_month_day(date: Date, calendar_name: &str) -> String {
    let md = format!("{:02}-{:02}", date.month(), date.day());
    match calendar_name {
        "always" => format!("{REF_ISO_YEAR}-{md}[u-ca=iso8601]"),
        "critical" => format!("{REF_ISO_YEAR}-{md}[!u-ca=iso8601]"),
        _ => md,
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl PlainMonthDay {
    #[qjs(constructor)]
    fn new(
        ctx: Ctx<'_>,
        iso_month: Value<'_>,
        iso_day: Value<'_>,
        calendar: Opt<Value<'_>>,
        reference_iso_year: Opt<Value<'_>>,
    ) -> Result<Self> {
        let m_num = iso_month
            .as_number()
            .or_throw_range(&ctx, "isoMonth must be a number")?;
        let d_num = iso_day
            .as_number()
            .or_throw_range(&ctx, "isoDay must be a number")?;
        if !m_num.is_finite() || !d_num.is_finite() {
            return Err(Exception::throw_range(
                &ctx,
                "isoMonth and isoDay must be finite numbers",
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

        let ref_year = if let Some(y_val) = reference_iso_year.0 {
            if y_val.is_undefined() {
                REF_ISO_YEAR
            } else {
                let y_num = y_val
                    .as_number()
                    .or_throw_range(&ctx, "referenceISOYear must be a number")?;
                if !y_num.is_finite() {
                    return Err(Exception::throw_range(
                        &ctx,
                        "referenceISOYear must be finite",
                    ));
                }
                i16::try_from(y_num.trunc() as i64)
                    .map_err(|_| Exception::throw_range(&ctx, "referenceISOYear out of range"))?
            }
        } else {
            REF_ISO_YEAR
        };

        let m = i8::try_from(m_num.trunc() as i64)
            .map_err(|_| Exception::throw_range(&ctx, "isoMonth out of range"))?;
        let d = i8::try_from(d_num.trunc() as i64)
            .map_err(|_| Exception::throw_range(&ctx, "isoDay out of range"))?;

        let validated = Date::new(ref_year, m, d).or_throw_range(&ctx, "")?;
        let normalized =
            Date::new(REF_ISO_YEAR, validated.month(), validated.day()).or_throw_range(&ctx, "")?;
        Ok(Self { inner: normalized })
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
                return Ok(Self::from_month_day(d.month(), d.day()));
            }
            if let Some(cls) = Class::<PlainDateTime>::from_object(obj) {
                let dt = cls.borrow().clone().into_inner();
                return Ok(Self::from_month_day(dt.month(), dt.day()));
            }
            if let Some(cls) = Class::<ZonedDateTime>::from_object(obj) {
                let zdt = cls.borrow().clone().to_plain_date().into_inner();
                return Ok(Self::from_month_day(zdt.month(), zdt.day()));
            }
            return Self::from_object(&ctx, obj, overflow);
        }

        let str = info
            .as_string()
            .and_then(|s| s.to_string().ok())
            .or_throw_type(&ctx, "Cannot convert value to PlainMonthDay")?;
        let inner = parse_iso_month_day(&ctx, &str)?;
        Ok(Self { inner })
    }

    fn equals(&self, ctx: Ctx<'_>, other: Value<'_>) -> Result<bool> {
        let other_md = Self::from(ctx, other, Opt(None))?;
        Ok(self.inner == other_md.inner)
    }

    fn with(&self, ctx: Ctx<'_>, item: Value<'_>, options: Opt<Value<'_>>) -> Result<Self> {
        let overflow = parse_overflow(&ctx, &options.0)?;
        let obj = item
            .as_object()
            .or_throw_type(&ctx, "with() argument must be an object")?;

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

        let d_val: Value<'_> = obj.get("day")?;
        let day = if d_val.is_undefined() {
            let max_d = days_in_month_iso(REF_ISO_YEAR, month);
            if overflow == Overflow::Constrain {
                self.inner.day().min(max_d)
            } else {
                self.inner.day()
            }
        } else {
            let n = d_val
                .as_number()
                .or_throw_type(&ctx, "day must be a number")?;
            Self::constrain_or_reject_day(&ctx, n, REF_ISO_YEAR, month, overflow)?
        };

        let inner = Date::new(REF_ISO_YEAR, month, day).or_throw_range(&ctx, "")?;
        Ok(Self { inner })
    }

    #[qjs(rename = "toPlainDate")]
    fn to_plain_date(&self, ctx: Ctx<'_>, item: Value<'_>) -> Result<PlainDate> {
        let obj = item
            .as_object()
            .or_throw_type(&ctx, "toPlainDate argument must be an object with year")?;
        let y_val: Value<'_> = obj.get("year")?;
        if y_val.is_undefined() {
            return Err(Exception::throw_type(&ctx, "year property is required"));
        }
        let y_num = y_val
            .as_number()
            .or_throw_type(&ctx, "year must be a number")?;
        if !y_num.is_finite() {
            return Err(Exception::throw_range(&ctx, "year must be finite"));
        }
        let year = i16::try_from(y_num.trunc() as i64)
            .map_err(|_| Exception::throw_range(&ctx, "year out of range"))?;
        let date =
            Date::new(year, self.inner.month(), self.inner.day()).or_throw_range(&ctx, "")?;
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
        Ok(format_iso_month_day(self.inner, &cal_name))
    }

    #[qjs(rename = PredefinedAtom::ToJSON)]
    fn to_json(&self) -> String {
        format_iso_month_day(self.inner, "auto")
    }

    #[qjs(rename = "toLocaleString")]
    fn to_locale_string(&self) -> String {
        format_iso_month_day(self.inner, "auto")
    }

    #[qjs(rename = "valueOf")]
    fn value_of(&self, ctx: Ctx<'_>) -> Result<()> {
        Err(Exception::throw_type(
            &ctx,
            "can't convert PlainMonthDay to primitive type",
        ))
    }

    #[qjs(get, rename = "calendarId")]
    fn calendar_id(&self) -> &'static str {
        "iso8601"
    }

    #[qjs(get, rename = "monthCode")]
    fn month_code(&self) -> String {
        format!("M{:02}", self.inner.month())
    }

    #[qjs(get)]
    fn day(&self) -> i8 {
        self.inner.day()
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    fn to_string_tag(&self) -> &'static str {
        "Temporal.PlainMonthDay"
    }
}

impl PlainMonthDay {
    pub(crate) fn from_month_day(month: i8, day: i8) -> Self {
        let inner = Date::new(REF_ISO_YEAR, month, day).unwrap_or(Date::MIN);
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

    fn constrain_or_reject_day(
        ctx: &Ctx<'_>,
        raw: f64,
        ref_year: i16,
        month: i8,
        overflow: Overflow,
    ) -> Result<i8> {
        if !raw.is_finite() {
            return Err(Exception::throw_range(ctx, "day must be finite"));
        }
        let d = raw.trunc() as i64;
        let max_d = days_in_month_iso(ref_year, month) as i64;
        if overflow == Overflow::Constrain {
            if d < 1 {
                return Err(Exception::throw_range(ctx, "day must be positive"));
            }
            return Ok(d.min(max_d) as i8);
        }
        if d < 1 || d > max_d {
            return Err(Exception::throw_range(ctx, "day out of range"));
        }
        Ok(d as i8)
    }

    fn from_object(ctx: &Ctx<'_>, obj: &Object<'_>, overflow: Overflow) -> Result<Self> {
        check_calendar_opt(ctx, obj)?;
        let y_val: Value<'_> = obj.get("year")?;
        let check_year = if y_val.is_undefined() {
            REF_ISO_YEAR
        } else {
            let yn = y_val
                .as_number()
                .or_throw_type(ctx, "year must be a number")?;
            if !yn.is_finite() {
                return Err(Exception::throw_range(ctx, "year must be finite"));
            }
            i16::try_from(yn.trunc() as i64)
                .map_err(|_| Exception::throw_range(ctx, "year out of range"))?
        };

        let m_val: Value<'_> = obj.get("month")?;
        let mc_val: Value<'_> = obj.get("monthCode")?;

        let month = match (m_val.is_undefined(), mc_val.is_undefined()) {
            (true, true) => {
                return Err(Exception::throw_type(
                    ctx,
                    "month or monthCode is required for PlainMonthDay",
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

        let d_val: Value<'_> = obj.get("day")?;
        if d_val.is_undefined() {
            return Err(Exception::throw_type(
                ctx,
                "day is required for PlainMonthDay",
            ));
        }
        let d_num = d_val
            .as_number()
            .or_throw_type(ctx, "day must be a number")?;
        let day = Self::constrain_or_reject_day(ctx, d_num, check_year, month, overflow)?;

        let inner = Date::new(REF_ISO_YEAR, month, day).or_throw_range(ctx, "")?;
        Ok(Self { inner })
    }
}
