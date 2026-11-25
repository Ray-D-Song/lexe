// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    collections::HashMap,
    io::{stderr, stdout, IsTerminal, Write},
    time::Instant,
};

use lexe_logging::{format_inspect, format_values_with_options, FormatOptions, NEWLINE};
use lexe_utils::{
    module::ModuleInfo,
    primordials::{BasePrimordials, Primordial},
};
use rquickjs::{
    atom::PredefinedAtom,
    class::{Trace, Tracer},
    function::{Opt, This},
    module::{Declarations, Exports, ModuleDef},
    prelude::Rest,
    qjs, Class, Coerced, Ctx, Error, Exception, Function, JsLifetime, Object, Result, Value,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

#[rquickjs::class]
pub struct Console<'js> {
    stdout: Option<Object<'js>>,
    stderr: Option<Object<'js>>,
    inspect_options: Option<Object<'js>>,
    ignore_errors: bool,
    color_mode: ColorMode,
    group_indentation: usize,
    counts: HashMap<String, u64>,
    timers: HashMap<String, Instant>,
    /// Nesting depth from `console.group`.
    indent: usize,
}

impl<'js> Trace<'js> for Console<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        if let Some(stdout) = &self.stdout {
            stdout.trace(tracer);
        }
        if let Some(stderr) = &self.stderr {
            stderr.trace(tracer);
        }
        if let Some(opts) = &self.inspect_options {
            opts.trace(tracer);
        }
    }
}

unsafe impl<'js> JsLifetime<'js> for Console<'js> {
    type Changed<'to> = Console<'to>;
}

impl<'js> Console<'js> {
    fn new_native() -> Self {
        Self {
            stdout: None,
            stderr: None,
            inspect_options: None,
            ignore_errors: true,
            color_mode: ColorMode::Auto,
            group_indentation: 2,
            counts: HashMap::new(),
            timers: HashMap::new(),
            indent: 0,
        }
    }

    fn throw_coded(ctx: &Ctx<'js>, ctor_name: &str, code: &str, message: &str) -> Error {
        if let Ok(ctor) = ctx.globals().get::<_, Function>(ctor_name) {
            if let Ok(err) = ctor.call::<_, Object>((message,)) {
                let _ = err.set("code", code);
                return ctx.throw(err.into_value());
            }
        }
        Exception::throw_type(ctx, message)
    }

    fn is_writable_stream(val: &Value<'js>) -> bool {
        val.as_object()
            .and_then(|o| o.get::<_, Value>("write").ok())
            .is_some_and(|w| w.is_function())
    }

    fn describe_js_type(val: &Value<'js>) -> String {
        if val.is_null() {
            "null".to_string()
        } else if val.is_undefined() {
            "undefined".to_string()
        } else if let Some(b) = val.as_bool() {
            format!("{b}")
        } else if let Some(s) = val.as_string() {
            format!("'{}'", s.to_string().unwrap_or_default())
        } else if let Some(n) = val.as_number() {
            format!("{n}")
        } else {
            "object".to_string()
        }
    }

    fn parse_color_mode(ctx: &Ctx<'js>, val: &Value<'js>) -> Result<ColorMode> {
        if val.is_undefined() {
            return Ok(ColorMode::Auto);
        }
        if let Some(b) = val.as_bool() {
            return Ok(if b {
                ColorMode::Always
            } else {
                ColorMode::Never
            });
        }
        if let Some(s) = val.as_string() {
            if s.to_string()? == "auto" {
                return Ok(ColorMode::Auto);
            }
        }
        let received = Self::describe_js_type(val);
        Err(Self::throw_coded(
            ctx,
            "TypeError",
            "ERR_INVALID_ARG_VALUE",
            &format!(
                "The argument 'colorMode' must be one of: 'auto', true, false. Received {received}"
            ),
        ))
    }

    fn parse_group_indentation(ctx: &Ctx<'js>, val: &Value<'js>) -> Result<usize> {
        if val.is_undefined() {
            return Ok(2);
        }
        let Some(num) = val.as_number() else {
            return Err(Self::throw_coded(
                ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "The \"groupIndentation\" argument must be of type number",
            ));
        };
        if !num.is_finite() || num.fract() != 0.0 || !(0.0..=1000.0).contains(&num) {
            return Err(Self::throw_coded(
                ctx,
                "RangeError",
                "ERR_OUT_OF_RANGE",
                &format!(
                    "The value of \"groupIndentation\" is out of range. It must be >= 0 && <= 1000. Received {num}"
                ),
            ));
        }
        Ok(num as usize)
    }

    fn label_string(_ctx: &Ctx<'_>, label: Opt<Value<'_>>) -> Result<String> {
        match label.0 {
            None => Ok("default".to_string()),
            Some(value) if value.is_undefined() => Ok("default".to_string()),
            Some(value) => Ok(value.get::<Coerced<String>>()?.0),
        }
    }

    fn target_stream(&self, use_stderr: bool) -> Option<&Object<'js>> {
        if use_stderr {
            self.stderr.as_ref()
        } else {
            self.stdout.as_ref()
        }
    }

    fn resolve_colors(&self, ctx: &Ctx<'js>, use_stderr: bool) -> bool {
        match self.color_mode {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => {
                if let Some(stream) = self.target_stream(use_stderr) {
                    if let Ok(has_colors) = stream.get::<_, Function>("hasColors") {
                        match has_colors.call::<_, bool>((This(stream.clone()),)) {
                            Ok(enabled) => return enabled,
                            Err(_) => {
                                let _ = ctx.catch();
                            },
                        }
                    }
                    stream.get::<_, bool>("isTTY").unwrap_or(false)
                } else if use_stderr {
                    stderr().is_terminal()
                } else {
                    stdout().is_terminal()
                }
            },
        }
    }

    fn build_format_options(
        &self,
        ctx: &Ctx<'js>,
        use_stderr: bool,
        default_newline: bool,
        override_opts: Option<&Object<'js>>,
    ) -> Result<FormatOptions<'js>> {
        let default_color = self.resolve_colors(ctx, use_stderr);
        match (&self.inspect_options, override_opts) {
            (Some(base), Some(over)) => {
                let merged = Object::new(ctx.clone())?;
                for key in base.keys::<String>().flatten() {
                    if let Ok(val) = base.get::<&str, Value>(key.as_str()) {
                        merged.set(key.as_str(), val)?;
                    }
                }
                for key in over.keys::<String>().flatten() {
                    if let Ok(val) = over.get::<&str, Value>(key.as_str()) {
                        merged.set(key.as_str(), val)?;
                    }
                }
                FormatOptions::from_js_options(ctx, default_color, default_newline, Some(&merged))
            },
            (Some(base), None) => {
                FormatOptions::from_js_options(ctx, default_color, default_newline, Some(base))
            },
            (None, Some(over)) => {
                FormatOptions::from_js_options(ctx, default_color, default_newline, Some(over))
            },
            (None, None) => FormatOptions::new(ctx, default_color, default_newline),
        }
    }

    fn write_raw(&self, ctx: &Ctx<'js>, use_stderr: bool, text: &str) -> Result<()> {
        if let Some(stream) = self.target_stream(use_stderr) {
            let write_fn: Function = stream.get("write").map_err(|_| {
                Self::throw_coded(
                    ctx,
                    "TypeError",
                    "ERR_CONSOLE_WRITABLE_STREAM",
                    "Console expects a writable stream instance",
                )
            })?;
            match write_fn.call::<_, Value>((This(stream.clone()), text)) {
                Ok(_) => Ok(()),
                Err(err) => {
                    if self.ignore_errors {
                        let _ = ctx.catch();
                        Ok(())
                    } else {
                        Err(err)
                    }
                },
            }
        } else if use_stderr {
            let _ = stderr().write_all(text.as_bytes());
            Ok(())
        } else {
            let _ = stdout().write_all(text.as_bytes());
            Ok(())
        }
    }

    fn write_line(&self, ctx: &Ctx<'js>, use_stderr: bool, text: &str) -> Result<()> {
        let mut out = indent_lines(text, self.indent * self.group_indentation);
        out.push(NEWLINE);
        self.write_raw(ctx, use_stderr, &out)
    }

    fn format_args(
        &self,
        ctx: &Ctx<'js>,
        use_stderr: bool,
        args: Rest<Value<'js>>,
    ) -> Result<String> {
        let mut opts = self.build_format_options(ctx, use_stderr, true, None)?;
        format_values_with_options(ctx, args, &mut opts)
    }

    fn group_impl(&mut self, ctx: &Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        if !args.0.is_empty() {
            let line = self.format_args(ctx, false, args)?;
            self.write_line(ctx, false, &line)?;
        }
        self.indent += 1;
        Ok(())
    }

    fn render_cell(&self, ctx: &Ctx<'js>, value: &Value<'js>) -> Result<String> {
        if let Some(s) = value.as_string() {
            return s.to_string();
        }
        let mut opts = self.build_format_options(ctx, false, false, None)?;
        opts.color = false;
        let out = format_values_with_options(ctx, Rest(vec![value.clone()]), &mut opts)?;
        Ok(out.trim_end_matches(NEWLINE).to_string())
    }

    fn build_table(
        &self,
        ctx: &Ctx<'js>,
        data: Option<Value<'js>>,
        columns: Option<Value<'js>>,
    ) -> Result<String> {
        enum Rows<'js> {
            Positional(Vec<Value<'js>>),
            Keyed(Vec<(String, Value<'js>)>),
        }
        let rows = match data {
            Some(value) => {
                let Some(obj) = value.as_object() else {
                    let mut opts = self.build_format_options(ctx, false, false, None)?;
                    opts.color = false;
                    let out = format_values_with_options(ctx, Rest(vec![value]), &mut opts)?;
                    return Ok(out.trim_end_matches(NEWLINE).to_string());
                };
                if let Some(arr) = obj.as_array() {
                    let mut items = Vec::new();
                    for item in arr.iter::<Value>() {
                        items.push(item?);
                    }
                    Rows::Positional(items)
                } else {
                    let mut entries = Vec::new();
                    for entry in obj.props::<String, Value>() {
                        let (key, value) = entry?;
                        entries.push((key, value));
                    }
                    Rows::Keyed(entries)
                }
            },
            None => return Ok(String::new()),
        };

        let filter: Option<Vec<String>> = match columns {
            Some(value) => match value.as_array() {
                Some(arr) => {
                    let mut out = Vec::new();
                    for item in arr.iter::<Value>() {
                        out.push(item?.get::<Coerced<String>>()?.0);
                    }
                    Some(out)
                },
                None => None,
            },
            None => None,
        };

        let mut index_cells: Vec<String> = Vec::new();
        let mut row_cells: Vec<Vec<(String, String)>> = Vec::new();
        let mut column_order: Vec<String> = Vec::new();
        let mut primitive_rows = false;

        let keep = |filter: &Option<Vec<String>>, key: &str| match filter {
            Some(list) => list.iter().any(|f| f == key),
            None => true,
        };

        match &rows {
            Rows::Positional(items) => {
                for (i, item) in items.iter().enumerate() {
                    index_cells.push(i.to_string());
                    if item.is_string()
                        || item.is_number()
                        || item.is_bool()
                        || item.is_null()
                        || item.is_undefined()
                    {
                        primitive_rows = true;
                        row_cells.push(Vec::new());
                        continue;
                    }
                    let mut cells = Vec::new();
                    if let Some(obj) = item.as_object() {
                        for entry in obj.props::<String, Value>() {
                            let (key, value) = entry?;
                            if !keep(&filter, &key) {
                                continue;
                            }
                            if !column_order.iter().any(|c| c == &key) {
                                column_order.push(key.clone());
                            }
                            cells.push((key, self.render_cell(ctx, &value)?));
                        }
                    }
                    row_cells.push(cells);
                }
                if primitive_rows && column_order.is_empty() {
                    column_order.push("Values".to_string());
                    for (i, item) in items.iter().enumerate() {
                        row_cells[i] = vec![("Values".to_string(), self.render_cell(ctx, item)?)];
                    }
                }
            },
            Rows::Keyed(entries) => {
                for (key, value) in entries {
                    index_cells.push(key.clone());
                    let mut cells = Vec::new();
                    if let Some(obj) = value.as_object() {
                        for entry in obj.props::<String, Value>() {
                            let (k, v) = entry?;
                            if !keep(&filter, &k) {
                                continue;
                            }
                            if !column_order.iter().any(|c| c == &k) {
                                column_order.push(k.clone());
                            }
                            cells.push((k, self.render_cell(ctx, &v)?));
                        }
                    }
                    row_cells.push(cells);
                }
            },
        }

        Ok(render_table(&index_cells, &column_order, &row_cells))
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Console<'js> {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Self> {
        let mut it = args.0.into_iter();
        let Some(first) = it.next() else {
            return Err(Self::throw_coded(
                &ctx,
                "TypeError",
                "ERR_CONSOLE_WRITABLE_STREAM",
                "Console expects a writable stream instance for stdout",
            ));
        };

        let (stdout_val, stderr_val, ignore_val, color_val, inspect_val, indent_val) =
            if Self::is_writable_stream(&first) {
                let stderr_arg = it
                    .next()
                    .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                let ignore_arg = it
                    .next()
                    .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                let undef = Value::new_undefined(ctx.clone());
                (
                    first,
                    stderr_arg,
                    ignore_arg,
                    undef.clone(),
                    undef.clone(),
                    undef,
                )
            } else if let Some(opts) = first.as_object().filter(|o| !o.is_array()) {
                let stdout_arg = opts
                    .get::<_, Value>("stdout")
                    .unwrap_or_else(|_| Value::new_undefined(ctx.clone()));
                let stderr_arg = opts
                    .get::<_, Value>("stderr")
                    .unwrap_or_else(|_| Value::new_undefined(ctx.clone()));
                let ignore_arg = opts
                    .get::<_, Value>("ignoreErrors")
                    .unwrap_or_else(|_| Value::new_undefined(ctx.clone()));
                let color_arg = opts
                    .get::<_, Value>("colorMode")
                    .unwrap_or_else(|_| Value::new_undefined(ctx.clone()));
                let inspect_arg = opts
                    .get::<_, Value>("inspectOptions")
                    .unwrap_or_else(|_| Value::new_undefined(ctx.clone()));
                let indent_arg = opts
                    .get::<_, Value>("groupIndentation")
                    .unwrap_or_else(|_| Value::new_undefined(ctx.clone()));
                (
                    stdout_arg,
                    stderr_arg,
                    ignore_arg,
                    color_arg,
                    inspect_arg,
                    indent_arg,
                )
            } else {
                return Err(Self::throw_coded(
                    &ctx,
                    "TypeError",
                    "ERR_CONSOLE_WRITABLE_STREAM",
                    "Console expects a writable stream instance for stdout",
                ));
            };

        if !Self::is_writable_stream(&stdout_val) {
            return Err(Self::throw_coded(
                &ctx,
                "TypeError",
                "ERR_CONSOLE_WRITABLE_STREAM",
                "Console expects a writable stream instance for stdout",
            ));
        }
        let stdout_obj = stdout_val
            .as_object()
            .expect("validated writable stream")
            .clone();

        let stderr_obj = if stderr_val.is_undefined() {
            stdout_obj.clone()
        } else if Self::is_writable_stream(&stderr_val) {
            stderr_val
                .as_object()
                .expect("validated writable stream")
                .clone()
        } else {
            return Err(Self::throw_coded(
                &ctx,
                "TypeError",
                "ERR_CONSOLE_WRITABLE_STREAM",
                "Console expects a writable stream instance for stderr",
            ));
        };

        let ignore_errors = if ignore_val.is_undefined() {
            true
        } else if let Some(b) = ignore_val.as_bool() {
            b
        } else {
            return Err(Self::throw_coded(
                &ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "The \"ignoreErrors\" argument must be of type boolean",
            ));
        };

        let color_mode = Self::parse_color_mode(&ctx, &color_val)?;
        let group_indentation = Self::parse_group_indentation(&ctx, &indent_val)?;

        let inspect_options = if inspect_val.is_undefined() {
            None
        } else if let Some(obj) = inspect_val.as_object().filter(|o| !o.is_array()) {
            Some(obj.clone())
        } else {
            return Err(Self::throw_coded(
                &ctx,
                "TypeError",
                "ERR_INVALID_ARG_TYPE",
                "The \"inspectOptions\" argument must be of type Object",
            ));
        };

        Ok(Self {
            stdout: Some(stdout_obj),
            stderr: Some(stderr_obj),
            inspect_options,
            ignore_errors,
            color_mode,
            group_indentation,
            counts: HashMap::new(),
            timers: HashMap::new(),
            indent: 0,
        })
    }

    pub fn log(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        let line = self.format_args(&ctx, false, args)?;
        self.write_line(&ctx, false, &line)
    }

    pub fn clear(&self) {
        if self.stdout.is_none() {
            clear();
        }
    }

    pub fn debug(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        let line = self.format_args(&ctx, false, args)?;
        self.write_line(&ctx, false, &line)
    }

    pub fn info(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        let line = self.format_args(&ctx, false, args)?;
        self.write_line(&ctx, false, &line)
    }

    pub fn dirxml(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        let line = self.format_args(&ctx, false, args)?;
        self.write_line(&ctx, false, &line)
    }

    pub fn trace(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        let mut header = if args.0.is_empty() {
            String::from("Trace")
        } else {
            let formatted = self.format_args(&ctx, true, args)?;
            format!("Trace: {formatted}")
        };
        if let Ok(primordials) = BasePrimordials::get(&ctx) {
            if let Ok(err_obj) = primordials.constructor_error.construct::<(), Object>(()) {
                if let Ok(stack) = err_obj.get::<_, String>(PredefinedAtom::Stack) {
                    let mut frames = stack
                        .lines()
                        .map(|l| l.trim_end_matches('\r'))
                        .filter(|l| !l.trim().is_empty() && *l != "Error")
                        .peekable();
                    // Skip the internal `globalThis.console.trace` arrow wrapper frame if present.
                    if let Some(first) = frames.peek() {
                        if first.trim_start().starts_with("at trace (eval_script:") {
                            frames.next();
                        }
                    }
                    for frame in frames {
                        header.push(NEWLINE);
                        if frame.starts_with("    at ") || frame.starts_with("  at ") {
                            header.push_str(frame);
                        } else {
                            header.push_str("    ");
                            header.push_str(frame.trim_start());
                        }
                    }
                }
            }
        }
        self.write_line(&ctx, true, &header)
    }

    pub fn error(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        let line = self.format_args(&ctx, true, args)?;
        self.write_line(&ctx, true, &line)
    }

    pub fn warn(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        let line = self.format_args(&ctx, true, args)?;
        self.write_line(&ctx, true, &line)
    }

    pub fn assert(
        &self,
        ctx: Ctx<'js>,
        expression: Opt<Value<'js>>,
        args: Rest<Value<'js>>,
    ) -> Result<()> {
        let truthy = match expression.0 {
            Some(val) => unsafe { qjs::JS_ToBool(ctx.as_raw().as_ptr(), val.as_raw()) > 0 },
            None => false,
        };
        if !truthy {
            let line = if args.0.is_empty() {
                String::from("Assertion failed")
            } else {
                let formatted = self.format_args(&ctx, true, args)?;
                format!("Assertion failed: {formatted}")
            };
            self.write_line(&ctx, true, &line)?;
        }
        Ok(())
    }

    pub fn count(&mut self, ctx: Ctx<'js>, label: Opt<Value<'js>>) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        let count = *self
            .counts
            .entry(label.clone())
            .and_modify(|c| *c += 1)
            .or_insert(1);
        self.write_line(&ctx, false, &format!("{label}: {count}"))
    }

    pub fn count_reset(&mut self, ctx: Ctx<'js>, label: Opt<Value<'js>>) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        self.counts.remove(&label);
        Ok(())
    }

    pub fn time(&mut self, ctx: Ctx<'js>, label: Opt<Value<'js>>) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        self.timers.entry(label).or_insert_with(Instant::now);
        Ok(())
    }

    pub fn time_log(
        &mut self,
        ctx: Ctx<'js>,
        label: Opt<Value<'js>>,
        args: Rest<Value<'js>>,
    ) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        if let Some(start) = self.timers.get(&label) {
            let elapsed = start.elapsed().as_millis();
            let prefix = format!("{label}: {elapsed}ms");
            if args.0.is_empty() {
                self.write_line(&ctx, false, &prefix)?;
            } else {
                let rest = self.format_args(&ctx, false, args)?;
                self.write_line(&ctx, false, &format!("{prefix} {rest}"))?;
            }
        }
        Ok(())
    }

    pub fn time_end(&mut self, ctx: Ctx<'js>, label: Opt<Value<'js>>) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        if let Some(start) = self.timers.remove(&label) {
            let elapsed = start.elapsed().as_millis();
            self.write_line(&ctx, false, &format!("{label}: {elapsed}ms"))?;
        }
        Ok(())
    }

    pub fn dir(&self, ctx: Ctx<'js>, value: Value<'js>, options: Opt<Value<'js>>) -> Result<()> {
        let override_opts = options.0.as_ref().and_then(|v| v.as_object());
        let mut opts = self.build_format_options(&ctx, false, false, override_opts)?;
        let line = format_inspect(&ctx, value, &mut opts)?;
        self.write_line(&ctx, false, &line)
    }

    pub fn group(&mut self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        self.group_impl(&ctx, args)
    }

    pub fn group_collapsed(&mut self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        // Node prints the label and indents in a non-devtools sink; there is no
        // collapsible state here, so this behaves like `group`.
        self.group_impl(&ctx, args)
    }

    pub fn group_end(&mut self) {
        // `groupEnd` without a matching `group` is a no-op in Node.
        self.indent = self.indent.saturating_sub(1);
    }

    pub fn table(
        &mut self,
        ctx: Ctx<'js>,
        data: Opt<Value<'js>>,
        columns: Opt<Value<'js>>,
    ) -> Result<()> {
        let text = self.build_table(&ctx, data.0, columns.0)?;
        self.write_line(&ctx, false, &text)
    }
}

pub fn log_fatal<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    write_console_log(stderr(), &ctx, args, false)
}

pub fn log_error<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
    write_console_log(stderr(), &ctx, args, false)
}

fn write_console_log<'js, T>(
    mut output: T,
    ctx: &Ctx<'js>,
    args: Rest<Value<'js>>,
    force_colors: bool,
) -> Result<()>
where
    T: Write + IsTerminal,
{
    let is_tty = output.is_terminal();
    let mut options = FormatOptions::new(ctx, force_colors || is_tty, true)?;
    let mut result = format_values_with_options(ctx, args, &mut options)?;
    result.push(NEWLINE);
    let _ = output.write_all(result.as_bytes());
    Ok(())
}

/// Prefix each line of `text` with `spaces` spaces.
fn indent_lines(text: &str, spaces: usize) -> String {
    if spaces == 0 {
        return text.to_string();
    }
    let prefix = " ".repeat(spaces);
    let mut out = String::with_capacity(text.len() + prefix.len() * 4);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push(NEWLINE);
        }
        out.push_str(&prefix);
        out.push_str(line);
    }
    out
}

/// Draw the box table, padding each column to its widest cell.
fn render_table(
    index_cells: &[String],
    columns: &[String],
    rows: &[Vec<(String, String)>],
) -> String {
    let mut headers: Vec<String> = vec!["(index)".to_string()];
    headers.extend(columns.iter().cloned());

    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for (i, index) in index_cells.iter().enumerate() {
        widths[0] = widths[0].max(index.chars().count());
        for (col, cell) in rows.get(i).map(|r| r.as_slice()).unwrap_or(&[]) {
            if let Some(pos) = columns.iter().position(|c| c == col) {
                widths[pos + 1] = widths[pos + 1].max(cell.chars().count());
            }
        }
    }

    let border = |left: &str, mid: &str, right: &str| {
        let mut line = String::from(left);
        for (i, w) in widths.iter().enumerate() {
            if i > 0 {
                line.push_str(mid);
            }
            line.push_str(&"─".repeat(w + 2));
        }
        line.push_str(right);
        line
    };

    let row_line = |cells: &[String]| {
        let mut line = String::from("│");
        for (i, w) in widths.iter().enumerate() {
            let text = cells.get(i).map(|s| s.as_str()).unwrap_or("");
            line.push(' ');
            line.push_str(text);
            line.push_str(&" ".repeat(w.saturating_sub(text.chars().count()) + 1));
            line.push('│');
        }
        line
    };

    let mut out = String::new();
    out.push_str(&border("┌", "┬", "┐"));
    out.push(NEWLINE);
    out.push_str(&row_line(&headers));
    out.push(NEWLINE);
    out.push_str(&border("├", "┼", "┤"));
    out.push(NEWLINE);
    for (i, index) in index_cells.iter().enumerate() {
        let mut cells = vec![index.clone()];
        for col in columns {
            let text = rows
                .get(i)
                .and_then(|r| r.iter().find(|(k, _)| k == col))
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            cells.push(text);
        }
        out.push_str(&row_line(&cells));
        out.push(NEWLINE);
    }
    out.push_str(&border("└", "┴", "┘"));
    out
}

fn clear() {
    let _ = stdout().write_all(b"\x1b[1;1H\x1b[0J");
}

const CONSOLE_METHOD_NAMES: &[&str] = &[
    "assert",
    "clear",
    "count",
    "countReset",
    "debug",
    "dir",
    "dirxml",
    "error",
    "group",
    "groupCollapsed",
    "groupEnd",
    "info",
    "log",
    "table",
    "time",
    "timeEnd",
    "timeLog",
    "trace",
    "warn",
    "profile",
    "profileEnd",
    "timeStamp",
];

pub struct ConsoleModule;

impl ModuleDef for ConsoleModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("Console")?;
        for name in CONSOLE_METHOD_NAMES {
            declare.declare(*name)?;
        }
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let globals = ctx.globals();
        if globals
            .get::<_, Object>("console")
            .ok()
            .and_then(|c| c.get::<_, Function>("Console").ok())
            .is_none()
        {
            init(ctx)?;
        }

        let global_console: Object = globals.get("console")?;
        let constructor: Value = global_console.get("Console")?;
        exports.export("Console", constructor)?;
        for name in CONSOLE_METHOD_NAMES {
            let fn_val: Value = global_console.get(*name)?;
            exports.export(*name, fn_val)?;
        }
        exports.export("default", global_console)?;
        Ok(())
    }
}

impl From<ConsoleModule> for ModuleInfo<ConsoleModule> {
    fn from(val: ConsoleModule) -> Self {
        ModuleInfo {
            name: "console",
            module: val,
        }
    }
}

pub fn init(ctx: &Ctx<'_>) -> Result<()> {
    let holder = Object::new(ctx.clone())?;
    Class::<Console>::define(&holder)?;
    let constructor: Function = holder.get("Console")?;
    let impl_instance = Class::instance(ctx.clone(), Console::new_native())?;

    let install: Function = ctx.eval(
        r#"(function (ConsoleCtor, implObj) {
  const ConsoleProto = ConsoleCtor.prototype;
  const methodNames = [
    "assert",
    "clear",
    "count",
    "countReset",
    "debug",
    "dir",
    "dirxml",
    "error",
    "group",
    "groupCollapsed",
    "groupEnd",
    "info",
    "log",
    "table",
    "time",
    "timeEnd",
    "timeLog",
    "trace",
    "warn",
  ];
  const noopNames = ["profile", "profileEnd", "timeStamp"];

  const namespaceProto = Object.create(Object.prototype);
  const globalConsole = Object.create(namespaceProto);

  Object.defineProperty(globalConsole, Symbol.toStringTag, {
    value: "console",
    writable: false,
    enumerable: false,
    configurable: true,
  });

  for (const name of methodNames) {
    const fn = (...args) => implObj[name](...args);
    Object.defineProperty(fn, "name", {
      value: name,
      writable: false,
      enumerable: false,
      configurable: true,
    });
    globalConsole[name] = fn;
  }

  for (const name of noopNames) {
    const fn = () => {};
    Object.defineProperty(fn, "name", {
      value: name,
      writable: false,
      enumerable: false,
      configurable: true,
    });
    globalConsole[name] = fn;
  }

  Object.defineProperty(globalConsole, "Console", {
    value: ConsoleCtor,
    writable: true,
    enumerable: true,
    configurable: true,
  });

  Object.defineProperty(ConsoleCtor, Symbol.hasInstance, {
    value(instance) {
      return (
        instance === globalConsole ||
        (typeof instance === "object" &&
          instance !== null &&
          ConsoleProto.isPrototypeOf(instance))
      );
    },
    writable: true,
    enumerable: false,
    configurable: true,
  });

  Object.defineProperty(globalThis, "console", {
    value: globalConsole,
    writable: true,
    enumerable: false,
    configurable: true,
  });
})"#,
    )?;
    install.call::<_, ()>((constructor, impl_instance))?;
    Ok(())
}
