// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{atom::PredefinedAtom, function::Opt, Array, Ctx, Exception, Object, Result, Value};

use crate::locale::{parse_bcp47_tag, resolve_locale_arg, supported_locales_of_impl};
use crate::plural_rules::select_cardinal_for_locale;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RtfStyle {
    Long,
    Short,
    Narrow,
}

impl RtfStyle {
    fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
            Self::Narrow => "narrow",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RtfNumeric {
    Always,
    Auto,
}

impl RtfNumeric {
    fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Auto => "auto",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RtfUnit {
    Year,
    Quarter,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
}

impl RtfUnit {
    pub(crate) fn from_str(s: &str) -> Option<Self> {
        match s {
            "year" | "years" => Some(Self::Year),
            "quarter" | "quarters" => Some(Self::Quarter),
            "month" | "months" => Some(Self::Month),
            "week" | "weeks" => Some(Self::Week),
            "day" | "days" => Some(Self::Day),
            "hour" | "hours" => Some(Self::Hour),
            "minute" | "minutes" => Some(Self::Minute),
            "second" | "seconds" => Some(Self::Second),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Year => "year",
            Self::Quarter => "quarter",
            Self::Month => "month",
            Self::Week => "week",
            Self::Day => "day",
            Self::Hour => "hour",
            Self::Minute => "minute",
            Self::Second => "second",
        }
    }
}

fn auto_phrase(lang: &str, unit: RtfUnit, val: i64, style: RtfStyle) -> Option<&'static str> {
    match lang {
        "zh" => match (unit, val) {
            (RtfUnit::Year, -1) => Some("去年"),
            (RtfUnit::Year, 0) => Some("今年"),
            (RtfUnit::Year, 1) => Some("明年"),
            (RtfUnit::Quarter, -1) => Some("上季度"),
            (RtfUnit::Quarter, 0) => Some("本季度"),
            (RtfUnit::Quarter, 1) => Some("下季度"),
            (RtfUnit::Month, -1) => Some("上个月"),
            (RtfUnit::Month, 0) => Some("本月"),
            (RtfUnit::Month, 1) => Some("下个月"),
            (RtfUnit::Week, -1) => Some("上周"),
            (RtfUnit::Week, 0) => Some("本周"),
            (RtfUnit::Week, 1) => Some("下周"),
            (RtfUnit::Day, -1) => Some("昨天"),
            (RtfUnit::Day, 0) => Some("今天"),
            (RtfUnit::Day, 1) => Some("明天"),
            (RtfUnit::Hour, 0) => Some("这一时间 / 1小时内"),
            (RtfUnit::Minute, 0) => Some("此刻"),
            (RtfUnit::Second, 0) => Some("现在"),
            _ => None,
        },
        "ja" => match (unit, val) {
            (RtfUnit::Year, -1) => Some("昨年"),
            (RtfUnit::Year, 0) => Some("今年"),
            (RtfUnit::Year, 1) => Some("来年"),
            (RtfUnit::Month, -1) => Some("先月"),
            (RtfUnit::Month, 0) => Some("今月"),
            (RtfUnit::Month, 1) => Some("来月"),
            (RtfUnit::Week, -1) => Some("先週"),
            (RtfUnit::Week, 0) => Some("今週"),
            (RtfUnit::Week, 1) => Some("来週"),
            (RtfUnit::Day, -1) => Some("昨日"),
            (RtfUnit::Day, 0) => Some("今日"),
            (RtfUnit::Day, 1) => Some("明日"),
            (RtfUnit::Second, 0) => Some("今"),
            _ => None,
        },
        "ko" => match (unit, val) {
            (RtfUnit::Year, -1) => Some("작년"),
            (RtfUnit::Year, 0) => Some("올해"),
            (RtfUnit::Year, 1) => Some("내년"),
            (RtfUnit::Month, -1) => Some("지난달"),
            (RtfUnit::Month, 0) => Some("이번 달"),
            (RtfUnit::Month, 1) => Some("다음 달"),
            (RtfUnit::Week, -1) => Some("지난주"),
            (RtfUnit::Week, 0) => Some("이번 주"),
            (RtfUnit::Week, 1) => Some("다음 주"),
            (RtfUnit::Day, -1) => Some("어제"),
            (RtfUnit::Day, 0) => Some("오늘"),
            (RtfUnit::Day, 1) => Some("내일"),
            (RtfUnit::Second, 0) => Some("지금"),
            _ => None,
        },
        "fr" => match (unit, val) {
            (RtfUnit::Year, -1) => Some("l’année dernière"),
            (RtfUnit::Year, 0) => Some("cette année"),
            (RtfUnit::Year, 1) => Some("l’année prochaine"),
            (RtfUnit::Month, -1) => Some("le mois dernier"),
            (RtfUnit::Month, 0) => Some("ce mois-ci"),
            (RtfUnit::Month, 1) => Some("le mois prochain"),
            (RtfUnit::Week, -1) => Some("la semaine dernière"),
            (RtfUnit::Week, 0) => Some("cette semaine"),
            (RtfUnit::Week, 1) => Some("la semaine prochaine"),
            (RtfUnit::Day, -1) => Some("hier"),
            (RtfUnit::Day, 0) => Some("aujourd’hui"),
            (RtfUnit::Day, 1) => Some("demain"),
            (RtfUnit::Second, 0) => Some("maintenant"),
            _ => None,
        },
        "de" => match (unit, val) {
            (RtfUnit::Year, -1) => Some("letztes Jahr"),
            (RtfUnit::Year, 0) => Some("dieses Jahr"),
            (RtfUnit::Year, 1) => Some("nächstes Jahr"),
            (RtfUnit::Month, -1) => Some("letzten Monat"),
            (RtfUnit::Month, 0) => Some("diesen Monat"),
            (RtfUnit::Month, 1) => Some("nächsten Monat"),
            (RtfUnit::Week, -1) => Some("letzte Woche"),
            (RtfUnit::Week, 0) => Some("diese Woche"),
            (RtfUnit::Week, 1) => Some("nächste Woche"),
            (RtfUnit::Day, -1) => Some("gestern"),
            (RtfUnit::Day, 0) => Some("heute"),
            (RtfUnit::Day, 1) => Some("morgen"),
            (RtfUnit::Second, 0) => Some("jetzt"),
            _ => None,
        },
        "es" => match (unit, val) {
            (RtfUnit::Year, -1) => Some("el año pasado"),
            (RtfUnit::Year, 0) => Some("este año"),
            (RtfUnit::Year, 1) => Some("el próximo año"),
            (RtfUnit::Month, -1) => Some("el mes pasado"),
            (RtfUnit::Month, 0) => Some("este mes"),
            (RtfUnit::Month, 1) => Some("el próximo mes"),
            (RtfUnit::Week, -1) => Some("la semana pasada"),
            (RtfUnit::Week, 0) => Some("esta semana"),
            (RtfUnit::Week, 1) => Some("la próxima semana"),
            (RtfUnit::Day, -1) => Some("ayer"),
            (RtfUnit::Day, 0) => Some("hoy"),
            (RtfUnit::Day, 1) => Some("mañana"),
            (RtfUnit::Second, 0) => Some("ahora"),
            _ => None,
        },
        _ => match (unit, val, style) {
            (RtfUnit::Year, -1, RtfStyle::Long) => Some("last year"),
            (RtfUnit::Year, -1, _) => Some("last yr."),
            (RtfUnit::Year, 0, RtfStyle::Long) => Some("this year"),
            (RtfUnit::Year, 0, _) => Some("this yr."),
            (RtfUnit::Year, 1, RtfStyle::Long) => Some("next year"),
            (RtfUnit::Year, 1, _) => Some("next yr."),
            (RtfUnit::Quarter, -1, RtfStyle::Long) => Some("last quarter"),
            (RtfUnit::Quarter, -1, _) => Some("last qtr."),
            (RtfUnit::Quarter, 0, RtfStyle::Long) => Some("this quarter"),
            (RtfUnit::Quarter, 0, _) => Some("this qtr."),
            (RtfUnit::Quarter, 1, RtfStyle::Long) => Some("next quarter"),
            (RtfUnit::Quarter, 1, _) => Some("next qtr."),
            (RtfUnit::Month, -1, RtfStyle::Long) => Some("last month"),
            (RtfUnit::Month, -1, _) => Some("last mo."),
            (RtfUnit::Month, 0, RtfStyle::Long) => Some("this month"),
            (RtfUnit::Month, 0, _) => Some("this mo."),
            (RtfUnit::Month, 1, RtfStyle::Long) => Some("next month"),
            (RtfUnit::Month, 1, _) => Some("next mo."),
            (RtfUnit::Week, -1, RtfStyle::Long) => Some("last week"),
            (RtfUnit::Week, -1, _) => Some("last wk."),
            (RtfUnit::Week, 0, RtfStyle::Long) => Some("this week"),
            (RtfUnit::Week, 0, _) => Some("this wk."),
            (RtfUnit::Week, 1, RtfStyle::Long) => Some("next week"),
            (RtfUnit::Week, 1, _) => Some("next wk."),
            (RtfUnit::Day, -1, _) => Some("yesterday"),
            (RtfUnit::Day, 0, _) => Some("today"),
            (RtfUnit::Day, 1, _) => Some("tomorrow"),
            (RtfUnit::Hour, 0, _) => Some("this hour"),
            (RtfUnit::Minute, 0, _) => Some("this minute"),
            (RtfUnit::Second, 0, _) => Some("now"),
            _ => None,
        },
    }
}

fn format_number_parts(
    abs_val: f64,
    unit_str: &'static str,
) -> Vec<(&'static str, String, Option<&'static str>)> {
    let raw = if abs_val.fract() == 0.0 && abs_val <= i64::MAX as f64 {
        format!("{}", abs_val as i64)
    } else {
        let mut s = format!("{abs_val:.3}");
        if s.contains('.') {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
        }
        s
    };

    let (int_str, frac_str) = match raw.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (raw.as_str(), None),
    };

    let mut out = Vec::new();
    if int_str.len() <= 3 {
        out.push(("integer", int_str.to_string(), Some(unit_str)));
    } else {
        let rem = int_str.len() % 3;
        let first = if rem == 0 { 3 } else { rem };
        out.push(("integer", int_str[..first].to_string(), Some(unit_str)));
        for chunk in int_str.as_bytes()[first..].chunks(3) {
            out.push(("group", ",".to_string(), Some(unit_str)));
            out.push((
                "integer",
                std::str::from_utf8(chunk).unwrap_or("").to_string(),
                Some(unit_str),
            ));
        }
    }
    if let Some(frac) = frac_str {
        out.push(("decimal", ".".to_string(), Some(unit_str)));
        out.push(("fraction", frac.to_string(), Some(unit_str)));
    }
    out
}

fn template_affixes(
    lang: &str,
    unit: RtfUnit,
    style: RtfStyle,
    is_past: bool,
    plural_cat: &str,
) -> (&'static str, &'static str) {
    match lang {
        "zh" => {
            let u = match (unit, style) {
                (RtfUnit::Year, _) => "年",
                (RtfUnit::Quarter, RtfStyle::Narrow) => "季度",
                (RtfUnit::Quarter, _) => "个季度",
                (RtfUnit::Month, RtfStyle::Narrow) => "个月",
                (RtfUnit::Month, _) => "个月",
                (RtfUnit::Week, _) => "周",
                (RtfUnit::Day, _) => "天",
                (RtfUnit::Hour, _) => "小时",
                (RtfUnit::Minute, RtfStyle::Narrow) => "分",
                (RtfUnit::Minute, _) => "分钟",
                (RtfUnit::Second, _) => "秒",
            };
            if is_past {
                (
                    "",
                    match u {
                        "年" => "年前",
                        "季度" => "季度前",
                        "个季度" => "个季度前",
                        "个月" => "个月前",
                        "周" => "周前",
                        "天" => "天前",
                        "小时" => "小时前",
                        "分" => "分前",
                        "分钟" => "分钟前",
                        _ => "秒前",
                    },
                )
            } else {
                (
                    "",
                    match u {
                        "年" => "年后",
                        "季度" => "季度后",
                        "个季度" => "个季度后",
                        "个月" => "个月后",
                        "周" => "周后",
                        "天" => "天后",
                        "小时" => "小时后",
                        "分" => "分后",
                        "分钟" => "分钟后",
                        _ => "秒后",
                    },
                )
            }
        },
        "ja" => {
            let suffix = match (unit, is_past) {
                (RtfUnit::Year, true) => " 年前",
                (RtfUnit::Year, false) => " 年後",
                (RtfUnit::Quarter, true) => " 四半期前",
                (RtfUnit::Quarter, false) => " 四半期後",
                (RtfUnit::Month, true) => " か月前",
                (RtfUnit::Month, false) => " か月後",
                (RtfUnit::Week, true) => " 週間前",
                (RtfUnit::Week, false) => " 週間後",
                (RtfUnit::Day, true) => " 日前",
                (RtfUnit::Day, false) => " 日後",
                (RtfUnit::Hour, true) => " 時間前",
                (RtfUnit::Hour, false) => " 時間後",
                (RtfUnit::Minute, true) => " 分前",
                (RtfUnit::Minute, false) => " 分後",
                (RtfUnit::Second, true) => " 秒前",
                (RtfUnit::Second, false) => " 秒後",
            };
            ("", suffix)
        },
        "ko" => {
            let suffix = match (unit, is_past) {
                (RtfUnit::Year, true) => "년 전",
                (RtfUnit::Year, false) => "년 후",
                (RtfUnit::Quarter, true) => "분기 전",
                (RtfUnit::Quarter, false) => "분기 후",
                (RtfUnit::Month, true) => "개월 전",
                (RtfUnit::Month, false) => "개월 후",
                (RtfUnit::Week, true) => "주 전",
                (RtfUnit::Week, false) => "주 후",
                (RtfUnit::Day, true) => "일 전",
                (RtfUnit::Day, false) => "일 후",
                (RtfUnit::Hour, true) => "시간 전",
                (RtfUnit::Hour, false) => "시간 후",
                (RtfUnit::Minute, true) => "분 전",
                (RtfUnit::Minute, false) => "분 후",
                (RtfUnit::Second, true) => "초 전",
                (RtfUnit::Second, false) => "초 후",
            };
            ("", suffix)
        },
        "fr" => {
            let singular = plural_cat == "one";
            let unit_label = match (unit, style, singular) {
                (RtfUnit::Year, RtfStyle::Long, true) => "an",
                (RtfUnit::Year, RtfStyle::Long, false) => "ans",
                (RtfUnit::Year, _, _) => "a",
                (RtfUnit::Quarter, RtfStyle::Long, true) => "trimestre",
                (RtfUnit::Quarter, RtfStyle::Long, false) => "trimestres",
                (RtfUnit::Quarter, _, _) => "trim.",
                (RtfUnit::Month, _, _) => "mois",
                (RtfUnit::Week, RtfStyle::Long, true) => "semaine",
                (RtfUnit::Week, RtfStyle::Long, false) => "semaines",
                (RtfUnit::Week, _, _) => "sem.",
                (RtfUnit::Day, RtfStyle::Long, true) => "jour",
                (RtfUnit::Day, RtfStyle::Long, false) => "jours",
                (RtfUnit::Day, _, _) => "j",
                (RtfUnit::Hour, RtfStyle::Long, true) => "heure",
                (RtfUnit::Hour, RtfStyle::Long, false) => "heures",
                (RtfUnit::Hour, _, _) => "h",
                (RtfUnit::Minute, RtfStyle::Long, true) => "minute",
                (RtfUnit::Minute, RtfStyle::Long, false) => "minutes",
                (RtfUnit::Minute, _, _) => "min",
                (RtfUnit::Second, RtfStyle::Long, true) => "seconde",
                (RtfUnit::Second, RtfStyle::Long, false) => "secondes",
                (RtfUnit::Second, _, _) => "s",
            };
            let _ = unit_label;
            // Prefix-based French
            if is_past {
                (
                    "il y a ",
                    match (unit, style, singular) {
                        (RtfUnit::Year, RtfStyle::Long, true) => " an",
                        (RtfUnit::Year, RtfStyle::Long, false) => " ans",
                        (RtfUnit::Year, _, _) => " a",
                        (RtfUnit::Quarter, RtfStyle::Long, true) => " trimestre",
                        (RtfUnit::Quarter, RtfStyle::Long, false) => " trimestres",
                        (RtfUnit::Quarter, _, _) => " trim.",
                        (RtfUnit::Month, _, _) => " mois",
                        (RtfUnit::Week, RtfStyle::Long, true) => " semaine",
                        (RtfUnit::Week, RtfStyle::Long, false) => " semaines",
                        (RtfUnit::Week, _, _) => " sem.",
                        (RtfUnit::Day, RtfStyle::Long, true) => " jour",
                        (RtfUnit::Day, RtfStyle::Long, false) => " jours",
                        (RtfUnit::Day, _, _) => " j",
                        (RtfUnit::Hour, RtfStyle::Long, true) => " heure",
                        (RtfUnit::Hour, RtfStyle::Long, false) => " heures",
                        (RtfUnit::Hour, _, _) => " h",
                        (RtfUnit::Minute, RtfStyle::Long, true) => " minute",
                        (RtfUnit::Minute, RtfStyle::Long, false) => " minutes",
                        (RtfUnit::Minute, _, _) => " min",
                        (RtfUnit::Second, RtfStyle::Long, true) => " seconde",
                        (RtfUnit::Second, RtfStyle::Long, false) => " secondes",
                        (RtfUnit::Second, _, _) => " s",
                    },
                )
            } else {
                (
                    "dans ",
                    match (unit, style, singular) {
                        (RtfUnit::Year, RtfStyle::Long, true) => " an",
                        (RtfUnit::Year, RtfStyle::Long, false) => " ans",
                        (RtfUnit::Year, _, _) => " a",
                        (RtfUnit::Quarter, RtfStyle::Long, true) => " trimestre",
                        (RtfUnit::Quarter, RtfStyle::Long, false) => " trimestres",
                        (RtfUnit::Quarter, _, _) => " trim.",
                        (RtfUnit::Month, _, _) => " mois",
                        (RtfUnit::Week, RtfStyle::Long, true) => " semaine",
                        (RtfUnit::Week, RtfStyle::Long, false) => " semaines",
                        (RtfUnit::Week, _, _) => " sem.",
                        (RtfUnit::Day, RtfStyle::Long, true) => " jour",
                        (RtfUnit::Day, RtfStyle::Long, false) => " jours",
                        (RtfUnit::Day, _, _) => " j",
                        (RtfUnit::Hour, RtfStyle::Long, true) => " heure",
                        (RtfUnit::Hour, RtfStyle::Long, false) => " heures",
                        (RtfUnit::Hour, _, _) => " h",
                        (RtfUnit::Minute, RtfStyle::Long, true) => " minute",
                        (RtfUnit::Minute, RtfStyle::Long, false) => " minutes",
                        (RtfUnit::Minute, _, _) => " min",
                        (RtfUnit::Second, RtfStyle::Long, true) => " seconde",
                        (RtfUnit::Second, RtfStyle::Long, false) => " secondes",
                        (RtfUnit::Second, _, _) => " s",
                    },
                )
            }
        },
        "de" => {
            let singular = plural_cat == "one";
            let suffix = match (unit, style, singular) {
                (RtfUnit::Year, _, true) => " Jahr",
                (RtfUnit::Year, _, false) => " Jahren",
                (RtfUnit::Quarter, _, _) => " Quart.",
                (RtfUnit::Month, _, true) => " Monat",
                (RtfUnit::Month, _, false) => " Monaten",
                (RtfUnit::Week, _, true) => " Woche",
                (RtfUnit::Week, _, false) => " Wochen",
                (RtfUnit::Day, _, true) => " Tag",
                (RtfUnit::Day, _, false) => " Tagen",
                (RtfUnit::Hour, _, true) => " Stunde",
                (RtfUnit::Hour, _, false) => " Stunden",
                (RtfUnit::Minute, _, true) => " Minute",
                (RtfUnit::Minute, _, false) => " Minuten",
                (RtfUnit::Second, _, true) => " Sekunde",
                (RtfUnit::Second, _, false) => " Sekunden",
            };
            if is_past {
                ("vor ", suffix)
            } else {
                ("in ", suffix)
            }
        },
        "es" => {
            let singular = plural_cat == "one";
            let suffix = match (unit, singular) {
                (RtfUnit::Year, true) => " año",
                (RtfUnit::Year, false) => " años",
                (RtfUnit::Quarter, true) => " trimestre",
                (RtfUnit::Quarter, false) => " trimestres",
                (RtfUnit::Month, true) => " mes",
                (RtfUnit::Month, false) => " meses",
                (RtfUnit::Week, true) => " semana",
                (RtfUnit::Week, false) => " semanas",
                (RtfUnit::Day, true) => " día",
                (RtfUnit::Day, false) => " días",
                (RtfUnit::Hour, true) => " hora",
                (RtfUnit::Hour, false) => " horas",
                (RtfUnit::Minute, true) => " minuto",
                (RtfUnit::Minute, false) => " minutos",
                (RtfUnit::Second, true) => " segundo",
                (RtfUnit::Second, false) => " segundos",
            };
            if is_past {
                ("hace ", suffix)
            } else {
                ("dentro de ", suffix)
            }
        },
        _ => {
            // English default
            let singular = plural_cat == "one";
            match (unit, style, is_past, singular) {
                (RtfUnit::Year, RtfStyle::Long, true, true) => ("", " year ago"),
                (RtfUnit::Year, RtfStyle::Long, true, false) => ("", " years ago"),
                (RtfUnit::Year, RtfStyle::Long, false, true) => ("in ", " year"),
                (RtfUnit::Year, RtfStyle::Long, false, false) => ("in ", " years"),
                (RtfUnit::Year, RtfStyle::Short, true, _) => ("", " yr. ago"),
                (RtfUnit::Year, RtfStyle::Short, false, _) => ("in ", " yr."),
                (RtfUnit::Year, RtfStyle::Narrow, true, _) => ("", "y ago"),
                (RtfUnit::Year, RtfStyle::Narrow, false, _) => ("in ", "y"),

                (RtfUnit::Quarter, RtfStyle::Long, true, true) => ("", " quarter ago"),
                (RtfUnit::Quarter, RtfStyle::Long, true, false) => ("", " quarters ago"),
                (RtfUnit::Quarter, RtfStyle::Long, false, true) => ("in ", " quarter"),
                (RtfUnit::Quarter, RtfStyle::Long, false, false) => ("in ", " quarters"),
                (RtfUnit::Quarter, _, true, _) => ("", " qtr. ago"),
                (RtfUnit::Quarter, _, false, _) => ("in ", " qtr."),

                (RtfUnit::Month, RtfStyle::Long, true, true) => ("", " month ago"),
                (RtfUnit::Month, RtfStyle::Long, true, false) => ("", " months ago"),
                (RtfUnit::Month, RtfStyle::Long, false, true) => ("in ", " month"),
                (RtfUnit::Month, RtfStyle::Long, false, false) => ("in ", " months"),
                (RtfUnit::Month, RtfStyle::Short, true, _) => ("", " mo. ago"),
                (RtfUnit::Month, RtfStyle::Short, false, _) => ("in ", " mo."),
                (RtfUnit::Month, RtfStyle::Narrow, true, _) => ("", "m ago"),
                (RtfUnit::Month, RtfStyle::Narrow, false, _) => ("in ", "m"),

                (RtfUnit::Week, RtfStyle::Long, true, true) => ("", " week ago"),
                (RtfUnit::Week, RtfStyle::Long, true, false) => ("", " weeks ago"),
                (RtfUnit::Week, RtfStyle::Long, false, true) => ("in ", " week"),
                (RtfUnit::Week, RtfStyle::Long, false, false) => ("in ", " weeks"),
                (RtfUnit::Week, RtfStyle::Short, true, _) => ("", " wk. ago"),
                (RtfUnit::Week, RtfStyle::Short, false, _) => ("in ", " wk."),
                (RtfUnit::Week, RtfStyle::Narrow, true, _) => ("", "w ago"),
                (RtfUnit::Week, RtfStyle::Narrow, false, _) => ("in ", "w"),

                (RtfUnit::Day, RtfStyle::Long, true, true) => ("", " day ago"),
                (RtfUnit::Day, RtfStyle::Long, true, false) => ("", " days ago"),
                (RtfUnit::Day, RtfStyle::Long, false, true) => ("in ", " day"),
                (RtfUnit::Day, RtfStyle::Long, false, false) => ("in ", " days"),
                (RtfUnit::Day, RtfStyle::Short, true, true) => ("", " day ago"),
                (RtfUnit::Day, RtfStyle::Short, true, false) => ("", " days ago"),
                (RtfUnit::Day, RtfStyle::Short, false, true) => ("in ", " day"),
                (RtfUnit::Day, RtfStyle::Short, false, false) => ("in ", " days"),
                (RtfUnit::Day, RtfStyle::Narrow, true, _) => ("", "d ago"),
                (RtfUnit::Day, RtfStyle::Narrow, false, _) => ("in ", "d"),

                (RtfUnit::Hour, RtfStyle::Long, true, true) => ("", " hour ago"),
                (RtfUnit::Hour, RtfStyle::Long, true, false) => ("", " hours ago"),
                (RtfUnit::Hour, RtfStyle::Long, false, true) => ("in ", " hour"),
                (RtfUnit::Hour, RtfStyle::Long, false, false) => ("in ", " hours"),
                (RtfUnit::Hour, RtfStyle::Short, true, _) => ("", " hr. ago"),
                (RtfUnit::Hour, RtfStyle::Short, false, _) => ("in ", " hr."),
                (RtfUnit::Hour, RtfStyle::Narrow, true, _) => ("", "h ago"),
                (RtfUnit::Hour, RtfStyle::Narrow, false, _) => ("in ", "h"),

                (RtfUnit::Minute, RtfStyle::Long, true, true) => ("", " minute ago"),
                (RtfUnit::Minute, RtfStyle::Long, true, false) => ("", " minutes ago"),
                (RtfUnit::Minute, RtfStyle::Long, false, true) => ("in ", " minute"),
                (RtfUnit::Minute, RtfStyle::Long, false, false) => ("in ", " minutes"),
                (RtfUnit::Minute, RtfStyle::Short, true, _) => ("", " min. ago"),
                (RtfUnit::Minute, RtfStyle::Short, false, _) => ("in ", " min."),
                (RtfUnit::Minute, RtfStyle::Narrow, true, _) => ("", "m ago"),
                (RtfUnit::Minute, RtfStyle::Narrow, false, _) => ("in ", "m"),

                (RtfUnit::Second, RtfStyle::Long, true, true) => ("", " second ago"),
                (RtfUnit::Second, RtfStyle::Long, true, false) => ("", " seconds ago"),
                (RtfUnit::Second, RtfStyle::Long, false, true) => ("in ", " second"),
                (RtfUnit::Second, RtfStyle::Long, false, false) => ("in ", " seconds"),
                (RtfUnit::Second, RtfStyle::Short, true, _) => ("", " sec. ago"),
                (RtfUnit::Second, RtfStyle::Short, false, _) => ("in ", " sec."),
                (RtfUnit::Second, RtfStyle::Narrow, true, _) => ("", "s ago"),
                (RtfUnit::Second, RtfStyle::Narrow, false, _) => ("in ", "s"),
            }
        },
    }
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct RelativeTimeFormat {
    #[qjs(skip_trace)]
    locale: String,
    #[qjs(skip_trace)]
    language: String,
    #[qjs(skip_trace)]
    style: RtfStyle,
    #[qjs(skip_trace)]
    numeric: RtfNumeric,
}

impl RelativeTimeFormat {
    fn build_parts(
        &self,
        ctx: &Ctx<'_>,
        value: f64,
        unit_str: &str,
    ) -> Result<Vec<(&'static str, String, Option<&'static str>)>> {
        if !value.is_finite() {
            return Err(Exception::throw_range(
                ctx,
                "Invalid value for Intl.RelativeTimeFormat",
            ));
        }
        let unit = RtfUnit::from_str(unit_str).ok_or_else(|| {
            Exception::throw_range(
                ctx,
                &format!("Invalid unit `{unit_str}` for Intl.RelativeTimeFormat"),
            )
        })?;

        if self.numeric == RtfNumeric::Auto && (value == -1.0 || value == 0.0 || value == 1.0) {
            if let Some(phrase) = auto_phrase(&self.language, unit, value as i64, self.style) {
                return Ok(vec![("literal", phrase.to_string(), None)]);
            }
        }

        let is_past = value.is_sign_negative();
        let abs_val = value.abs();
        let plural_cat = select_cardinal_for_locale(&self.locale, abs_val);
        let (prefix, suffix) =
            template_affixes(&self.language, unit, self.style, is_past, plural_cat);

        let mut parts = Vec::new();
        if !prefix.is_empty() {
            parts.push(("literal", prefix.to_string(), None));
        }
        parts.extend(format_number_parts(abs_val, unit.as_str()));
        if !suffix.is_empty() {
            parts.push(("literal", suffix.to_string(), None));
        }
        Ok(parts)
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl RelativeTimeFormat {
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
        let mut style = RtfStyle::Long;
        let mut numeric = RtfNumeric::Always;

        if let Some(opts) = options.into_inner() {
            if let Some(lm) = opts.get::<_, Option<String>>("localeMatcher")? {
                if lm != "lookup" && lm != "best fit" {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid localeMatcher for Intl.RelativeTimeFormat",
                    ));
                }
            }
            if let Some(s) = opts.get::<_, Option<String>>("style")? {
                style = match s.as_str() {
                    "long" => RtfStyle::Long,
                    "short" => RtfStyle::Short,
                    "narrow" => RtfStyle::Narrow,
                    _ => {
                        return Err(Exception::throw_range(
                            &ctx,
                            "Invalid style for Intl.RelativeTimeFormat",
                        ))
                    },
                };
            }
            if let Some(n) = opts.get::<_, Option<String>>("numeric")? {
                numeric = match n.as_str() {
                    "always" => RtfNumeric::Always,
                    "auto" => RtfNumeric::Auto,
                    _ => {
                        return Err(Exception::throw_range(
                            &ctx,
                            "Invalid numeric option for Intl.RelativeTimeFormat",
                        ))
                    },
                };
            }
        }

        Ok(Self {
            locale,
            language,
            style,
            numeric,
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

    pub fn format(&self, ctx: Ctx<'_>, value: Value<'_>, unit: String) -> Result<String> {
        let num = value.as_number().ok_or_else(|| {
            Exception::throw_range(&ctx, "Value must be a number in Intl.RelativeTimeFormat")
        })?;
        let parts = self.build_parts(&ctx, num, &unit)?;
        let mut out = String::new();
        for (_, text, _) in parts {
            out.push_str(&text);
        }
        Ok(out)
    }

    #[qjs(rename = "formatToParts")]
    pub fn format_to_parts<'js>(
        &self,
        ctx: Ctx<'js>,
        value: Value<'_>,
        unit: String,
    ) -> Result<Array<'js>> {
        let num = value.as_number().ok_or_else(|| {
            Exception::throw_range(&ctx, "Value must be a number in Intl.RelativeTimeFormat")
        })?;
        let parts = self.build_parts(&ctx, num, &unit)?;
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
        obj.set("style", self.style.as_str())?;
        obj.set("numeric", self.numeric.as_str())?;
        obj.set("numberingSystem", "latn")?;
        Ok(obj)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Intl.RelativeTimeFormat"
    }
}
