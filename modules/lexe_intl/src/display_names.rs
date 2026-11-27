// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use rquickjs::{atom::PredefinedAtom, function::Opt, Array, Ctx, Exception, Object, Result, Value};

use crate::locale::{parse_bcp47_tag, resolve_locale_arg, supported_locales_of_impl};

#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayNamesType {
    Language,
    Region,
    Script,
    Currency,
    Calendar,
    DateTimeField,
}

impl DisplayNamesType {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "language" => Some(Self::Language),
            "region" => Some(Self::Region),
            "script" => Some(Self::Script),
            "currency" => Some(Self::Currency),
            "calendar" => Some(Self::Calendar),
            "dateTimeField" => Some(Self::DateTimeField),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Language => "language",
            Self::Region => "region",
            Self::Script => "script",
            Self::Currency => "currency",
            Self::Calendar => "calendar",
            Self::DateTimeField => "dateTimeField",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayStyle {
    Long,
    Short,
    Narrow,
}

impl DisplayStyle {
    fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
            Self::Narrow => "narrow",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FallbackMode {
    Code,
    None,
}

impl FallbackMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LanguageDisplay {
    Dialect,
    Standard,
}

impl LanguageDisplay {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dialect => "dialect",
            Self::Standard => "standard",
        }
    }
}

fn lookup_date_time_field(lang: &str, code: &str, style: DisplayStyle) -> Option<&'static str> {
    match lang {
        "zh" => match code {
            "era" => Some("纪元"),
            "year" => Some("年"),
            "quarter" => Some(if style == DisplayStyle::Narrow {
                "季"
            } else {
                "季度"
            }),
            "month" => Some("月"),
            "weekOfYear" => Some("周"),
            "weekday" => Some(if style == DisplayStyle::Long {
                "工作日"
            } else {
                "周几"
            }),
            "day" => Some("日"),
            "dayPeriod" => Some("上午/下午"),
            "hour" => Some(if style == DisplayStyle::Narrow {
                "时"
            } else {
                "小时"
            }),
            "minute" => Some(if style == DisplayStyle::Narrow {
                "分"
            } else {
                "分钟"
            }),
            "second" => Some("秒"),
            "timeZoneName" => Some("时区"),
            _ => None,
        },
        "ja" => match code {
            "era" => Some("時代"),
            "year" => Some("年"),
            "quarter" => Some("四半期"),
            "month" => Some("月"),
            "weekOfYear" => Some("週"),
            "weekday" => Some("曜日"),
            "day" => Some("日"),
            "dayPeriod" => Some("午前/午後"),
            "hour" => Some("時"),
            "minute" => Some("分"),
            "second" => Some("秒"),
            "timeZoneName" => Some("タイムゾーン"),
            _ => None,
        },
        "ko" => match code {
            "era" => Some("연호"),
            "year" => Some("년"),
            "quarter" => Some("분기"),
            "month" => Some("월"),
            "weekOfYear" => Some("주"),
            "weekday" => Some("요일"),
            "day" => Some("일"),
            "dayPeriod" => Some("오전/오후"),
            "hour" => Some("시"),
            "minute" => Some("분"),
            "second" => Some("초"),
            "timeZoneName" => Some("시간대"),
            _ => None,
        },
        "fr" => match code {
            "era" => Some("ère"),
            "year" => Some(if style == DisplayStyle::Long {
                "année"
            } else {
                "an"
            }),
            "quarter" => Some(if style == DisplayStyle::Long {
                "trimestre"
            } else {
                "trim."
            }),
            "month" => Some("mois"),
            "weekOfYear" => Some(if style == DisplayStyle::Long {
                "semaine"
            } else {
                "sem."
            }),
            "weekday" => Some("jour de la semaine"),
            "day" => Some(if style == DisplayStyle::Narrow {
                "j"
            } else {
                "jour"
            }),
            "dayPeriod" => Some("cadran"),
            "hour" => Some(if style == DisplayStyle::Long {
                "heure"
            } else {
                "h"
            }),
            "minute" => Some(if style == DisplayStyle::Long {
                "minute"
            } else {
                "min"
            }),
            "second" => Some(if style == DisplayStyle::Long {
                "seconde"
            } else {
                "s"
            }),
            "timeZoneName" => Some("fuseau horaire"),
            _ => None,
        },
        "de" => match code {
            "era" => Some("Epoche"),
            "year" => Some(if style == DisplayStyle::Narrow {
                "J"
            } else {
                "Jahr"
            }),
            "quarter" => Some(if style == DisplayStyle::Long {
                "Quartal"
            } else {
                "Quart."
            }),
            "month" => Some(if style == DisplayStyle::Narrow {
                "M"
            } else {
                "Monat"
            }),
            "weekOfYear" => Some(if style == DisplayStyle::Long {
                "Woche"
            } else {
                "W"
            }),
            "weekday" => Some("Wochentag"),
            "day" => Some("Tag"),
            "dayPeriod" => Some("Tageshälfte"),
            "hour" => Some(if style == DisplayStyle::Long {
                "Stunde"
            } else {
                "Std."
            }),
            "minute" => Some(if style == DisplayStyle::Long {
                "Minute"
            } else {
                "Min."
            }),
            "second" => Some(if style == DisplayStyle::Long {
                "Sekunde"
            } else {
                "Sek."
            }),
            "timeZoneName" => Some("Zeitzone"),
            _ => None,
        },
        "es" => match code {
            "era" => Some("era"),
            "year" => Some("año"),
            "quarter" => Some(if style == DisplayStyle::Long {
                "trimestre"
            } else {
                "trim."
            }),
            "month" => Some("mes"),
            "weekOfYear" => Some(if style == DisplayStyle::Long {
                "semana"
            } else {
                "sem."
            }),
            "weekday" => Some("día de la semana"),
            "day" => Some("día"),
            "dayPeriod" => Some("a. m./p. m."),
            "hour" => Some("hora"),
            "minute" => Some(if style == DisplayStyle::Long {
                "minuto"
            } else {
                "min"
            }),
            "second" => Some(if style == DisplayStyle::Long {
                "segundo"
            } else {
                "s"
            }),
            "timeZoneName" => Some("zona horaria"),
            _ => None,
        },
        _ => match (code, style) {
            ("era", _) => Some("era"),
            ("year", DisplayStyle::Long) => Some("year"),
            ("year", _) => Some("yr."),
            ("quarter", DisplayStyle::Long) => Some("quarter"),
            ("quarter", _) => Some("qtr."),
            ("month", DisplayStyle::Long) => Some("month"),
            ("month", _) => Some("mo."),
            ("weekOfYear", DisplayStyle::Long) => Some("week"),
            ("weekOfYear", _) => Some("wk."),
            ("weekday", DisplayStyle::Long) => Some("day of the week"),
            ("weekday", _) => Some("day of wk."),
            ("day", _) => Some("day"),
            ("dayPeriod", _) => Some("AM/PM"),
            ("hour", DisplayStyle::Long) => Some("hour"),
            ("hour", _) => Some("hr."),
            ("minute", DisplayStyle::Long) => Some("minute"),
            ("minute", _) => Some("min."),
            ("second", DisplayStyle::Long) => Some("second"),
            ("second", _) => Some("sec."),
            ("timeZoneName", _) => Some("time zone"),
            _ => None,
        },
    }
}

fn lookup_calendar(lang: &str, code: &str) -> Option<&'static str> {
    match (lang, code) {
        ("zh", "gregory") => Some("公历"),
        ("zh", "iso8601") => Some("ISO 8601 日历"),
        ("zh", "chinese") => Some("农历"),
        ("zh", "japanese") => Some("和历"),
        ("zh", "buddhist") => Some("佛历"),
        ("zh", "hebrew") => Some("希伯来历"),
        ("zh", "islamic") => Some("伊斯兰历"),
        ("zh", "persian") => Some("波斯历"),
        ("zh", "roc") => Some("民国纪年"),
        (_, "buddhist") => Some("Buddhist Calendar"),
        (_, "chinese") => Some("Chinese Calendar"),
        (_, "coptic") => Some("Coptic Calendar"),
        (_, "dangi") => Some("Dangi Calendar"),
        (_, "ethioaa") => Some("Ethiopic Amete Alem Calendar"),
        (_, "ethiopic") => Some("Ethiopic Calendar"),
        (_, "gregory") => Some("Gregorian Calendar"),
        (_, "hebrew") => Some("Hebrew Calendar"),
        (_, "indian") => Some("Indian National Calendar"),
        (_, "islamic") => Some("Islamic Calendar"),
        (_, "islamic-civil") => Some("Islamic Calendar (tabular, civil epoch)"),
        (_, "islamic-rgsa") => Some("Islamic Calendar (Saudi Arabia, sighting)"),
        (_, "islamic-tbla") => Some("Islamic Calendar (tabular, astronomical epoch)"),
        (_, "islamic-umalqura") => Some("Islamic Calendar (Umm al-Qura)"),
        (_, "iso8601") => Some("ISO-8601 Calendar"),
        (_, "japanese") => Some("Japanese Calendar"),
        (_, "persian") => Some("Persian Calendar"),
        (_, "roc") => Some("Minguo Calendar"),
        _ => None,
    }
}

fn lookup_currency(lang: &str, code: &str, style: DisplayStyle) -> Option<&'static str> {
    if style == DisplayStyle::Narrow {
        if let Some(sym) = match code {
            "USD" | "AUD" | "CAD" | "NZD" | "SGD" | "HKD" | "MXN" => Some("$"),
            "EUR" => Some("€"),
            "GBP" => Some("£"),
            "JPY" | "CNY" => Some("¥"),
            "KRW" => Some("₩"),
            "INR" => Some("₹"),
            "RUB" => Some("₽"),
            "TRY" => Some("₺"),
            "THB" => Some("฿"),
            "VND" => Some("₫"),
            "ILS" => Some("₪"),
            "BRL" => Some("R$"),
            _ => None,
        } {
            return Some(sym);
        }
    }
    if lang == "zh" {
        if let Some(name) = match code {
            "USD" => Some("美元"),
            "EUR" => Some("欧元"),
            "GBP" => Some("英镑"),
            "JPY" => Some("日元"),
            "CNY" => Some("人民币"),
            "CNH" => Some("离岸人民币"),
            "HKD" => Some("港元"),
            "TWD" => Some("新台币"),
            "KRW" => Some("韩元"),
            "INR" => Some("印度卢比"),
            "AUD" => Some("澳大利亚元"),
            "CAD" => Some("加拿大元"),
            "CHF" => Some("瑞士法郎"),
            "SGD" => Some("新加坡元"),
            "NZD" => Some("新西兰元"),
            "RUB" => Some("俄罗斯卢布"),
            "BRL" => Some("巴西雷亚尔"),
            _ => None,
        } {
            return Some(name);
        }
    }
    match code {
        "USD" => Some("US Dollar"),
        "EUR" => Some("Euro"),
        "GBP" => Some("British Pound"),
        "JPY" => Some("Japanese Yen"),
        "CNY" => Some("Chinese Yuan"),
        "CNH" => Some("Chinese Yuan (offshore)"),
        "HKD" => Some("Hong Kong Dollar"),
        "TWD" => Some("New Taiwan Dollar"),
        "KRW" => Some("South Korean Won"),
        "INR" => Some("Indian Rupee"),
        "AUD" => Some("Australian Dollar"),
        "CAD" => Some("Canadian Dollar"),
        "CHF" => Some("Swiss Franc"),
        "SGD" => Some("Singapore Dollar"),
        "NZD" => Some("New Zealand Dollar"),
        "SEK" => Some("Swedish Krona"),
        "NOK" => Some("Norwegian Krone"),
        "DKK" => Some("Danish Krone"),
        "PLN" => Some("Polish Zloty"),
        "CZK" => Some("Czech Koruna"),
        "HUF" => Some("Hungarian Forint"),
        "RON" => Some("Romanian Leu"),
        "RUB" => Some("Russian Ruble"),
        "UAH" => Some("Ukrainian Hryvnia"),
        "TRY" => Some("Turkish Lira"),
        "BRL" => Some("Brazilian Real"),
        "MXN" => Some("Mexican Peso"),
        "ARS" => Some("Argentine Peso"),
        "CLP" => Some("Chilean Peso"),
        "COP" => Some("Colombian Peso"),
        "PEN" => Some("Peruvian Sol"),
        "ZAR" => Some("South African Rand"),
        "SAR" => Some("Saudi Riyal"),
        "AED" => Some("United Arab Emirates Dirham"),
        "ILS" => Some("Israeli New Shekel"),
        "EGP" => Some("Egyptian Pound"),
        "THB" => Some("Thai Baht"),
        "VND" => Some("Vietnamese Dong"),
        "IDR" => Some("Indonesian Rupiah"),
        "MYR" => Some("Malaysian Ringgit"),
        "PHP" => Some("Philippine Peso"),
        "PKR" => Some("Pakistani Rupee"),
        "BDT" => Some("Bangladeshi Taka"),
        "NGN" => Some("Nigerian Naira"),
        "KES" => Some("Kenyan Shilling"),
        _ => None,
    }
}

fn lookup_region(lang: &str, code: &str, style: DisplayStyle) -> Option<&'static str> {
    if lang == "zh" {
        if let Some(name) = match code {
            "US" => Some("美国"),
            "CN" => Some("中国"),
            "TW" => Some("中国台湾"),
            "HK" => Some("中国香港"),
            "MO" => Some("中国澳门"),
            "JP" => Some("日本"),
            "KR" => Some("韩国"),
            "GB" => Some("英国"),
            "DE" => Some("德国"),
            "FR" => Some("法国"),
            "ES" => Some("西班牙"),
            "IT" => Some("意大利"),
            "PT" => Some("葡萄牙"),
            "NL" => Some("荷兰"),
            "RU" => Some("俄罗斯"),
            "IN" => Some("印度"),
            "AU" => Some("澳大利亚"),
            "CA" => Some("加拿大"),
            "BR" => Some("巴西"),
            "SG" => Some("新加坡"),
            _ => None,
        } {
            return Some(name);
        }
    }
    match (code, style) {
        ("US", DisplayStyle::Short) => Some("US"),
        ("US", _) => Some("United States"),
        ("GB", DisplayStyle::Short) => Some("UK"),
        ("GB", _) => Some("United Kingdom"),
        ("HK", DisplayStyle::Short) => Some("Hong Kong"),
        ("HK", _) => Some("Hong Kong SAR China"),
        ("MO", DisplayStyle::Short) => Some("Macao"),
        ("MO", _) => Some("Macao SAR China"),
        ("AE", _) => Some("United Arab Emirates"),
        ("AR", _) => Some("Argentina"),
        ("AT", _) => Some("Austria"),
        ("AU", _) => Some("Australia"),
        ("BD", _) => Some("Bangladesh"),
        ("BE", _) => Some("Belgium"),
        ("BR", _) => Some("Brazil"),
        ("CA", _) => Some("Canada"),
        ("CH", _) => Some("Switzerland"),
        ("CL", _) => Some("Chile"),
        ("CN", _) => Some("China"),
        ("CO", _) => Some("Colombia"),
        ("CZ", _) => Some("Czechia"),
        ("DE", _) => Some("Germany"),
        ("DK", _) => Some("Denmark"),
        ("EG", _) => Some("Egypt"),
        ("ES", _) => Some("Spain"),
        ("FI", _) => Some("Finland"),
        ("FR", _) => Some("France"),
        ("GR", _) => Some("Greece"),
        ("HU", _) => Some("Hungary"),
        ("ID", _) => Some("Indonesia"),
        ("IE", _) => Some("Ireland"),
        ("IL", _) => Some("Israel"),
        ("IN", _) => Some("India"),
        ("IT", _) => Some("Italy"),
        ("JP", _) => Some("Japan"),
        ("KE", _) => Some("Kenya"),
        ("KR", _) => Some("South Korea"),
        ("MX", _) => Some("Mexico"),
        ("MY", _) => Some("Malaysia"),
        ("NG", _) => Some("Nigeria"),
        ("NL", _) => Some("Netherlands"),
        ("NO", _) => Some("Norway"),
        ("NZ", _) => Some("New Zealand"),
        ("PE", _) => Some("Peru"),
        ("PH", _) => Some("Philippines"),
        ("PK", _) => Some("Pakistan"),
        ("PL", _) => Some("Poland"),
        ("PT", _) => Some("Portugal"),
        ("RO", _) => Some("Romania"),
        ("RU", _) => Some("Russia"),
        ("SA", _) => Some("Saudi Arabia"),
        ("SE", _) => Some("Sweden"),
        ("SG", _) => Some("Singapore"),
        ("TH", _) => Some("Thailand"),
        ("TR", _) => Some("Türkiye"),
        ("TW", _) => Some("Taiwan"),
        ("UA", _) => Some("Ukraine"),
        ("VN", _) => Some("Vietnam"),
        ("ZA", _) => Some("South Africa"),
        ("001", _) => Some("World"),
        ("419", _) => Some("Latin America"),
        _ => None,
    }
}

fn lookup_script(lang: &str, code: &str) -> Option<&'static str> {
    if lang == "zh" {
        if let Some(name) = match code {
            "Latn" => Some("拉丁文"),
            "Hans" => Some("简体中文"),
            "Hant" => Some("繁体中文"),
            "Cyrl" => Some("西里尔文"),
            "Arab" => Some("阿拉伯文"),
            "Jpan" => Some("日文"),
            "Kore" => Some("韩文"),
            "Grek" => Some("希腊文"),
            "Hebr" => Some("希伯来文"),
            _ => None,
        } {
            return Some(name);
        }
    }
    match code {
        "Arab" => Some("Arabic"),
        "Armn" => Some("Armenian"),
        "Beng" => Some("Bangla"),
        "Cyrl" => Some("Cyrillic"),
        "Deva" => Some("Devanagari"),
        "Geor" => Some("Georgian"),
        "Grek" => Some("Greek"),
        "Gujr" => Some("Gujarati"),
        "Guru" => Some("Gurmukhi"),
        "Hang" => Some("Hangul"),
        "Hani" => Some("Han"),
        "Hans" => Some("Simplified"),
        "Hant" => Some("Traditional"),
        "Hebr" => Some("Hebrew"),
        "Hira" => Some("Hiragana"),
        "Jpan" => Some("Japanese"),
        "Kana" => Some("Katakana"),
        "Knda" => Some("Kannada"),
        "Kore" => Some("Korean"),
        "Latn" => Some("Latin"),
        "Mlym" => Some("Malayalam"),
        "Sinh" => Some("Sinhala"),
        "Taml" => Some("Tamil"),
        "Telu" => Some("Telugu"),
        "Thai" => Some("Thai"),
        _ => None,
    }
}

fn lookup_language_base(lang: &str, code: &str) -> Option<&'static str> {
    if lang == "zh" {
        if let Some(name) = match code {
            "en" => Some("英语"),
            "zh" => Some("中文"),
            "ja" => Some("日语"),
            "ko" => Some("韩语"),
            "fr" => Some("法语"),
            "de" => Some("德语"),
            "es" => Some("西班牙语"),
            "it" => Some("意大利语"),
            "pt" => Some("葡萄牙语"),
            "ru" => Some("俄语"),
            "ar" => Some("阿拉伯语"),
            "hi" => Some("印地语"),
            _ => None,
        } {
            return Some(name);
        }
    }
    match code {
        "ar" => Some("Arabic"),
        "bn" => Some("Bangla"),
        "cs" => Some("Czech"),
        "da" => Some("Danish"),
        "de" => Some("German"),
        "el" => Some("Greek"),
        "en" => Some("English"),
        "es" => Some("Spanish"),
        "fa" => Some("Persian"),
        "fi" => Some("Finnish"),
        "fr" => Some("French"),
        "he" => Some("Hebrew"),
        "hi" => Some("Hindi"),
        "hu" => Some("Hungarian"),
        "id" => Some("Indonesian"),
        "it" => Some("Italian"),
        "ja" => Some("Japanese"),
        "ko" => Some("Korean"),
        "nb" => Some("Norwegian Bokmål"),
        "nl" => Some("Dutch"),
        "nn" => Some("Norwegian Nynorsk"),
        "no" => Some("Norwegian"),
        "pl" => Some("Polish"),
        "pt" => Some("Portuguese"),
        "ro" => Some("Romanian"),
        "ru" => Some("Russian"),
        "sv" => Some("Swedish"),
        "th" => Some("Thai"),
        "tr" => Some("Turkish"),
        "uk" => Some("Ukrainian"),
        "ur" => Some("Urdu"),
        "vi" => Some("Vietnamese"),
        "zh" => Some("Chinese"),
        _ => None,
    }
}

fn format_language_display(
    lang: &str,
    code: &str,
    style: DisplayStyle,
    language_display: LanguageDisplay,
) -> std::result::Result<Option<String>, String> {
    let parsed = parse_bcp47_tag(code)?;
    if language_display == LanguageDisplay::Dialect {
        let base = parsed.base_name();
        if let Some(exact) = match (lang, base.as_str()) {
            ("zh", "en-US") => Some("美国英语"),
            ("zh", "en-GB") => Some("英国英语"),
            ("zh", "zh-Hans") => Some("简体中文"),
            ("zh", "zh-Hant") => Some("繁体中文"),
            (_, "en-US") => Some("American English"),
            (_, "en-GB") => Some("British English"),
            (_, "en-AU") => Some("Australian English"),
            (_, "en-CA") => Some("Canadian English"),
            (_, "zh-Hans") => Some("Simplified Chinese"),
            (_, "zh-Hant") => Some("Traditional Chinese"),
            (_, "pt-BR") => Some("Brazilian Portuguese"),
            (_, "pt-PT") => Some("European Portuguese"),
            (_, "es-419") => Some("Latin American Spanish"),
            (_, "es-MX") => Some("Mexican Spanish"),
            (_, "fr-CA") => Some("Canadian French"),
            _ => None,
        } {
            return Ok(Some(exact.to_string()));
        }
    }

    let Some(base_name) = lookup_language_base(lang, &parsed.language) else {
        return Ok(None);
    };

    let mut qualifiers = Vec::new();
    if let Some(ref scr) = parsed.script {
        let s = lookup_script(lang, scr).unwrap_or(scr.as_str());
        qualifiers.push(s.to_string());
    }
    if let Some(ref reg) = parsed.region {
        let r = lookup_region(lang, reg, style).unwrap_or(reg.as_str());
        qualifiers.push(r.to_string());
    }
    if qualifiers.is_empty() {
        Ok(Some(base_name.to_string()))
    } else {
        Ok(Some(format!("{base_name} ({})", qualifiers.join(", "))))
    }
}

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct DisplayNames {
    #[qjs(skip_trace)]
    locale: String,
    #[qjs(skip_trace)]
    language: String,
    #[qjs(skip_trace)]
    display_type: DisplayNamesType,
    #[qjs(skip_trace)]
    style: DisplayStyle,
    #[qjs(skip_trace)]
    fallback: FallbackMode,
    #[qjs(skip_trace)]
    language_display: LanguageDisplay,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl DisplayNames {
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

        let Some(opts) = options.into_inner() else {
            return Err(Exception::throw_type(
                &ctx,
                "Options object with type property is required for Intl.DisplayNames",
            ));
        };

        if let Some(lm) = opts.get::<_, Option<String>>("localeMatcher")? {
            if lm != "lookup" && lm != "best fit" {
                return Err(Exception::throw_range(
                    &ctx,
                    "Invalid localeMatcher for Intl.DisplayNames",
                ));
            }
        }

        let Some(t) = opts.get::<_, Option<String>>("type")? else {
            return Err(Exception::throw_type(
                &ctx,
                "type option is required for Intl.DisplayNames",
            ));
        };
        let display_type = DisplayNamesType::from_str(&t).ok_or_else(|| {
            Exception::throw_range(&ctx, "Invalid type option for Intl.DisplayNames")
        })?;

        let mut style = DisplayStyle::Long;
        if let Some(s) = opts.get::<_, Option<String>>("style")? {
            style = match s.as_str() {
                "long" => DisplayStyle::Long,
                "short" => DisplayStyle::Short,
                "narrow" => DisplayStyle::Narrow,
                _ => {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid style option for Intl.DisplayNames",
                    ))
                },
            };
        }

        let mut fallback = FallbackMode::Code;
        if let Some(f) = opts.get::<_, Option<String>>("fallback")? {
            fallback = match f.as_str() {
                "code" => FallbackMode::Code,
                "none" => FallbackMode::None,
                _ => {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid fallback option for Intl.DisplayNames",
                    ))
                },
            };
        }

        let mut language_display = LanguageDisplay::Dialect;
        if let Some(ld) = opts.get::<_, Option<String>>("languageDisplay")? {
            language_display = match ld.as_str() {
                "dialect" => LanguageDisplay::Dialect,
                "standard" => LanguageDisplay::Standard,
                _ => {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid languageDisplay option for Intl.DisplayNames",
                    ))
                },
            };
        }

        Ok(Self {
            locale,
            language,
            display_type,
            style,
            fallback,
            language_display,
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

    pub fn of<'js>(&self, ctx: Ctx<'js>, code: Value<'_>) -> Result<Value<'js>> {
        let raw = code
            .as_string()
            .and_then(|s| s.to_string().ok())
            .ok_or_else(|| Exception::throw_type(&ctx, "code must be a string"))?;

        let resolved: Option<String> = match self.display_type {
            DisplayNamesType::Region => {
                let is_valid = (raw.len() == 2 && raw.bytes().all(|b| b.is_ascii_alphabetic()))
                    || (raw.len() == 3 && raw.bytes().all(|b| b.is_ascii_digit()));
                if !is_valid {
                    return Err(Exception::throw_range(
                        &ctx,
                        &format!("Invalid region code: {raw}"),
                    ));
                }
                let norm = raw.to_ascii_uppercase();
                lookup_region(&self.language, &norm, self.style).map(str::to_string)
            },
            DisplayNamesType::Script => {
                if raw.len() != 4 || !raw.bytes().all(|b| b.is_ascii_alphabetic()) {
                    return Err(Exception::throw_range(
                        &ctx,
                        &format!("Invalid script code: {raw}"),
                    ));
                }
                let lower = raw.to_ascii_lowercase();
                let mut chars = lower.chars();
                let norm = match chars.next() {
                    Some(c) => format!("{}{}", c.to_ascii_uppercase(), chars.as_str()),
                    None => lower,
                };
                lookup_script(&self.language, &norm).map(str::to_string)
            },
            DisplayNamesType::Currency => {
                if raw.len() != 3 || !raw.bytes().all(|b| b.is_ascii_alphabetic()) {
                    return Err(Exception::throw_range(
                        &ctx,
                        &format!("Invalid currency code: {raw}"),
                    ));
                }
                let norm = raw.to_ascii_uppercase();
                lookup_currency(&self.language, &norm, self.style).map(str::to_string)
            },
            DisplayNamesType::Calendar => {
                if raw.is_empty()
                    || !raw.split('-').all(|p| {
                        (3..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric())
                    })
                {
                    return Err(Exception::throw_range(
                        &ctx,
                        &format!("Invalid calendar code: {raw}"),
                    ));
                }
                let norm = raw.to_ascii_lowercase();
                lookup_calendar(&self.language, &norm).map(str::to_string)
            },
            DisplayNamesType::DateTimeField => {
                match lookup_date_time_field(&self.language, &raw, self.style) {
                    Some(v) => Some(v.to_string()),
                    None => {
                        return Err(Exception::throw_range(
                            &ctx,
                            &format!("Invalid dateTimeField code: {raw}"),
                        ))
                    },
                }
            },
            DisplayNamesType::Language => {
                format_language_display(&self.language, &raw, self.style, self.language_display)
                    .map_err(|msg| Exception::throw_range(&ctx, &msg))?
            },
        };

        match resolved {
            Some(s) => Ok(rquickjs::String::from_str(ctx, &s)?.into_value()),
            None => match self.fallback {
                FallbackMode::Code => Ok(rquickjs::String::from_str(ctx, &raw)?.into_value()),
                FallbackMode::None => Ok(Value::new_undefined(ctx)),
            },
        }
    }

    #[qjs(rename = "resolvedOptions")]
    pub fn resolved_options<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx)?;
        obj.set("locale", self.locale.as_str())?;
        obj.set("style", self.style.as_str())?;
        obj.set("type", self.display_type.as_str())?;
        obj.set("fallback", self.fallback.as_str())?;
        if self.display_type == DisplayNamesType::Language {
            obj.set("languageDisplay", self.language_display.as_str())?;
        }
        Ok(obj)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Intl.DisplayNames"
    }
}
