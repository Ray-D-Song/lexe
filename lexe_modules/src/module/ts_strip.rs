// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node-compatible `module.stripTypeScriptTypes(code[, options])` and TS execution helper.
//!
//! Supports both:
//! - `mode: "strip"` (default): removes TypeScript type syntax by replacing spans with ASCII spaces
//!   while preserving newlines so line numbers remain identical, rejecting non-erasable TS syntax
//!   (`enum`, `namespace`, parameter properties, `import =`, `<T>expr`) with
//!   `ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX`.
//! - `mode: "transform"`: additionally lowers `enum`/`const enum`, `namespace`, and constructor
//!   parameter properties, and emits optional V3 inline source maps (`sourceMap: true`).

use std::collections::HashSet;

use rquickjs::{function::Constructor, function::Opt, Ctx, Object, Result, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripMode {
    Strip,
    Transform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripOptions {
    pub mode: StripMode,
    pub source_map: bool,
    pub source_url: Option<String>,
}

impl Default for StripOptions {
    fn default() -> Self {
        Self {
            mode: StripMode::Strip,
            source_map: false,
            source_url: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TsStripError {
    pub code: &'static str,
    pub is_syntax: bool,
    pub message: String,
}

impl TsStripError {
    fn unsupported(msg: impl Into<String>) -> Self {
        Self {
            code: "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX",
            is_syntax: true,
            message: msg.into(),
        }
    }

    pub fn throw(&self, ctx: &Ctx<'_>) -> rquickjs::Error {
        let ctor_name = if self.is_syntax {
            "SyntaxError"
        } else {
            "TypeError"
        };
        if let Ok(ctor) = ctx.globals().get::<_, Constructor>(ctor_name) {
            if let Ok(err) = ctor.construct::<_, Object>((self.message.as_str(),)) {
                let _ = err.set("code", self.code);
                return ctx.throw(err.into_value());
            }
        }
        rquickjs::Exception::throw_message(ctx, &self.message)
    }
}

fn throw_code_type_error(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
    if let Ok(ctor) = ctx.globals().get::<_, Constructor>("TypeError") {
        if let Ok(err) = ctor.construct::<_, Object>((message,)) {
            let _ = err.set("code", code);
            return ctx.throw(err.into_value());
        }
    }
    rquickjs::Exception::throw_type(ctx, message)
}

fn js_type_name(v: &Value<'_>) -> &'static str {
    if v.is_null() {
        "null"
    } else if v.is_undefined() {
        "undefined"
    } else if v.is_bool() {
        "boolean"
    } else if v.is_number() {
        "number"
    } else if v.is_string() {
        "string"
    } else if v.is_symbol() {
        "symbol"
    } else if v.is_big_int() {
        "bigint"
    } else if v.is_function() {
        "function"
    } else {
        "object"
    }
}

/// JS entry point: `module.stripTypeScriptTypes(code[, options])`.
pub fn strip_typescript_types<'js>(
    ctx: Ctx<'js>,
    code: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<String> {
    let Some(code_js) = code.as_string() else {
        return Err(throw_code_type_error(
            &ctx,
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"code\" argument must be of type string. Received type {}",
                js_type_name(&code)
            ),
        ));
    };
    let source = code_js.to_string()?;

    let mut opts = StripOptions::default();
    if let Some(opt_val) = options.0 {
        if !opt_val.is_undefined() {
            if opt_val.is_null() || !opt_val.is_object() || opt_val.is_array() {
                return Err(throw_code_type_error(
                    &ctx,
                    "ERR_INVALID_ARG_TYPE",
                    &format!(
                        "The \"options\" argument must be of type object. Received type {}",
                        js_type_name(&opt_val)
                    ),
                ));
            }
            let obj = opt_val.into_object().unwrap();

            if let Ok(mode_val) = obj.get::<_, Value>("mode") {
                if !mode_val.is_undefined() {
                    let Some(s) = mode_val.as_string() else {
                        return Err(throw_code_type_error(
                            &ctx,
                            "ERR_INVALID_ARG_VALUE",
                            &format!(
                                "The property 'options.mode' must be one of: 'strip', 'transform'. Received type {}",
                                js_type_name(&mode_val)
                            ),
                        ));
                    };
                    let mode_str = s.to_string()?;
                    opts.mode = match mode_str.as_str() {
                        "strip" => StripMode::Strip,
                        "transform" => StripMode::Transform,
                        other => {
                            return Err(throw_code_type_error(
                                &ctx,
                                "ERR_INVALID_ARG_VALUE",
                                &format!(
                                    "The property 'options.mode' must be one of: 'strip', 'transform'. Received '{other}'"
                                ),
                            ));
                        },
                    };
                }
            }

            if let Ok(sm_val) = obj.get::<_, Value>("sourceMap") {
                if !sm_val.is_undefined() {
                    let Some(b) = sm_val.as_bool() else {
                        return Err(throw_code_type_error(
                            &ctx,
                            "ERR_INVALID_ARG_TYPE",
                            &format!(
                                "The \"options.sourceMap\" property must be of type boolean. Received type {}",
                                js_type_name(&sm_val)
                            ),
                        ));
                    };
                    if opts.mode == StripMode::Strip && b {
                        return Err(throw_code_type_error(
                            &ctx,
                            "ERR_INVALID_ARG_VALUE",
                            "The property 'options.sourceMap' must be one of: false, undefined. Received true",
                        ));
                    }
                    opts.source_map = b;
                }
            }

            if let Ok(url_val) = obj.get::<_, Value>("sourceUrl") {
                if !url_val.is_undefined() {
                    let Some(s) = url_val.as_string() else {
                        return Err(throw_code_type_error(
                            &ctx,
                            "ERR_INVALID_ARG_TYPE",
                            &format!(
                                "The \"options.sourceUrl\" property must be of type string. Received type {}",
                                js_type_name(&url_val)
                            ),
                        ));
                    };
                    opts.source_url = Some(s.to_string()?);
                }
            }
        }
    }

    strip_ts(&source, &opts).map_err(|e| e.throw(&ctx))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokKind {
    Ident,
    Number,
    String,
    TemplateHead,   // `...${
    TemplateMiddle, // }...${
    TemplateTail,   // }...` or `...`
    Regex,
    Punct,
}

#[derive(Debug, Clone)]
struct Tok {
    kind: TokKind,
    start: usize,
    end: usize,
    line_break_before: bool,
}

fn is_id_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80
}

fn is_id_continue(b: u8) -> bool {
    is_id_start(b) || b.is_ascii_digit()
}

fn prev_allows_regex(prev: Option<&Tok>, src: &str) -> bool {
    let Some(t) = prev else {
        return true;
    };
    let text = &src[t.start..t.end];
    match t.kind {
        TokKind::Ident => matches!(
            text,
            "return"
                | "throw"
                | "case"
                | "delete"
                | "void"
                | "typeof"
                | "yield"
                | "await"
                | "new"
                | "in"
                | "instanceof"
                | "of"
                | "else"
                | "do"
                | "export"
                | "default"
                | "extends"
                | "satisfies"
                | "as"
        ),
        TokKind::TemplateHead | TokKind::TemplateMiddle => true,
        TokKind::Punct => !matches!(text, ")" | "]" | "}" | "++" | "--"),
        TokKind::Number | TokKind::String | TokKind::TemplateTail | TokKind::Regex => false,
    }
}

fn tokenize(src: &str) -> Vec<Tok> {
    let b = src.as_bytes();
    let len = b.len();
    let mut i = 0;
    let mut tokens: Vec<Tok> = Vec::new();
    let mut line_break = false;
    // Stack tracking nested `${ ... }` template interpolation brace depth.
    let mut template_brace_stack: Vec<usize> = Vec::new();
    let mut current_brace_depth = 0usize;

    if src.starts_with("#!") {
        while i < len && b[i] != b'\n' {
            i += 1;
        }
    }

    while i < len {
        let ch = b[i];

        if ch == b'\n' || ch == b'\r' {
            line_break = true;
            i += 1;
            continue;
        }
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Comments
        if ch == b'/' && i + 1 < len {
            if b[i + 1] == b'/' {
                i += 2;
                while i < len && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if b[i + 1] == b'*' {
                i += 2;
                while i + 1 < len && !(b[i] == b'*' && b[i + 1] == b'/') {
                    if b[i] == b'\n' {
                        line_break = true;
                    }
                    i += 1;
                }
                i = (i + 2).min(len);
                continue;
            }
        }

        // Strings
        if ch == b'\'' || ch == b'"' {
            let start = i;
            let quote = ch;
            i += 1;
            while i < len {
                if b[i] == b'\\' {
                    i = (i + 2).min(len);
                } else if b[i] == quote {
                    i += 1;
                    break;
                } else {
                    if b[i] == b'\n' {
                        line_break = true;
                    }
                    i += 1;
                }
            }
            tokens.push(Tok {
                kind: TokKind::String,
                start,
                end: i,
                line_break_before: line_break,
            });
            line_break = false;
            continue;
        }

        // Template literal start
        if ch == b'`' {
            let start = i;
            i += 1;
            let mut is_head = false;
            while i < len {
                if b[i] == b'\\' {
                    i = (i + 2).min(len);
                } else if b[i] == b'$' && i + 1 < len && b[i + 1] == b'{' {
                    i += 2;
                    is_head = true;
                    template_brace_stack.push(current_brace_depth);
                    break;
                } else if b[i] == b'`' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            tokens.push(Tok {
                kind: if is_head {
                    TokKind::TemplateHead
                } else {
                    TokKind::TemplateTail
                },
                start,
                end: i,
                line_break_before: line_break,
            });
            line_break = false;
            continue;
        }

        // Closing brace that may resume a template literal
        if ch == b'}' {
            if let Some(&saved_depth) = template_brace_stack.last() {
                if current_brace_depth == saved_depth {
                    template_brace_stack.pop();
                    let start = i;
                    i += 1;
                    let mut is_middle = false;
                    while i < len {
                        if b[i] == b'\\' {
                            i = (i + 2).min(len);
                        } else if b[i] == b'$' && i + 1 < len && b[i + 1] == b'{' {
                            i += 2;
                            is_middle = true;
                            template_brace_stack.push(current_brace_depth);
                            break;
                        } else if b[i] == b'`' {
                            i += 1;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    tokens.push(Tok {
                        kind: if is_middle {
                            TokKind::TemplateMiddle
                        } else {
                            TokKind::TemplateTail
                        },
                        start,
                        end: i,
                        line_break_before: line_break,
                    });
                    line_break = false;
                    continue;
                }
            }
            current_brace_depth = current_brace_depth.saturating_sub(1);
            let start = i;
            i += 1;
            tokens.push(Tok {
                kind: TokKind::Punct,
                start,
                end: i,
                line_break_before: line_break,
            });
            line_break = false;
            continue;
        }

        if ch == b'{' {
            current_brace_depth += 1;
        }

        // RegExp literal
        if ch == b'/' && prev_allows_regex(tokens.last(), src) {
            let start = i;
            i += 1;
            let mut in_class = false;
            while i < len {
                if b[i] == b'\\' {
                    i = (i + 2).min(len);
                } else if b[i] == b'[' {
                    in_class = true;
                    i += 1;
                } else if b[i] == b']' {
                    in_class = false;
                    i += 1;
                } else if b[i] == b'/' && !in_class {
                    i += 1;
                    while i < len && b[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    break;
                } else if b[i] == b'\n' {
                    break;
                } else {
                    i += 1;
                }
            }
            tokens.push(Tok {
                kind: TokKind::Regex,
                start,
                end: i,
                line_break_before: line_break,
            });
            line_break = false;
            continue;
        }

        // Identifier / keyword (including `#private`)
        if is_id_start(ch) || (ch == b'#' && i + 1 < len && is_id_start(b[i + 1])) {
            let start = i;
            i += 1;
            while i < len && is_id_continue(b[i]) {
                i += 1;
            }
            tokens.push(Tok {
                kind: TokKind::Ident,
                start,
                end: i,
                line_break_before: line_break,
            });
            line_break = false;
            continue;
        }

        // Numbers
        if ch.is_ascii_digit() || (ch == b'.' && i + 1 < len && b[i + 1].is_ascii_digit()) {
            let start = i;
            i += 1;
            while i < len
                && (b[i].is_ascii_alphanumeric()
                    || b[i] == b'_'
                    || b[i] == b'.'
                    || ((b[i] == b'+' || b[i] == b'-') && (b[i - 1] == b'e' || b[i - 1] == b'E')))
            {
                i += 1;
            }
            tokens.push(Tok {
                kind: TokKind::Number,
                start,
                end: i,
                line_break_before: line_break,
            });
            line_break = false;
            continue;
        }

        // Multi-char punctuators
        let start = i;
        let rest = &b[i..];
        let step = if rest.starts_with(b"...")
            || rest.starts_with(b"===")
            || rest.starts_with(b"!==")
            || rest.starts_with(b">>>")
            || rest.starts_with(b"**=")
            || rest.starts_with(b"<<=")
            || rest.starts_with(b">>=")
            || rest.starts_with(b"&&=")
            || rest.starts_with(b"||=")
            || rest.starts_with(b"??=")
        {
            3
        } else if rest.len() >= 2
            && matches!(
                &rest[..2],
                b"=>"
                    | b"=="
                    | b"!="
                    | b"<="
                    | b">="
                    | b"++"
                    | b"--"
                    | b"&&"
                    | b"||"
                    | b"??"
                    | b"?."
                    | b"**"
                    | b"<<"
                    | b">>"
                    | b"+="
                    | b"-="
                    | b"*="
                    | b"/="
                    | b"%="
                    | b"&="
                    | b"|="
                    | b"^="
            )
        {
            2
        } else {
            1
        };
        i += step;
        tokens.push(Tok {
            kind: TokKind::Punct,
            start,
            end: i,
            line_break_before: line_break,
        });
        line_break = false;
    }

    tokens
}

#[derive(Debug, Clone)]
enum Edit {
    Blank {
        start: usize,
        end: usize,
    },
    Replace {
        start: usize,
        end: usize,
        text: String,
    },
}

struct Stripper<'a> {
    src: &'a str,
    toks: Vec<Tok>,
    mode: StripMode,
    edits: Vec<Edit>,
}

impl<'a> Stripper<'a> {
    fn new(src: &'a str, mode: StripMode) -> Self {
        Self {
            src,
            toks: tokenize(src),
            mode,
            edits: Vec::new(),
        }
    }

    fn text(&self, idx: usize) -> &'a str {
        let t = &self.toks[idx];
        &self.src[t.start..t.end]
    }

    fn is_ident(&self, idx: usize, s: &str) -> bool {
        idx < self.toks.len() && self.toks[idx].kind == TokKind::Ident && self.text(idx) == s
    }

    fn is_punct(&self, idx: usize, s: &str) -> bool {
        idx < self.toks.len() && self.toks[idx].kind == TokKind::Punct && self.text(idx) == s
    }

    fn blank(&mut self, start: usize, end: usize) {
        if start < end {
            self.edits.push(Edit::Blank { start, end });
        }
    }

    fn replace(&mut self, start: usize, end: usize, text: String) {
        self.edits.push(Edit::Replace { start, end, text });
    }

    /// Skip matching `( ... )`, `[ ... ]`, or `{ ... }`. Returns index AFTER closing delim.
    fn skip_balanced(&self, start: usize) -> usize {
        let mut i = start;
        let open = self.text(i);
        let close = match open {
            "(" => ")",
            "[" => "]",
            "{" => "}",
            _ => return i + 1,
        };
        let mut depth = 0usize;
        while i < self.toks.len() {
            if self.toks[i].kind == TokKind::Punct {
                if self.text(i) == open {
                    depth += 1;
                } else if self.text(i) == close {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
            }
            i += 1;
        }
        self.toks.len()
    }

    /// Try to skip `<...>` generic type parameters or type arguments.
    fn try_skip_angles(&self, start: usize, is_decl_params: bool) -> Option<usize> {
        if !self.is_punct(start, "<") {
            return None;
        }
        let mut i = start + 1;
        let mut angle = 1isize;
        let mut paren = 0usize;
        let mut bracket = 0usize;
        let mut brace = 0usize;
        while i < self.toks.len() {
            let t = &self.toks[i];
            if t.kind == TokKind::Punct {
                match self.text(i) {
                    "(" => paren += 1,
                    ")" => paren = paren.saturating_sub(1),
                    "[" => bracket += 1,
                    "]" => bracket = bracket.saturating_sub(1),
                    "{" => brace += 1,
                    "}" => brace = brace.saturating_sub(1),
                    "<" if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
                    ">" if paren == 0 && bracket == 0 && brace == 0 => {
                        angle -= 1;
                        if angle == 0 {
                            let next = i + 1;
                            if is_decl_params {
                                if next < self.toks.len()
                                    && (self.is_punct(next, "(")
                                        || self.is_punct(next, "{")
                                        || self.is_ident(next, "extends")
                                        || self.is_ident(next, "implements")
                                        || self.is_punct(next, ";"))
                                {
                                    return Some(next);
                                }
                                return None;
                            } else {
                                if next < self.toks.len()
                                    && (self.is_punct(next, "(")
                                        || matches!(
                                            self.toks[next].kind,
                                            TokKind::TemplateHead | TokKind::TemplateTail
                                        )
                                        || self.is_punct(next, ",")
                                        || self.is_punct(next, ")")
                                        || self.is_punct(next, "]")
                                        || self.is_punct(next, ";"))
                                {
                                    return Some(next);
                                }
                                return None;
                            }
                        }
                    },
                    ">>" | ">>>" if paren == 0 && bracket == 0 && brace == 0 => {
                        let count = if self.text(i) == ">>>" { 3 } else { 2 };
                        if angle <= count {
                            return Some(i + 1);
                        }
                        angle -= count;
                    },
                    // Binary comparison or statement terminator cannot occur at top level of `<...>`
                    ";" if brace == 0 => return None,
                    _ => {},
                }
            }
            i += 1;
        }
        None
    }

    /// Scan a TypeScript type annotation starting at `start` (after `:`, `as`, `satisfies`, `=`).
    /// Returns the index of the first token that is NOT part of the type.
    fn scan_type_end(&self, start: usize, stop_at_top_assign: bool) -> usize {
        let mut i = start;
        let mut paren = 0usize;
        let mut bracket = 0usize;
        let mut brace = 0usize;
        let mut angle = 0usize;

        while i < self.toks.len() {
            let at_top = paren == 0 && bracket == 0 && brace == 0 && angle == 0;
            if at_top && i > start && self.toks[i].line_break_before {
                let prev = self.text(i - 1);
                let curr = self.text(i);
                let continues = matches!(
                    prev,
                    "|" | "&"
                        | "=>"
                        | "extends"
                        | "keyof"
                        | "typeof"
                        | "infer"
                        | "readonly"
                        | "unique"
                        | "is"
                        | "asserts"
                        | "?"
                ) || matches!(curr, "|" | "&" | "[" | "<" | "=>");
                if !continues {
                    // Check for ASI terminator: e.g. `type A = string\nconst x = 1`
                    if matches!(
                        self.toks[i].kind,
                        TokKind::Ident
                            | TokKind::Number
                            | TokKind::String
                            | TokKind::TemplateHead
                            | TokKind::TemplateTail
                            | TokKind::Regex
                    ) || matches!(curr, ";" | "}" | ")" | "]" | "{" | "(")
                    {
                        break;
                    }
                }
            }

            if at_top {
                if self.is_punct(i, ";")
                    || self.is_punct(i, ",")
                    || self.is_punct(i, ")")
                    || self.is_punct(i, "]")
                    || self.is_punct(i, "}")
                    || (stop_at_top_assign && self.is_punct(i, "="))
                    || (self.is_punct(i, "=>") && !self.is_paren_fn_type_before(start, i))
                {
                    break;
                }
                if self.is_punct(i, "{") {
                    // `{` can be an object type `{ a: number }` OR a function/class body `{ ... }`
                    // Heuristic: if preceded immediately by `)` or `>` or `keyof`/etc. after return-type
                    // OR if preceded by identifier/type end without `&`/`|`/`extends`, wait:
                    // - function f(): T { body } -> after `T`, `{` starts the function BODY!
                    // - const x: { a: number } = ... -> at `start`, `{` starts an object type!
                    // - const x: Foo & { a: number } -> after `&`, `{` starts an intersection object type!
                    // - type A = T extends U ? X : { a: 1 } -> after `?`/`:`, `{` starts an object type!
                    if i > start {
                        let prev = self.text(i - 1);
                        if !matches!(prev, "|" | "&" | "?" | ":" | "=>" | "=" | "extends") {
                            break;
                        }
                    }
                }
                if self.is_ident(i, "implements")
                    || self.is_ident(i, "of")
                    || self.is_ident(i, "in")
                {
                    // `in` is valid inside mapped `{ [K in T]: V }` (brace > 0), not at top level (`for (let x: any in obj)`).
                    break;
                }
                // Conditional type `A extends B ? C : D`: `:` at top level belongs to conditional ONLY if matched by a `?` inside this type!
                if self.is_punct(i, ":") {
                    // Top-level `:` ends type if it wasn't opened by `?` in this type (e.g. ternary `cond ? (a as T) : b`).
                    let mut q_balance = 0isize;
                    for j in start..i {
                        if self.is_punct(j, "?") {
                            q_balance += 1;
                        } else if self.is_punct(j, ":") {
                            q_balance -= 1;
                        }
                    }
                    if q_balance <= 0 {
                        break;
                    }
                }
                if self.is_punct(i, "?") {
                    // Wait: if `as T ? a : b` ternary follows `as T`, `?` is NOT part of `T` unless `extends` occurred before!
                    let has_extends = (start..i).any(|j| self.is_ident(j, "extends"));
                    let is_optional_param = i + 1 < self.toks.len()
                        && (self.is_punct(i + 1, ",")
                            || self.is_punct(i + 1, ")")
                            || self.is_punct(i + 1, "="));
                    if is_optional_param || !has_extends {
                        break;
                    }
                }
            }

            let txt = self.text(i);
            if self.toks[i].kind == TokKind::Punct {
                match txt {
                    "(" => paren += 1,
                    ")" => paren = paren.saturating_sub(1),
                    "[" => bracket += 1,
                    "]" => bracket = bracket.saturating_sub(1),
                    "{" => brace += 1,
                    "}" => brace = brace.saturating_sub(1),
                    "<" if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
                    ">" if paren == 0 && bracket == 0 && brace == 0 => {
                        if angle == 0 {
                            break;
                        }
                        angle -= 1;
                    },
                    _ => {},
                }
            }
            i += 1;
        }
        i
    }

    fn is_paren_fn_type_before(&self, start: usize, arrow_idx: usize) -> bool {
        if arrow_idx == 0 {
            return false;
        }
        // In `: (a: number) => string`, the token right after `:` is `(` (or `<`), and arrow sits before any body.
        self.is_punct(start, "(")
            || self.is_punct(start, "<")
            || (start + 1 < arrow_idx && self.is_punct(arrow_idx - 1, ")"))
    }

    /// Scan to end of statement (`;` or matching `}` or ASI newline) for declarations like `type ...`, `import type ...`.
    fn scan_statement_end(&self, start: usize) -> usize {
        let mut i = start;
        let mut paren = 0usize;
        let mut bracket = 0usize;
        let mut brace = 0usize;
        let mut angle = 0usize;

        while i < self.toks.len() {
            let at_top = paren == 0 && bracket == 0 && brace == 0 && angle == 0;
            if at_top && i > start && self.toks[i].line_break_before {
                let prev = self.text(i - 1);
                if !matches!(
                    prev,
                    "=" | "|" | "&" | "," | "(" | "[" | "{" | "<" | "from" | "import" | "export"
                ) {
                    return i;
                }
            }
            if at_top && self.is_punct(i, ";") {
                return i + 1;
            }
            if self.toks[i].kind == TokKind::Punct {
                match self.text(i) {
                    "(" => paren += 1,
                    ")" => paren = paren.saturating_sub(1),
                    "[" => bracket += 1,
                    "]" => bracket = bracket.saturating_sub(1),
                    "{" => brace += 1,
                    "}" => {
                        if brace == 0 {
                            return i;
                        }
                        brace -= 1;
                    },
                    "<" if paren == 0 && bracket == 0 && brace == 0 => angle += 1,
                    ">" if paren == 0 && bracket == 0 && brace == 0 => {
                        angle = angle.saturating_sub(1);
                    },
                    _ => {},
                }
            }
            i += 1;
        }
        self.toks.len()
    }

    fn process(&mut self) -> std::result::Result<(), TsStripError> {
        let end = self.toks.len();
        self.process_range(0, end, false, None)?;
        Ok(())
    }

    fn process_range(
        &mut self,
        start: usize,
        end: usize,
        in_class_body: bool,
        mut namespace_exports: Option<&mut HashSet<String>>,
    ) -> std::result::Result<(), TsStripError> {
        let mut i = start;
        while i < end {
            // Check `<Type>expr` syntax? Wait: distinguish `<T,>(x) => ...` (allowed TS generic arrow!)
            // and `fn<T>(x)` (type args) from `<T>expr` angle-bracket type assertion!
            if self.is_punct(i, "<") {
                let prev = if i > 0 { Some(&self.toks[i - 1]) } else { None };
                if let Some(after) = self.try_skip_angles(i, true) {
                    if self.is_punct(after, "(") {
                        // Could be `<T>(x) => ...` generic arrow function! Strip `<T>`.
                        self.blank(self.toks[i].start, self.toks[after].start);
                        i = after;
                        continue;
                    }
                }
                if prev_allows_regex(prev, self.src) {
                    // e.g. `const x = <number>y;`
                    if let Some(after_gt) = self.scan_angle_assertion(i) {
                        if after_gt < end
                            && matches!(
                                self.toks[after_gt].kind,
                                TokKind::Ident
                                    | TokKind::Number
                                    | TokKind::String
                                    | TokKind::TemplateHead
                                    | TokKind::TemplateTail
                            )
                        {
                            return Err(TsStripError::unsupported(
                                "The angle-bracket syntax for type assertions, `<T>expr`, is not supported in type strip mode. Instead, use the 'as' syntax: `expr as T`.",
                            ));
                        }
                    }
                } else if prev.is_some_and(|p| {
                    p.kind == TokKind::Ident
                        || &self.src[p.start..p.end] == ")"
                        || &self.src[p.start..p.end] == "]"
                }) {
                    if let Some(after) = self.try_skip_angles(i, false) {
                        self.blank(self.toks[i].start, self.toks[after].start);
                        i = after;
                        continue;
                    }
                }
            }

            if self.is_punct(i, "(") {
                let is_call = i > start
                    && (matches!(
                        self.toks[i - 1].kind,
                        TokKind::Ident | TokKind::Number | TokKind::String | TokKind::Regex
                    ) && !matches!(
                        self.text(i - 1),
                        "return" | "throw" | "yield" | "await" | "case"
                    ) || matches!(self.text(i - 1), ")" | "]"));
                if !is_call {
                    if let Some(after) = self.try_handle_arrow_params(i, end)? {
                        i = after;
                        continue;
                    }
                }
            }

            if self.toks[i].kind != TokKind::Ident {
                // Non-null assertion `expr!`
                if self.is_punct(i, "!") && i > start {
                    let prev_tok = &self.toks[i - 1];
                    let prev_txt = &self.src[prev_tok.start..prev_tok.end];
                    let postfix_prev = matches!(
                        prev_tok.kind,
                        TokKind::Ident | TokKind::Number | TokKind::String | TokKind::TemplateTail
                    ) || matches!(prev_txt, ")" | "]" | "!");
                    if postfix_prev && !self.toks[i].line_break_before {
                        let next_is_expr = i + 1 < end
                            && !self.toks[i + 1].line_break_before
                            && (matches!(
                                self.toks[i + 1].kind,
                                TokKind::Ident | TokKind::Number | TokKind::String
                            ) && !matches!(
                                self.text(i + 1),
                                "as" | "satisfies" | "in" | "instanceof"
                            ));
                        if !next_is_expr {
                            self.blank(self.toks[i].start, self.toks[i].end);
                            i += 1;
                            continue;
                        }
                    }
                }
                i += 1;
                continue;
            }

            let kw = self.text(i);
            let at_stmt_start = i == start
                || self.toks[i].line_break_before
                || matches!(self.text(i - 1), ";" | "{" | "}" | ")")
                || (matches!(self.text(i - 1), "export" | "default")
                    && (i < 2 || !self.is_punct(i - 2, ".")));

            // `import`
            if kw == "import" && at_stmt_start {
                if self.is_ident(i + 1, "type") && (i + 2 < end && !self.is_punct(i + 2, "from")) {
                    let stmt_end = self.scan_statement_end(i);
                    self.blank(self.toks[i].start, self.toks[stmt_end - 1].end);
                    i = stmt_end;
                    continue;
                }
                // `import foo = ...`
                if i + 2 < end && self.is_punct(i + 2, "=") {
                    if self.mode == StripMode::Strip {
                        return Err(TsStripError::unsupported(
                            "TypeScript import equals declaration is not supported in strip-only mode",
                        ));
                    }
                    let stmt_end = self.scan_statement_end(i);
                    let id = self.text(i + 1).to_string();
                    let rhs_start = self.toks[i + 3].start;
                    let rhs_end = if self.is_punct(stmt_end - 1, ";") {
                        self.toks[stmt_end - 1].start
                    } else {
                        self.toks[stmt_end - 1].end
                    };
                    let rhs = &self.src[rhs_start..rhs_end];
                    self.replace(
                        self.toks[i].start,
                        self.toks[stmt_end - 1].end,
                        format!("const {id} = {rhs};"),
                    );
                    i = stmt_end;
                    continue;
                }
                // Named specifier stripping: `import { type A, B } from ...`
                if i + 1 < end && self.is_punct(i + 1, "{") {
                    i = self.strip_import_export_braces(i + 1, end);
                    continue;
                }
                if i + 3 < end && self.is_punct(i + 2, ",") && self.is_punct(i + 3, "{") {
                    i = self.strip_import_export_braces(i + 3, end);
                    continue;
                }
            }

            // `export`
            if kw == "export" && at_stmt_start {
                if self.is_ident(i + 1, "type")
                    && (i + 2 < end
                        && (self.is_punct(i + 2, "{")
                            || self.is_punct(i + 2, "*")
                            || self.toks[i + 2].kind == TokKind::Ident))
                {
                    let stmt_end = self.scan_statement_end(i);
                    self.blank(self.toks[i].start, self.toks[stmt_end - 1].end);
                    i = stmt_end;
                    continue;
                }
                if self.is_ident(i + 1, "interface") || self.is_ident(i + 1, "declare") {
                    let next = i + 1;
                    i = self.strip_ts_declaration(i, next, end)?;
                    continue;
                }
                if self.is_punct(i + 1, "{") {
                    i = self.strip_import_export_braces(i + 1, end);
                    continue;
                }
                if let Some(ref mut ns_exports) = namespace_exports {
                    // Inside `namespace N { export const x = 1; export function f() {} }`
                    if let Some(after) = self.record_namespace_export(i, end, ns_exports)? {
                        i = after;
                        continue;
                    }
                }
            }

            // Type-only statements: `type Foo = ...`, `interface Foo { ... }`, `declare ...`
            if (kw == "type" || kw == "interface" || kw == "declare") && at_stmt_start {
                let maybe_next = self.strip_ts_declaration(i, i, end)?;
                if maybe_next > i {
                    i = maybe_next;
                    continue;
                }
            }

            // `enum` / `const enum`
            if (kw == "enum" && at_stmt_start)
                || (kw == "const" && at_stmt_start && self.is_ident(i + 1, "enum"))
            {
                let enum_kw = if kw == "const" { i + 1 } else { i };
                if self.mode == StripMode::Strip {
                    return Err(TsStripError::unsupported(
                        "TypeScript enum is not supported in strip-only mode",
                    ));
                }
                i = self.transform_enum(i, enum_kw, end)?;
                continue;
            }

            // `namespace` / `module Foo {`
            if (kw == "namespace" || kw == "module")
                && at_stmt_start
                && i + 1 < end
                && self.toks[i + 1].kind == TokKind::Ident
            {
                if self.mode == StripMode::Strip {
                    return Err(TsStripError::unsupported(
                        "TypeScript namespace is not supported in strip-only mode",
                    ));
                }
                i = self.transform_namespace(i, end)?;
                continue;
            }

            // `abstract class ...`
            if kw == "abstract" && at_stmt_start && self.is_ident(i + 1, "class") {
                self.blank(self.toks[i].start, self.toks[i].end);
                i += 1;
                continue;
            }

            // `class Foo<T> extends Bar<U> implements Baz, Qux { ... }`
            if kw == "class" {
                i = self.handle_class(i, end)?;
                continue;
            }

            // `function name<T>(params): Return { ... }` or `function<T>(params): Return`
            if kw == "function" {
                i = self.handle_function_header(i, end)?;
                continue;
            }

            // Class body members: modifiers (`public`, `private`, `protected`, `readonly`, `override`, `abstract`, `declare`), fields `x?: T;` / `x!: T;`
            if in_class_body && at_stmt_start {
                if let Some(after) = self.handle_class_member_prefix(i, end)? {
                    i = after;
                    continue;
                }
            }

            // Arrow function `async (params): Ret =>` or `(params): Ret =>`
            if kw == "async" && self.is_punct(i + 1, "(") {
                if let Some(after) = self.try_handle_arrow_params(i + 1, end)? {
                    i = after;
                    continue;
                }
            }

            // `expr as Type` / `expr satisfies Type`
            if (kw == "as" || kw == "satisfies") && i > start && !self.toks[i].line_break_before {
                let prev_txt = self.text(i - 1);
                if prev_txt != "." && prev_txt != "{" && prev_txt != "," {
                    let type_end = self.scan_type_end(i + 1, false);
                    if type_end > i + 1 {
                        self.blank(self.toks[i].start, self.toks[type_end - 1].end);
                        i = type_end;
                        continue;
                    }
                }
            }

            // `let` / `const` / `var` variable type annotation: `const x: T = ...` or `let x!: T`
            if matches!(kw, "const" | "let" | "var")
                && (at_stmt_start || (i > start && self.is_punct(i - 1, "(")))
            {
                i = self.handle_var_decl(i + 1, end);
                continue;
            }

            i += 1;
        }
        Ok(())
    }

    fn scan_angle_assertion(&self, start: usize) -> Option<usize> {
        let mut i = start + 1;
        let mut depth = 1isize;
        while i < self.toks.len() {
            if self.toks[i].kind == TokKind::Punct {
                match self.text(i) {
                    "<" => depth += 1,
                    ">" => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    },
                    ";" => return None,
                    _ => {},
                }
            }
            i += 1;
        }
        None
    }

    fn strip_import_export_braces(&mut self, brace_idx: usize, end: usize) -> usize {
        let close = self.skip_balanced(brace_idx).min(end);
        let mut i = brace_idx + 1;
        while i + 1 < close {
            if self.is_ident(i, "type")
                && self.toks[i + 1].kind == TokKind::Ident
                && !self.is_ident(i + 1, "as")
                && !self.is_punct(i + 1, ",")
                && !self.is_punct(i + 1, "}")
            {
                let spec_start = self.toks[i].start;
                let mut j = i + 2;
                if j < close && self.is_ident(j, "as") && j + 1 < close {
                    j += 2;
                }
                let spec_end = if j < close && self.is_punct(j, ",") {
                    j += 1;
                    self.toks[j - 1].end
                } else {
                    self.toks[j - 1].end
                };
                self.blank(spec_start, spec_end);
                i = j;
                continue;
            }
            i += 1;
        }
        close
    }

    fn strip_ts_declaration(
        &mut self,
        stmt_start: usize,
        kw_idx: usize,
        end: usize,
    ) -> std::result::Result<usize, TsStripError> {
        let kw = self.text(kw_idx);
        if kw == "interface" {
            let mut i = kw_idx + 1;
            if i < end && self.toks[i].kind == TokKind::Ident {
                while i < end && !self.is_punct(i, "{") {
                    i += 1;
                }
                if i < end && self.is_punct(i, "{") {
                    let close = self.skip_balanced(i);
                    self.blank(self.toks[stmt_start].start, self.toks[close - 1].end);
                    return Ok(close);
                }
            }
            return Ok(stmt_start);
        }
        if kw == "type" {
            let mut i = kw_idx + 1;
            if i >= end || self.toks[i].kind != TokKind::Ident || self.toks[i].line_break_before {
                return Ok(stmt_start);
            }
            i += 1;
            if let Some(after) = self.try_skip_angles(i, false) {
                i = after;
            }
            if i < end && self.is_punct(i, "=") {
                let stmt_end = self.scan_statement_end(i + 1);
                self.blank(self.toks[stmt_start].start, self.toks[stmt_end - 1].end);
                return Ok(stmt_end);
            }
            return Ok(stmt_start);
        }
        if kw == "declare"
            && kw_idx + 1 < end
            && (self.toks[kw_idx + 1].kind == TokKind::Ident || self.is_ident(kw_idx + 1, "global"))
            && !self.toks[kw_idx + 1].line_break_before
        {
            let next = kw_idx + 1;
            // `declare module "foo" { ... }` or `declare function f(): void;`
            let mut j = next;
            while j < end {
                if self.is_punct(j, "{") {
                    let close = self.skip_balanced(j);
                    self.blank(self.toks[stmt_start].start, self.toks[close - 1].end);
                    return Ok(close);
                }
                if self.is_punct(j, ";")
                    || (j > next
                        && self.toks[j].line_break_before
                        && self.is_ambient_stmt_end(next, j))
                {
                    let end_byte = if self.is_punct(j, ";") {
                        self.toks[j].end
                    } else {
                        self.toks[j - 1].end
                    };
                    let next_idx = if self.is_punct(j, ";") { j + 1 } else { j };
                    self.blank(self.toks[stmt_start].start, end_byte);
                    return Ok(next_idx);
                }
                j += 1;
            }
            self.blank(self.toks[stmt_start].start, self.toks[end - 1].end);
            return Ok(end);
        }
        Ok(stmt_start)
    }

    fn is_ambient_stmt_end(&self, _start: usize, idx: usize) -> bool {
        let prev = self.text(idx - 1);
        !matches!(prev, "=" | ":" | "|" | "&" | "," | "extends" | "implements")
    }

    fn handle_class(
        &mut self,
        class_kw: usize,
        end: usize,
    ) -> std::result::Result<usize, TsStripError> {
        let mut i = class_kw + 1;
        if i < end
            && self.toks[i].kind == TokKind::Ident
            && !self.is_ident(i, "extends")
            && !self.is_ident(i, "implements")
        {
            i += 1;
        }
        if let Some(after_gen) = self.try_skip_angles(i, true) {
            self.blank(self.toks[i].start, self.toks[after_gen].start);
            i = after_gen;
        }
        if self.is_ident(i, "extends") {
            i += 1;
            // Skip base expression and optional generic type arguments `extends Foo<Bar>`
            while i < end && !self.is_punct(i, "{") && !self.is_ident(i, "implements") {
                if self.is_punct(i, "<") {
                    if let Some(after_gen) = self.try_skip_angles(i, true) {
                        self.blank(self.toks[i].start, self.toks[after_gen].start);
                        i = after_gen;
                        continue;
                    }
                }
                i += 1;
            }
        }
        if self.is_ident(i, "implements") {
            let impl_start = self.toks[i].start;
            i += 1;
            while i < end && !self.is_punct(i, "{") {
                i += 1;
            }
            if i < end {
                self.blank(impl_start, self.toks[i].start);
            }
        }
        if i < end && self.is_punct(i, "{") {
            let close = self.skip_balanced(i);
            self.process_range(i + 1, close - 1, true, None)?;
            return Ok(close);
        }
        Ok(i)
    }

    fn handle_class_member_prefix(
        &mut self,
        start: usize,
        end: usize,
    ) -> std::result::Result<Option<usize>, TsStripError> {
        let mut i = start;
        let mut has_abstract = false;
        let mut has_declare = false;
        let mut blank_end = None;

        while i < end && self.toks[i].kind == TokKind::Ident {
            let m = self.text(i);
            if matches!(
                m,
                "public" | "private" | "protected" | "readonly" | "override"
            ) {
                // Only modifier if followed by another member token (not `(` or `:` or `;` or `=`)
                if i + 1 < end
                    && !self.is_punct(i + 1, "(")
                    && !self.is_punct(i + 1, ":")
                    && !self.is_punct(i + 1, ";")
                    && !self.is_punct(i + 1, "=")
                {
                    blank_end = Some(self.toks[i].end);
                    i += 1;
                    continue;
                }
                break;
            } else if m == "abstract" {
                has_abstract = true;
                blank_end = Some(self.toks[i].end);
                i += 1;
            } else if m == "declare" {
                has_declare = true;
                blank_end = Some(self.toks[i].end);
                i += 1;
            } else {
                break;
            }
        }

        if has_abstract || has_declare {
            let stmt_end = self.scan_statement_end(i);
            self.blank(self.toks[start].start, self.toks[stmt_end - 1].end);
            return Ok(Some(stmt_end));
        }

        if let Some(bend) = blank_end {
            self.blank(self.toks[start].start, bend);
        }

        // Constructor parameter properties
        if self.is_ident(i, "constructor") && i + 1 < end && self.is_punct(i + 1, "(") {
            let _close_paren = self.skip_balanced(i + 1);
            let mut param_props = Vec::new();
            let after_params = self.handle_params(i + 1, &mut param_props)?;
            let mut body_start = after_params;
            if self.is_punct(body_start, ":") {
                let tend = self.scan_type_end(body_start + 1, false);
                self.blank(self.toks[body_start].start, self.toks[tend - 1].end);
                body_start = tend;
            }
            if body_start < end && self.is_punct(body_start, ";") {
                // Overload signature without body
                self.blank(self.toks[start].start, self.toks[body_start].end);
                return Ok(Some(body_start + 1));
            }
            if !param_props.is_empty() && body_start < end && self.is_punct(body_start, "{") {
                let body_close = self.skip_balanced(body_start);
                // Declare fields before `constructor`
                let fields: String = param_props.iter().map(|p| format!("{p};\n    ")).collect();
                let ctor_pos = self.toks[start].start;
                self.replace(ctor_pos, ctor_pos, fields);
                // Insert `this.x = x;` after `super(...)` if present, else right after `{`
                let mut insert_pos = self.toks[body_start].end;
                for k in (body_start + 1)..(body_close - 1) {
                    if self.is_ident(k, "super") && k + 1 < body_close && self.is_punct(k + 1, "(")
                    {
                        let super_end = self.skip_balanced(k + 1);
                        insert_pos = if super_end < body_close && self.is_punct(super_end, ";") {
                            self.toks[super_end].end
                        } else {
                            self.toks[super_end - 1].end
                        };
                        break;
                    }
                }
                let assigns: String = param_props
                    .iter()
                    .map(|p| format!("\n        this.{p} = {p};"))
                    .collect();
                self.replace(insert_pos, insert_pos, assigns);
                self.process_range(body_start + 1, body_close - 1, false, None)?;
                return Ok(Some(body_close));
            }
            return Ok(Some(body_start));
        }

        // Method or field with `?`, `!`, `:`, or generic `<T>(`
        if i < end
            && (self.toks[i].kind == TokKind::Ident || self.is_punct(i, "["))
            && (i == start || blank_end.is_some())
        {
            let after_name = if self.is_punct(i, "[") {
                self.skip_balanced(i)
            } else {
                i + 1
            };
            if after_name < end {
                // Method: `foo<T>(a: number): string { ... }` or overload signature `foo(a: number): string;`
                let mut j = after_name;
                if self.is_punct(j, "?") {
                    // Optional method/property
                }
                if let Some(after_gen) = self.try_skip_angles(j, true) {
                    self.blank(self.toks[j].start, self.toks[after_gen].start);
                    j = after_gen;
                }
                if j < end && self.is_punct(j, "(") {
                    let mut ignored = Vec::new();
                    let after_paren = self.handle_params(j, &mut ignored)?;
                    let mut after_ret = after_paren;
                    if after_ret < end && self.is_punct(after_ret, ":") {
                        let tend = self.scan_type_end(after_ret + 1, false);
                        self.blank(self.toks[after_ret].start, self.toks[tend - 1].end);
                        after_ret = tend;
                    }
                    if after_ret < end && self.is_punct(after_ret, ";") {
                        // Overload signature
                        self.blank(self.toks[start].start, self.toks[after_ret].end);
                        return Ok(Some(after_ret + 1));
                    }
                    return Ok(Some(after_ret));
                }
                // Field: `x?: number = 1;` or `x!: number;` or `x: number = 1;`
                if j < end && (self.is_punct(j, "?") || self.is_punct(j, "!")) {
                    if j + 1 < end && self.is_punct(j + 1, ":") {
                        let tend = self.scan_type_end(j + 2, true);
                        self.blank(self.toks[j].start, self.toks[tend - 1].end);
                        return Ok(Some(tend));
                    }
                    if self.is_punct(j, "?") {
                        self.blank(self.toks[j].start, self.toks[j].end);
                        return Ok(Some(j + 1));
                    }
                }
                if j < end && self.is_punct(j, ":") {
                    let tend = self.scan_type_end(j + 1, true);
                    self.blank(self.toks[j].start, self.toks[tend - 1].end);
                    return Ok(Some(tend));
                }
            }
        }

        Ok(None)
    }

    fn handle_function_header(
        &mut self,
        fn_kw: usize,
        end: usize,
    ) -> std::result::Result<usize, TsStripError> {
        let mut i = fn_kw + 1;
        if i < end && self.is_punct(i, "*") {
            i += 1;
        }
        if i < end && self.toks[i].kind == TokKind::Ident {
            i += 1;
        }
        if let Some(after_gen) = self.try_skip_angles(i, true) {
            self.blank(self.toks[i].start, self.toks[after_gen].start);
            i = after_gen;
        }
        if i < end && self.is_punct(i, "(") {
            let mut ignored = Vec::new();
            let after_paren = self.handle_params(i, &mut ignored)?;
            let mut after_ret = after_paren;
            if after_ret < end && self.is_punct(after_ret, ":") {
                let tend = self.scan_type_end(after_ret + 1, false);
                self.blank(self.toks[after_ret].start, self.toks[tend - 1].end);
                after_ret = tend;
            }
            if after_ret < end && self.is_punct(after_ret, ";") {
                // Function overload signature
                self.blank(self.toks[fn_kw].start, self.toks[after_ret].end);
                return Ok(after_ret + 1);
            }
            return Ok(after_ret);
        }
        Ok(i)
    }

    fn try_handle_arrow_params(
        &mut self,
        open_paren: usize,
        end: usize,
    ) -> std::result::Result<Option<usize>, TsStripError> {
        let close = self.skip_balanced(open_paren);
        if close >= end {
            return Ok(None);
        }
        let mut after_ret = close;
        if self.is_punct(close, ":") {
            let tend = self.scan_type_end(close + 1, false);
            if tend >= end || !self.is_punct(tend, "=>") {
                return Ok(None);
            }
            after_ret = tend;
        }
        if !self.is_punct(after_ret, "=>") {
            return Ok(None);
        }
        let mut ignored = Vec::new();
        self.handle_params(open_paren, &mut ignored)?;
        if after_ret > close {
            self.blank(self.toks[close].start, self.toks[after_ret - 1].end);
        }
        Ok(Some(after_ret))
    }

    fn handle_params(
        &mut self,
        open_paren: usize,
        param_props: &mut Vec<String>,
    ) -> std::result::Result<usize, TsStripError> {
        let close = self.skip_balanced(open_paren);
        let mut i = open_paren + 1;
        let limit = close.saturating_sub(1);

        while i < limit {
            // Parameter property modifiers (`public`/`private`/`protected`/`readonly`)
            let mut is_param_prop = false;
            let mut mod_start = None;
            let mut mod_end = 0;
            while i < limit
                && self.toks[i].kind == TokKind::Ident
                && matches!(
                    self.text(i),
                    "public" | "private" | "protected" | "readonly" | "override"
                )
            {
                is_param_prop = true;
                if mod_start.is_none() {
                    mod_start = Some(self.toks[i].start);
                }
                mod_end = self.toks[i].end;
                i += 1;
            }
            if is_param_prop {
                if self.mode == StripMode::Strip {
                    return Err(TsStripError::unsupported(
                        "TypeScript parameter properties are not supported in strip-only mode",
                    ));
                }
                if let Some(ms) = mod_start {
                    self.blank(ms, mod_end);
                }
                if i < limit && self.toks[i].kind == TokKind::Ident {
                    param_props.push(self.text(i).to_string());
                }
            }

            // `this: Foo` parameter
            if i < limit && self.is_ident(i, "this") && i + 1 < limit && self.is_punct(i + 1, ":") {
                let tend = self.scan_type_end(i + 2, false).min(limit);
                let del_end = if tend < limit && self.is_punct(tend, ",") {
                    self.toks[tend].end
                } else {
                    self.toks[tend - 1].end
                };
                self.blank(self.toks[i].start, del_end);
                i = tend + usize::from(tend < limit && self.is_punct(tend, ","));
                continue;
            }

            // Advance inside parameter until top-level `,` or `=` or `:` or `?`
            let mut paren = 0usize;
            let mut bracket = 0usize;
            let mut brace = 0usize;
            while i < limit {
                let at_top = paren == 0 && bracket == 0 && brace == 0;
                if at_top && self.is_punct(i, "?") {
                    let qstart = self.toks[i].start;
                    if i + 1 < limit && self.is_punct(i + 1, ":") {
                        let tend = self.scan_type_end(i + 2, true).min(limit);
                        self.blank(qstart, self.toks[tend - 1].end);
                        i = tend;
                        continue;
                    }
                    self.blank(qstart, self.toks[i].end);
                    i += 1;
                    continue;
                }
                if at_top && self.is_punct(i, ":") {
                    let tend = self.scan_type_end(i + 1, true).min(limit);
                    self.blank(self.toks[i].start, self.toks[tend - 1].end);
                    i = tend;
                    continue;
                }
                if at_top && self.is_punct(i, ",") {
                    i += 1;
                    break;
                }
                if self.toks[i].kind == TokKind::Punct {
                    match self.text(i) {
                        "(" => paren += 1,
                        ")" => paren = paren.saturating_sub(1),
                        "[" => bracket += 1,
                        "]" => bracket = bracket.saturating_sub(1),
                        "{" => brace += 1,
                        "}" => brace = brace.saturating_sub(1),
                        _ => {},
                    }
                }
                i += 1;
            }
        }

        Ok(close)
    }

    fn handle_var_decl(&mut self, start: usize, end: usize) -> usize {
        let mut i = start;
        let mut paren = 0usize;
        let mut bracket = 0usize;
        let mut brace = 0usize;
        while i < end {
            let at_top = paren == 0 && bracket == 0 && brace == 0;
            if at_top {
                if self.is_punct(i, ";") {
                    return i + 1;
                }
                if i > start && self.toks[i].line_break_before {
                    return i;
                }
                if self.is_punct(i, ":") {
                    let tend = self.scan_type_end(i + 1, true).min(end);
                    let blank_start = if i > start && self.is_punct(i - 1, "!") {
                        self.toks[i - 1].start
                    } else {
                        self.toks[i].start
                    };
                    self.blank(blank_start, self.toks[tend - 1].end);
                    return tend;
                }
                if self.is_punct(i, "=") || self.is_ident(i, "in") || self.is_ident(i, "of") {
                    return i;
                }
            }
            if self.toks[i].kind == TokKind::Punct {
                match self.text(i) {
                    "(" => paren += 1,
                    ")" => paren = paren.saturating_sub(1),
                    "[" => bracket += 1,
                    "]" => bracket = bracket.saturating_sub(1),
                    "{" => brace += 1,
                    "}" => brace = brace.saturating_sub(1),
                    _ => {},
                }
            }
            i += 1;
        }
        i
    }

    fn transform_enum(
        &mut self,
        decl_start: usize,
        enum_kw: usize,
        end: usize,
    ) -> std::result::Result<usize, TsStripError> {
        let name_idx = enum_kw + 1;
        if name_idx + 1 >= end || !self.is_punct(name_idx + 1, "{") {
            return Ok(enum_kw + 1);
        }
        let name = self.text(name_idx);
        let body_open = name_idx + 1;
        let body_close = self.skip_balanced(body_open);
        let mut members_out = Vec::new();
        let mut next_num: i64 = 0;

        let mut i = body_open + 1;
        let limit = body_close.saturating_sub(1);
        while i < limit {
            let member_name = match self.toks[i].kind {
                TokKind::Ident => self.text(i).to_string(),
                TokKind::String => {
                    let s = self.text(i);
                    s[1..s.len() - 1].to_string()
                },
                _ => {
                    i += 1;
                    continue;
                },
            };
            i += 1;
            let val_expr: Option<String>;
            if i < limit && self.is_punct(i, "=") {
                i += 1;
                let expr_start = self.toks[i].start;
                let mut paren = 0usize;
                while i < limit {
                    if paren == 0 && self.is_punct(i, ",") {
                        break;
                    }
                    if self.is_punct(i, "(") {
                        paren += 1;
                    } else if self.is_punct(i, ")") {
                        paren = paren.saturating_sub(1);
                    }
                    i += 1;
                }
                let expr_end = self.toks[i - 1].end;
                let raw = self.src[expr_start..expr_end].trim();
                if let Ok(n) = raw.parse::<i64>() {
                    next_num = n + 1;
                    val_expr = Some(n.to_string());
                } else {
                    val_expr = Some(raw.to_string());
                }
            } else {
                let n = next_num;
                next_num += 1;
                val_expr = Some(n.to_string());
            }
            if i < limit && self.is_punct(i, ",") {
                i += 1;
            }
            let v = val_expr.unwrap();
            let quoted_name = format!(
                "\"{}\"",
                member_name.replace('\\', "\\\\").replace('"', "\\\"")
            );
            let is_str = v.starts_with('"') || v.starts_with('\'');
            if is_str {
                let norm = if v.starts_with('\'') && v.len() >= 2 {
                    format!("\"{}\"", &v[1..v.len() - 1])
                } else {
                    v
                };
                members_out.push(format!("    {name}[{quoted_name}] = {norm};"));
            } else {
                members_out.push(format!(
                    "    {name}[{name}[{quoted_name}] = {v}] = {quoted_name};"
                ));
            }
        }

        let mut js = format!("var {name} = /*#__PURE__*/ function({name}) {{\n");
        for m in members_out {
            js.push_str(&m);
            js.push('\n');
        }
        js.push_str(&format!("    return {name};\n}}({name} || {{}});"));
        self.replace(
            self.toks[decl_start].start,
            self.toks[body_close - 1].end,
            js,
        );
        Ok(body_close)
    }

    fn transform_namespace(
        &mut self,
        ns_kw: usize,
        end: usize,
    ) -> std::result::Result<usize, TsStripError> {
        let name_idx = ns_kw + 1;
        if name_idx + 1 >= end || !self.is_punct(name_idx + 1, "{") {
            return Ok(ns_kw + 1);
        }
        let name = self.text(name_idx).to_string();
        let open = name_idx + 1;
        let close = self.skip_balanced(open);
        let mut exports = HashSet::new();
        self.process_range(open + 1, close - 1, false, Some(&mut exports))?;

        let prefix = format!("(function({name}) {{");
        let suffix = format!("}})({name} || ({name} = {{}}));\nvar {name};");
        self.replace(self.toks[ns_kw].start, self.toks[open].end, prefix);
        self.replace(self.toks[close - 1].start, self.toks[close - 1].end, suffix);
        Ok(close)
    }

    fn record_namespace_export(
        &mut self,
        export_kw: usize,
        end: usize,
        _exports: &mut HashSet<String>,
    ) -> std::result::Result<Option<usize>, TsStripError> {
        if export_kw + 1 >= end {
            return Ok(None);
        }
        let next = export_kw + 1;
        if matches!(self.text(next), "const" | "let" | "var") && next + 1 < end {
            let id = self.text(next + 1).to_string();
            // Find parent namespace parameter from enclosing transform? Keep simple: assign `export` -> drop `export` and attach assignment?
            // Simpler & valid for single bindings `export const x = 1;`:
            let after_var = next + 2;
            let mut j = after_var;
            while j < end && !self.is_punct(j, "=") && !self.is_punct(j, ";") {
                j += 1;
            }
            if j < end && self.is_punct(j, "=") {
                // Find namespace name by walking backwards to the active namespace token?
                // Since `namespace N {` wrapped as `(function(N) {`, we don't have `N` directly here unless passed,
                // so record and replace only if namespace name matches via `self`:
                let _ = id;
            }
        }
        Ok(None)
    }

    fn apply_edits(self) -> String {
        let mut bytes = self.src.as_bytes().to_vec();
        // Sort Blank edits first, apply Replace edits in reverse source position
        let mut replaces = Vec::new();
        for edit in self.edits {
            match edit {
                Edit::Blank { start, end } => {
                    for b in &mut bytes[start..end] {
                        if *b != b'\n' && *b != b'\r' {
                            *b = b' ';
                        }
                    }
                },
                Edit::Replace { start, end, text } => {
                    replaces.push((start, end, text));
                },
            }
        }
        let mut out = String::from_utf8_lossy(&bytes).into_owned();
        replaces.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        for (start, end, text) in replaces {
            if start <= end && end <= out.len() {
                out.replace_range(start..end, &text);
            }
        }
        out
    }
}

pub fn strip_ts(src: &str, opts: &StripOptions) -> std::result::Result<String, TsStripError> {
    let mut stripper = Stripper::new(src, opts.mode);
    stripper.process()?;
    let mut out = stripper.apply_edits();

    // Handle `export const/function` inside `namespace N` cleanly on `transform` output when present
    if opts.mode == StripMode::Transform && out.contains("(function(") {
        out = rewrite_namespace_exports(&out);
    }

    if opts.source_map {
        let source_name = opts.source_url.as_deref().unwrap_or("");
        let sm_json = build_identity_source_map(source_name, &out);
        let b64 = base64_encode(sm_json.as_bytes());
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n//# sourceMappingURL=data:application/json;base64,");
        out.push_str(&b64);
    }

    if let Some(ref url) = opts.source_url {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n//# sourceURL=");
        out.push_str(url);
    }

    Ok(out)
}

fn rewrite_namespace_exports(src: &str) -> String {
    let mut out = src.to_string();
    // Lightweight pass for `(function(N) { export const/let/var K = ...; })`
    let mut pos = 0;
    while let Some(idx) = out[pos..].find("(function(") {
        let abs = pos + idx;
        if let Some(close_name) = out[abs + 10..].find(") {") {
            let ns_name = out[abs + 10..abs + 10 + close_name].trim().to_string();
            let body_start = abs + 10 + close_name + 3;
            let end_marker = format!("}})({ns_name} || ({ns_name} = {{}}));");
            if let Some(rel_end) = out[body_start..].find(&end_marker) {
                let body_end = body_start + rel_end;
                let body = out[body_start..body_end].to_string();
                let mut replaced = body;
                for kw in ["export const ", "export let ", "export var "] {
                    replaced = replaced.replace(kw, &format!("{ns_name}."));
                }
                out.replace_range(body_start..body_end, &replaced);
            }
        }
        pos = abs + 10;
    }
    out
}

fn vlq_encode_signed(val: isize, out: &mut String) {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut v = if val < 0 { ((-val) << 1) + 1 } else { val << 1 } as usize;
    loop {
        let mut digit = v & 31;
        v >>= 5;
        if v > 0 {
            digit |= 32;
        }
        out.push(B64[digit] as char);
        if v == 0 {
            break;
        }
    }
}

fn build_identity_source_map(source_name: &str, generated: &str) -> String {
    let line_count = generated.lines().count().max(1);
    let mut mappings = String::new();
    for line in 0..line_count {
        if line > 0 {
            mappings.push(';');
        }
        vlq_encode_signed(0, &mut mappings); // gen col 0
        vlq_encode_signed(0, &mut mappings); // src file idx 0
        vlq_encode_signed(if line == 0 { 0 } else { 1 }, &mut mappings); // src line delta
        vlq_encode_signed(0, &mut mappings); // src col 0
    }
    let escaped = source_name.replace('\\', "\\\\").replace('"', "\\\"");
    format!(r#"{{"version":3,"sources":["{escaped}"],"names":[],"mappings":"{mappings}"}}"#)
}

fn base64_encode(input: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = *chunk.get(1).unwrap_or(&0) as usize;
        let b2 = *chunk.get(2).unwrap_or(&0) as usize;
        out.push(TBL[b0 >> 2] as char);
        out.push(TBL[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            out.push(TBL[((b1 & 15) << 2) | (b2 >> 6)] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TBL[b2 & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn is_typescript_filename(filename: &str) -> bool {
    filename.ends_with(".ts") || filename.ends_with(".mts") || filename.ends_with(".cts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_basic_annotations_and_preserves_lines() {
        let src = "const x: number = 1;\nexport type Foo = { a: string };\nfunction f<T>(a: T): T { return a; }\n";
        let out = strip_ts(src, &StripOptions::default()).unwrap();
        assert_eq!(out.lines().count(), src.lines().count());
        assert!(out.contains("const x         = 1;"));
        assert!(!out.contains("export type"));
        assert!(out.contains("function f   (a   )    { return a; }"));
    }

    #[test]
    fn strips_as_satisfies_non_null_and_keeps_ternary_and_regex() {
        let src = "const y = (1 as number) satisfies number;\nconst z = x!.value;\nconst t = ok ? a : b;\nconst r = /foo: bar/;\n";
        let out = strip_ts(src, &StripOptions::default()).unwrap();
        assert!(!out.contains(" as "));
        assert!(!out.contains("satisfies"));
        assert!(out.contains("x .value;"));
        assert!(out.contains("ok ? a : b;"));
        assert!(out.contains("/foo: bar/"));
    }

    #[test]
    fn rejects_enum_namespace_and_param_props_in_strip_mode() {
        assert_eq!(
            strip_ts("enum E { A = 1 }", &StripOptions::default())
                .unwrap_err()
                .code,
            "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX"
        );
        assert_eq!(
            strip_ts(
                "namespace N { export const x = 1; }",
                &StripOptions::default()
            )
            .unwrap_err()
            .code,
            "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX"
        );
        assert_eq!(
            strip_ts(
                "class C { constructor(public x: number) {} }",
                &StripOptions::default()
            )
            .unwrap_err()
            .code,
            "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX"
        );
    }

    #[test]
    fn transforms_enum_namespace_param_props_and_source_map() {
        let out = strip_ts(
            "enum E { A, B = 5, C }\nnamespace N { export const x: number = 1; }\nclass C { constructor(public readonly a: number) {} }",
            &StripOptions {
                mode: StripMode::Transform,
                source_map: true,
                source_url: Some("test.ts".to_string()),
            },
        )
        .unwrap();
        assert!(out.contains("E[E[\"A\"] = 0] = \"A\";"));
        assert!(out.contains("E[E[\"C\"] = 6] = \"C\";"));
        assert!(out.contains("N.x"));
        assert!(out.contains("this.a = a;"));
        assert!(out.contains("//# sourceMappingURL=data:application/json;base64,"));
        assert!(out.contains("//# sourceURL=test.ts"));
    }
}
