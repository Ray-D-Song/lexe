// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{
    collections::HashMap,
    io::{stderr, stdout, IsTerminal, Write},
    time::Instant,
};

use raster_runtime_logging::{build_formatted_string, FormatOptions, NEWLINE};
use raster_runtime_utils::module::{export_default, ModuleInfo};
use rquickjs::{
    function::Opt,
    module::{Declarations, Exports, ModuleDef},
    prelude::Rest,
    Class, Coerced, Ctx, Function, IntoJs, Object, Result, Value,
};

#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct Console {
    #[qjs(skip_trace)]
    counts: HashMap<String, u64>,
    #[qjs(skip_trace)]
    timers: HashMap<String, Instant>,
    /// Nesting depth from `console.group`. Node indents every line by two
    /// spaces per level, including each line of a multi-line inspected value.
    #[qjs(skip_trace)]
    indent: usize,
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

impl Console {
    fn label_string(_ctx: &Ctx<'_>, label: Opt<Value<'_>>) -> Result<String> {
        match label.0 {
            None => Ok("default".to_string()),
            Some(value) if value.is_undefined() => Ok("default".to_string()),
            Some(value) => Ok(value.get::<Coerced<String>>()?.0),
        }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Console {
    #[qjs(constructor)]
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
            timers: HashMap::new(),
            indent: 0,
        }
    }

    pub fn log<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        write_console_log_indented(stdout(), &ctx, args, false, self.indent)
    }
    pub fn clear(&self) {
        clear()
    }
    pub fn debug<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        write_console_log_indented(stdout(), &ctx, args, false, self.indent)
    }
    pub fn info<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        write_console_log_indented(stdout(), &ctx, args, false, self.indent)
    }
    pub fn trace<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        write_console_log_indented(stdout(), &ctx, args, false, self.indent)
    }
    pub fn error<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        write_console_log_indented(stderr(), &ctx, args, false, self.indent)
    }
    pub fn warn<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        write_console_log_indented(stderr(), &ctx, args, false, self.indent)
    }
    pub fn assert<'js>(
        &self,
        ctx: Ctx<'js>,
        expression: bool,
        args: Rest<Value<'js>>,
    ) -> Result<()> {
        if !expression {
            write_console_log_indented(stderr(), &ctx, args, false, self.indent)?;
        }
        Ok(())
    }

    pub fn count<'js>(&mut self, ctx: Ctx<'js>, label: Opt<Value<'js>>) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        let count = self
            .counts
            .entry(label.clone())
            .and_modify(|c| *c += 1)
            .or_insert(1);
        write_console_log_indented(
            stdout(),
            &ctx,
            Rest(vec![format!("{label}: {count}").into_js(&ctx)?]),
            false,
            self.indent,
        )
    }

    pub fn count_reset<'js>(&mut self, ctx: Ctx<'js>, label: Opt<Value<'js>>) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        self.counts.remove(&label);
        Ok(())
    }

    pub fn time<'js>(&mut self, ctx: Ctx<'js>, label: Opt<Value<'js>>) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        self.timers.entry(label).or_insert_with(Instant::now);
        Ok(())
    }

    pub fn time_log<'js>(
        &mut self,
        ctx: Ctx<'js>,
        label: Opt<Value<'js>>,
        args: Rest<Value<'js>>,
    ) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        if let Some(start) = self.timers.get(&label) {
            let elapsed = start.elapsed().as_millis();
            let mut values = vec![format!("{label}: {elapsed}ms").into_js(&ctx)?];
            values.extend(args.0);
            write_console_log_indented(stdout(), &ctx, Rest(values), false, self.indent)?;
        }
        Ok(())
    }

    pub fn time_end<'js>(&mut self, ctx: Ctx<'js>, label: Opt<Value<'js>>) -> Result<()> {
        let label = Self::label_string(&ctx, label)?;
        if let Some(start) = self.timers.remove(&label) {
            let elapsed = start.elapsed().as_millis();
            write_console_log_indented(
                stdout(),
                &ctx,
                Rest(vec![format!("{label}: {elapsed}ms").into_js(&ctx)?]),
                false,
                self.indent,
            )?;
        }
        Ok(())
    }

    pub fn dir<'js>(
        &self,
        ctx: Ctx<'js>,
        value: Value<'js>,
        options: Opt<Object<'js>>,
    ) -> Result<()> {
        let mut colors = stdout().is_terminal();
        if let Some(options) = options.0 {
            if let Ok(Some(enabled)) = options.get::<_, Option<bool>>("colors") {
                colors = enabled;
            }
        }
        write_console_log_indented(stdout(), &ctx, Rest(vec![value]), colors, self.indent)
    }

    pub fn group<'js>(&mut self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        self.group_impl(&ctx, args)
    }

    pub fn group_collapsed<'js>(&mut self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        // Node prints the label and indents in a non-devtools sink; there is no
        // collapsible state here, so this behaves like `group`.
        self.group_impl(&ctx, args)
    }

    pub fn group_end(&mut self) {
        // `groupEnd` without a matching `group` is a no-op in Node.
        self.indent = self.indent.saturating_sub(1);
    }

    pub fn table<'js>(
        &mut self,
        ctx: Ctx<'js>,
        data: Opt<Value<'js>>,
        columns: Opt<Value<'js>>,
    ) -> Result<()> {
        let text = build_table(&ctx, data.0, columns.0)?;
        let mut out = stdout();
        let _ = out.write_all(indent_lines(&text, self.indent).as_bytes());
        let _ = out.write_all(&[NEWLINE as u8]);
        Ok(())
    }
}

impl Console {
    /// Shared body of `group` / `groupCollapsed`: print the label (if any) at the
    /// current depth, then indent everything that follows by one more level.
    fn group_impl<'js>(&mut self, ctx: &Ctx<'js>, args: Rest<Value<'js>>) -> Result<()> {
        if !args.0.is_empty() {
            write_console_log_indented(stdout(), ctx, args, false, self.indent)?;
        }
        self.indent += 1;
        Ok(())
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
    write_console_log_indented(output, ctx, args, force_colors, 0)
}

/// As [`write_console_log`], but prefixes every line with two spaces per
/// `indent` level. Node indents each line of a multi-line inspected value too,
/// so the indent is applied per line rather than once at the start.
fn write_console_log_indented<'js, T>(
    mut output: T,
    ctx: &Ctx<'js>,
    args: Rest<Value<'js>>,
    force_colors: bool,
    indent: usize,
) -> Result<()>
where
    T: Write + IsTerminal,
{
    let is_tty = output.is_terminal();
    let mut result = String::new();
    let mut options = FormatOptions::new(ctx, force_colors || is_tty, true)?;
    build_formatted_string(&mut result, ctx, args, &mut options)?;

    let _ = output.write_all(indent_lines(&result, indent).as_bytes());
    let _ = output.write_all(&[NEWLINE as u8]);
    Ok(())
}

/// Prefix each line of `text` with `indent` levels of two spaces.
fn indent_lines(text: &str, indent: usize) -> String {
    if indent == 0 {
        return text.to_string();
    }
    let prefix = "  ".repeat(indent);
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

/// Build the box-drawn table `console.table` prints.
///
/// Structure follows Node: an `(index)` column first, then one column per key
/// in first-seen order, and a `Values` column when the rows are primitives.
/// Output is meant for people, so widths are computed but the alignment is the
/// only thing that has to match - the column order, headers and row order are
/// what callers depend on.
fn build_table<'js>(
    ctx: &Ctx<'js>,
    data: Option<Value<'js>>,
    columns: Option<Value<'js>>,
) -> Result<String> {
    // Node only tabulates arrays and plain objects; anything else is printed as
    // it would be by `console.log`.
    // Rows are either positional (array) or keyed (plain object).
    enum Rows<'js> {
        Positional(Vec<Value<'js>>),
        Keyed(Vec<(String, Value<'js>)>),
    }
    let rows = match data {
        Some(value) => {
            let Some(obj) = value.as_object() else {
                return format_value_as_line(ctx, value);
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
    let is_array = matches!(rows, Rows::Positional(_));
    let row_count = match &rows {
        Rows::Positional(v) => v.len(),
        Rows::Keyed(v) => v.len(),
    };

    // Column filter: an array of key names to keep, in that order.
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

    // Index cells and, per row, the key/value pairs that make up the columns.
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
                        cells.push((key, render_cell(ctx, &value)?));
                    }
                }
                row_cells.push(cells);
            }
            if primitive_rows && column_order.is_empty() {
                column_order.push("Values".to_string());
                for (i, item) in items.iter().enumerate() {
                    row_cells[i] = vec![("Values".to_string(), render_cell(ctx, item)?)];
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
                        cells.push((k, render_cell(ctx, &v)?));
                    }
                }
                row_cells.push(cells);
            }
        },
    }
    let _ = row_count;

    Ok(render_table(&index_cells, &column_order, &row_cells))
}

/// A cell's text: strings unquoted, everything else inspected.
fn render_cell<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<String> {
    if let Some(s) = value.as_string() {
        return s.to_string();
    }
    format_value_as_line(ctx, value.clone())
}

/// Render a value the way `console.log` would, without the trailing newline.
fn format_value_as_line<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
    let mut out = String::new();
    // `newline: false` asks the formatter for the single-line form, which is
    // what a table cell needs: a cell holding an object must not break the row.
    let mut options = FormatOptions::new(ctx, false, false)?;
    build_formatted_string(&mut out, ctx, Rest(vec![value]), &mut options)?;
    Ok(out.trim_end_matches(NEWLINE).to_string())
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

pub struct ConsoleModule;

impl ModuleDef for ConsoleModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare(stringify!(Console))?;
        declare.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        export_default(ctx, exports, |default| {
            let globals = ctx.globals();
            let constructor: Function = globals.get(stringify!(Console))?;
            default.set(stringify!(Console), constructor)?;
            Ok(())
        })
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
    let globals = ctx.globals();

    Class::<Console>::define(&globals)?;

    ctx.eval::<(), _>(
        r#"(function () {
  const console = new Console();
  for (const name of [
    "assert",
    "clear",
    "count",
    "countReset",
    "debug",
    "dir",
    "error",
    "group",
    "groupCollapsed",
    "groupEnd",
    "info",
    "log",
    "time",
    "timeEnd",
    "table",
    "timeLog",
    "trace",
    "warn",
  ]) {
    console[name] = console[name].bind(console);
  }
  globalThis.console = console;
})();"#,
    )?;

    Ok(())
}
