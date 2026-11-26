// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::cmp::Ordering;

use rquickjs::{atom::PredefinedAtom, function::Opt, Ctx, Exception, Object, Result, Value};

use crate::parse_locale_value;

#[derive(Clone, rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct Collator {
    #[qjs(skip_trace)]
    locale: String,
    #[qjs(skip_trace)]
    usage: String,
    #[qjs(skip_trace)]
    sensitivity: String,
    #[qjs(skip_trace)]
    numeric: bool,
    #[qjs(skip_trace)]
    ignore_punctuation: bool,
    #[qjs(skip_trace)]
    case_first: String,
}

fn compare_char_insensitive(a: &str, b: &str) -> Ordering {
    let al = a.to_lowercase();
    let bl = b.to_lowercase();
    al.cmp(&bl)
}

fn compare_numeric_segments(a: &str, b: &str, fold_case: bool) -> Ordering {
    let mut ai = a.char_indices().peekable();
    let mut bi = b.char_indices().peekable();

    while ai.peek().is_some() && bi.peek().is_some() {
        let (a_start, a_ch) = *ai.peek().unwrap();
        let (b_start, b_ch) = *bi.peek().unwrap();
        if a_ch.is_ascii_digit() && b_ch.is_ascii_digit() {
            let mut a_end = a_start;
            while let Some(&(idx, c)) = ai.peek() {
                if c.is_ascii_digit() {
                    a_end = idx + c.len_utf8();
                    ai.next();
                } else {
                    break;
                }
            }
            let mut b_end = b_start;
            while let Some(&(idx, c)) = bi.peek() {
                if c.is_ascii_digit() {
                    b_end = idx + c.len_utf8();
                    bi.next();
                } else {
                    break;
                }
            }
            let a_digits = a[a_start..a_end].trim_start_matches('0');
            let b_digits = b[b_start..b_end].trim_start_matches('0');
            let ord = a_digits
                .len()
                .cmp(&b_digits.len())
                .then_with(|| a_digits.cmp(b_digits));
            if ord != Ordering::Equal {
                return ord;
            }
            // Longer leading-zero count comes after shorter when values match
            let full_ord = a[a_start..a_end].len().cmp(&b[b_start..b_end].len());
            if full_ord != Ordering::Equal {
                return full_ord;
            }
        } else {
            ai.next();
            bi.next();
            let ord = if fold_case {
                a_ch.to_lowercase().cmp(b_ch.to_lowercase())
            } else {
                a_ch.cmp(&b_ch)
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
    }

    ai.peek().is_some().cmp(&bi.peek().is_some())
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Collator {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, locales: Opt<Value<'_>>, options: Opt<Object<'_>>) -> Result<Self> {
        let locale = parse_locale_value(locales.into_inner())?;
        let mut usage = "sort".to_string();
        let mut sensitivity = "variant".to_string();
        let mut numeric = false;
        let mut ignore_punctuation = false;
        let mut case_first = "false".to_string();

        if let Some(opts) = options.into_inner() {
            if let Ok(Some(u)) = opts.get::<_, Option<String>>("usage") {
                if !matches!(u.as_str(), "sort" | "search") {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid usage for Intl.Collator",
                    ));
                }
                usage = u;
            }
            if let Ok(Some(s)) = opts.get::<_, Option<String>>("sensitivity") {
                if !matches!(s.as_str(), "base" | "accent" | "case" | "variant") {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid sensitivity for Intl.Collator",
                    ));
                }
                sensitivity = s;
            }
            if let Ok(Some(n)) = opts.get::<_, Option<bool>>("numeric") {
                numeric = n;
            }
            if let Ok(Some(p)) = opts.get::<_, Option<bool>>("ignorePunctuation") {
                ignore_punctuation = p;
            }
            if let Ok(Some(c)) = opts.get::<_, Option<String>>("caseFirst") {
                if !matches!(c.as_str(), "upper" | "lower" | "false") {
                    return Err(Exception::throw_range(
                        &ctx,
                        "Invalid caseFirst for Intl.Collator",
                    ));
                }
                case_first = c;
            }
        }

        Ok(Self {
            locale,
            usage,
            sensitivity,
            numeric,
            ignore_punctuation,
            case_first,
        })
    }

    pub fn compare(&self, a: String, b: String) -> i32 {
        let strip = |s: String| {
            if self.ignore_punctuation {
                s.chars()
                    .filter(|c| !c.is_ascii_punctuation())
                    .collect::<String>()
            } else {
                s
            }
        };
        let sa = strip(a);
        let sb = strip(b);
        let fold_case = matches!(self.sensitivity.as_str(), "base" | "accent");

        let ord = if self.numeric {
            compare_numeric_segments(&sa, &sb, fold_case)
        } else if fold_case {
            compare_char_insensitive(&sa, &sb)
        } else {
            compare_char_insensitive(&sa, &sb).then_with(|| sa.cmp(&sb))
        };

        match ord {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }
    }

    #[qjs(rename = "resolvedOptions")]
    pub fn resolved_options<'js>(&self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let obj = Object::new(ctx)?;
        obj.set("locale", self.locale.as_str())?;
        obj.set("usage", self.usage.as_str())?;
        obj.set("sensitivity", self.sensitivity.as_str())?;
        obj.set("ignorePunctuation", self.ignore_punctuation)?;
        obj.set("collation", "default")?;
        obj.set("numeric", self.numeric)?;
        obj.set("caseFirst", self.case_first.as_str())?;
        Ok(obj)
    }

    #[qjs(get, rename = PredefinedAtom::SymbolToStringTag)]
    pub fn to_string_tag(&self) -> &'static str {
        "Intl.Collator"
    }
}
