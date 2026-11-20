//! The `endmark` command: lint and inspect Endmark files.
//!
//! This is the read path only. The commands that write records come with
//! milestone M2 of RFC 0001.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use endmark_core::{Applicability, Detail, Diagnostic, Document, Item, Landing};
use serde_json::{json, Map, Value};

const USAGE: &str = "\
endmark: comments, suggestions and history for Markdown files

Usage:
  endmark lint [--strict] [--format text|json] FILE...
  endmark status [--format text|json] FILE
  endmark body FILE
  endmark --help | --version

FILE may be - to read standard input.

Commands:
  lint     Check files. Reports records a reader would ignore, and items
           that need attention. Never writes.
  status   List every item with its state.
  body     Print the document without its records.

Options:
  --strict         lint: exit 1 on warnings as well as errors
  --format FORMAT  text (default) or json

Exit status:
  0  no errors
  1  at least one error, or a warning under --strict
  2  a file could not be read, or the arguments were wrong
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("endmark: {message}");
            ExitCode::from(2)
        }
    }
}

#[derive(PartialEq)]
enum Format {
    Text,
    Json,
}

struct Options {
    strict: bool,
    format: Format,
    files: Vec<String>,
}

fn parse_options(args: &[String], allow_strict: bool) -> Result<Options, String> {
    let mut options = Options {
        strict: false,
        format: Format::Text,
        files: Vec::new(),
    };
    let mut only_files = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let format_value = |v: &str| match v {
            "text" => Ok(Format::Text),
            "json" => Ok(Format::Json),
            other => Err(format!("unknown format \"{other}\": use text or json")),
        };
        if only_files || arg == "-" || !arg.starts_with('-') {
            options.files.push(arg.clone());
        } else if arg == "--" {
            only_files = true;
        } else if arg == "--strict" && allow_strict {
            options.strict = true;
        } else if arg == "--format" {
            let value = it.next().ok_or("--format needs a value: text or json")?;
            options.format = format_value(value)?;
        } else if let Some(value) = arg.strip_prefix("--format=") {
            options.format = format_value(value)?;
        } else {
            return Err(format!("unknown option \"{arg}\"; see endmark --help"));
        }
    }
    Ok(options)
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some(command) = args.first() else {
        eprint!("{USAGE}");
        return Ok(ExitCode::from(2));
    };
    let rest = &args[1..];
    match command.as_str() {
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        "-V" | "--version" => {
            println!("endmark {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        "lint" => lint(parse_options(rest, true)?),
        "status" => status(parse_options(rest, false)?),
        "body" => body(parse_options(rest, false)?),
        other => Err(format!("unknown command \"{other}\"; see endmark --help")),
    }
}

fn read(path: &str) -> Result<String, String> {
    let bytes = if path == "-" {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| format!("standard input: {e}"))?;
        buf
    } else {
        std::fs::read(path).map_err(|e| format!("{path}: {e}"))?
    };
    String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8", display_name(path)))
}

fn display_name(path: &str) -> &str {
    if path == "-" {
        "<stdin>"
    } else {
        path
    }
}

fn one_file(options: &Options, command: &str) -> Result<String, String> {
    match options.files.as_slice() {
        [file] => Ok(file.clone()),
        [] => Err(format!("{command} needs a file; see endmark --help")),
        _ => Err(format!("{command} takes one file")),
    }
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

// ---- lint ----

fn lint(options: Options) -> Result<ExitCode, String> {
    if options.files.is_empty() {
        return Err("lint needs at least one file; see endmark --help".into());
    }
    let mut out = io::stdout().lock();
    let (mut errors, mut warnings, mut checked) = (0, 0, 0);
    let mut unreadable = false;

    for path in &options.files {
        let source = match read(path) {
            Ok(source) => source,
            Err(message) => {
                eprintln!("endmark: {message}");
                unreadable = true;
                continue;
            }
        };
        let doc = endmark_core::parse(&source);
        let name = display_name(path);
        checked += 1;
        errors += doc.errors();
        warnings += doc.warnings();
        let written = match options.format {
            Format::Json => writeln!(
                out,
                "{}",
                json!({
                    "file": name,
                    "errors": doc.errors(),
                    "warnings": doc.warnings(),
                    "findings": doc.diagnostics.iter().map(finding_json).collect::<Vec<_>>(),
                })
            ),
            Format::Text => doc.diagnostics.iter().try_for_each(|d| {
                let id = d.id.as_deref().map(|id| format!(" {id}")).unwrap_or_default();
                writeln!(
                    out,
                    "{name}:{}: {}[{}]{id}: {}\n  hint: {}",
                    d.line,
                    d.severity().as_str(),
                    d.code.as_str(),
                    d.message,
                    d.hint()
                )
            }),
        };
        written.map_err(|e| format!("standard output: {e}"))?;
    }

    if options.format == Format::Text && checked > 0 {
        let summary = if errors + warnings == 0 {
            "no findings".to_string()
        } else {
            format!("{}, {}", plural(errors, "error"), plural(warnings, "warning"))
        };
        writeln!(out, "checked {}: {summary}", plural(checked, "file")).map_err(|e| format!("standard output: {e}"))?;
    }

    Ok(if unreadable {
        ExitCode::from(2)
    } else if errors > 0 || (options.strict && warnings > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn finding_json(d: &Diagnostic) -> Value {
    json!({
        "line": d.line,
        "severity": d.severity().as_str(),
        "code": d.code.as_str(),
        "id": d.id,
        "message": d.message,
        "hint": d.hint(),
    })
}

// ---- body ----

fn body(options: Options) -> Result<ExitCode, String> {
    let path = one_file(&options, "body")?;
    let doc = endmark_core::parse(&read(&path)?);
    io::stdout()
        .write_all(doc.body.as_bytes())
        .map_err(|e| format!("standard output: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

// ---- status ----

fn status(options: Options) -> Result<ExitCode, String> {
    let path = one_file(&options, "status")?;
    let doc = endmark_core::parse(&read(&path)?);
    let mut out = io::stdout().lock();
    let written = match options.format {
        Format::Json => writeln!(out, "{}", document_json(display_name(&path), &doc)),
        Format::Text => status_text(&mut out, &doc),
    };
    written.map_err(|e| format!("standard output: {e}"))?;
    Ok(if doc.errors() > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// The state of an item as one or two words, and what it points at.
fn summary(item: &Item) -> (String, String, String) {
    match &item.detail {
        Detail::Comment(c) => {
            let state = if c.retracted {
                "retracted".to_string()
            } else {
                match c.thread {
                    Some(thread) => thread.as_str().to_string(),
                    None => format!("reply to {}", item.re.as_deref().unwrap_or(&c.root)),
                }
            };
            let quote = c.quote.as_ref().map(|q| q.on.clone()).unwrap_or_default();
            (state, c.anchor.as_str().to_string(), quote)
        }
        Detail::Suggestion(s) => {
            let derived = match (s.applicability, s.landing) {
                (Some(Applicability::Applicable), _) => "applicable".to_string(),
                (Some(Applicability::Stale(reason)), _) => format!("stale: {}", reason.as_str()),
                (None, Some(landing)) => landing.as_str().to_string(),
                (None, None) if !s.in_effect => "reverted".to_string(),
                (None, None) => String::new(),
            };
            (s.state.as_str().to_string(), derived, plural(s.edits.len(), "edit"))
        }
        Detail::Edit(e) | Detail::Revert(e) => {
            let target = match (&item.detail, &item.re) {
                (Detail::Revert(_), Some(re)) => format!("reverts {re}, {}", plural(e.edits.len(), "edit")),
                _ => plural(e.edits.len(), "edit"),
            };
            let derived = e.landing.map_or("reverted", Landing::as_str);
            ("applied".to_string(), derived.to_string(), target)
        }
    }
}

fn status_text(out: &mut impl Write, doc: &Document) -> io::Result<()> {
    if doc.items.is_empty() {
        writeln!(out, "no items")?;
    }
    for item in &doc.items {
        let (state, derived, target) = summary(item);
        let author = match &item.on_behalf_of {
            Some(person) => format!("{} for {person}", item.by),
            None => item.by.clone(),
        };
        let target: String = target.replace('\n', "\\n").chars().take(48).collect();
        writeln!(
            out,
            "{:<10} {:<10} {:<16} {:<16} {:<24} {}",
            item.id,
            item.kind(),
            state,
            derived,
            author,
            target
        )?;
    }
    if !doc.diagnostics.is_empty() {
        writeln!(
            out,
            "{}, {}: run endmark lint for details",
            plural(doc.errors(), "error"),
            plural(doc.warnings(), "warning")
        )?;
    }
    Ok(())
}

fn document_json(file: &str, doc: &Document) -> Value {
    json!({
        "file": file,
        "body": doc.body,
        "items": doc.items.iter().map(item_json).collect::<Vec<_>>(),
        "diagnostics": doc.diagnostics.iter().map(finding_json).collect::<Vec<_>>(),
        "errors": doc.errors(),
        "warnings": doc.warnings(),
    })
}

fn edits_json(edits: &[endmark_core::Edit]) -> Value {
    edits
        .iter()
        .map(|e| json!({"in": e.scope, "replace": e.replace, "with": e.with}))
        .collect()
}

/// One item. Keys that do not apply to an item are left out, not set to null:
/// `thread` appears on thread roots only, `root` on replies only, and `status`,
/// `reason` and `reverted_by` only when there is one.
fn item_json(item: &Item) -> Value {
    let mut m = Map::new();
    let mut put = |key: &str, value: Value| {
        m.insert(key.to_string(), value);
    };
    put("id", json!(item.id));
    put("kind", json!(item.kind()));
    put("line", json!(item.line));
    put("by", json!(item.by));
    put("at", json!(item.at));
    put("for", json!(item.on_behalf_of));
    put("re", json!(item.re));
    put("note", json!(item.note));
    match &item.detail {
        Detail::Comment(c) => {
            put("state", json!(if c.retracted { "retracted" } else { "live" }));
            match c.thread {
                Some(thread) => put("thread", json!(thread.as_str())),
                None => put("root", json!(c.root)),
            }
            put("anchor", json!(c.anchor.as_str()));
            if let endmark_core::Anchor::Attached { start, end } = c.anchor {
                put("range", json!([start, end]));
            }
            put(
                "quote",
                match &c.quote {
                    Some(q) => json!({"in": q.scope, "on": q.on}),
                    None => Value::Null,
                },
            );
        }
        Detail::Suggestion(s) => {
            put("state", json!(s.state.as_str()));
            match (s.applicability, s.landing) {
                (Some(Applicability::Applicable), _) => put("status", json!("applicable")),
                (Some(Applicability::Stale(reason)), _) => {
                    put("status", json!("stale"));
                    put("reason", json!(reason.as_str()));
                }
                (None, Some(landing)) => put("status", json!(landing.as_str())),
                (None, None) if !s.in_effect => put("status", json!("reverted")),
                (None, None) => {}
            }
            if let Some(by) = &s.reverted_by {
                put("reverted_by", json!(by));
            }
            put("edits", edits_json(&s.edits));
        }
        Detail::Edit(e) | Detail::Revert(e) => {
            put("state", json!("applied"));
            put("status", json!(e.landing.map_or("reverted", Landing::as_str)));
            if let Some(by) = &e.reverted_by {
                put("reverted_by", json!(by));
            }
            put("edits", edits_json(&e.edits));
        }
    }
    put(
        "events",
        item.events
            .iter()
            .map(|e| {
                json!({
                    "verb": e.verb.as_str(),
                    "line": e.line,
                    "by": e.by,
                    "at": e.at,
                    "for": e.on_behalf_of,
                    "note": e.note,
                })
            })
            .collect(),
    );
    Value::Object(m)
}
