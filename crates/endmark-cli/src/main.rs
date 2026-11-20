//! The `endmark` command: lint and inspect Endmark files.
//!
//! Commands that read (`lint`, `status`, `body`, `log`) and commands that
//! write, one per verb of RFC 0001.

use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use endmark_core::write::{self, Stamp};
use endmark_core::{Applicability, Detail, Diagnostic, Document, Edit, Item, Landing, Quote, Verb};
use serde_json::{json, Map, Value};

const USAGE: &str = "\
endmark: comments, suggestions and history for Markdown files

Usage:
  endmark lint [--strict] [--format text|json] FILE...
  endmark status [--format text|json] FILE
  endmark log [--format text|json] FILE
  endmark body FILE

  endmark comment [--on TEXT [--in TEXT]] [--note TEXT] FILE
  endmark reply   [--on TEXT [--in TEXT]] --note TEXT FILE ID
  endmark suggest ([--in TEXT] --replace TEXT --with TEXT)... [--note TEXT] FILE
  endmark edit    ([--in TEXT] --replace TEXT --with TEXT)... [--note TEXT] FILE
  endmark accept | reject | retract | resolve | reopen [--note TEXT] FILE ID
  endmark anchor  --on TEXT [--in TEXT] FILE ID
  endmark revert  [--note TEXT] FILE ID

  endmark --help | --version

FILE may be - to read standard input. A command that writes then prints
the resulting file to standard output.

Commands that read:
  lint     Check files. Reports records a reader would ignore, and items
           that need attention. Never writes.
  status   List every item with its state.
  log      Print the history, newest first.
  body     Print the document without its records.

Commands that write. Each appends one record. Only accept, edit and
revert change the document itself.
  comment  Comment on quoted text (--on), or on the whole document.
  reply    Reply to a comment, or remark on a suggestion.
  suggest  Propose one or more replacements. The document is not changed.
  edit     Make one or more replacements now, and log them.
  accept   Apply every edit of a suggestion.
  reject   Decline a suggestion.
  retract  Withdraw your own comment or suggestion.
  resolve  Close a thread. ID may be any comment in it.
  reopen   Open a thread again.
  anchor   Point a comment at new text, after its quote went missing.
  revert   Undo an accepted suggestion, an edit, or an earlier revert.
           The text goes back and nothing is erased. To bring the change
           back, revert again. ID may be any record of the chain.

Quoted text (--on, --in, --replace) is Markdown source, exactly as in
the file, and must match exactly once. --in gives surrounding text to
search within. An empty --with deletes; an empty --replace appends to
the end of the document.

Options:
  --strict         lint: exit 1 on warnings as well as errors
  --format FORMAT  text (default) or json
  --by HANDLE      who is writing. Default: $ENDMARK_BY
  --for PERSON     the person an agent is acting for
  --note TEXT, -m  the note, in Markdown
  --dry-run        print the resulting file, do not write it
  --id ID          id for a new item. Default: random
  --at TIME        RFC 3339 UTC time. Default: now

Exit status:
  0  no errors; for a write: the file was written
  1  at least one error, or a warning under --strict; for a write:
     refused, and nothing was written
  2  a file could not be read or written, or the arguments were wrong
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        // The reader of our output went away, as in `endmark log | head`.
        Err(message) if message.contains("Broken pipe") => ExitCode::SUCCESS,
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
        "log" => log(parse_options(rest, false)?),
        "comment" | "reply" | "suggest" | "edit" | "accept" | "reject" | "retract" | "resolve" | "reopen"
        | "anchor" | "revert" => write_command(command, rest),
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

// ---- revert ----

/// The current time as RFC 3339 in UTC, to the second.
fn now_utc() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// A type letter, a hyphen and four random base-32 characters (RFC 5.1),
/// not already used in the file.
fn fresh_id(letter: char, doc: &Document) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    loop {
        // The standard library seeds each RandomState from the operating system.
        let mut bits = std::collections::hash_map::RandomState::new().build_hasher().finish();
        let mut id = format!("{letter}-");
        for _ in 0..4 {
            id.push(ALPHABET[(bits & 31) as usize] as char);
            bits >>= 5;
        }
        if doc.item(&id).is_none() {
            return id;
        }
    }
}

/// Replaces a file in one step (RFC 9.2), unless it changed since it was
/// read (RFC 9.5).
fn replace_file(path: &str, read: &str, new: &str) -> Result<(), String> {
    let fail = |e: io::Error| format!("{path}: {e}");
    // Write through a symbolic link, not over it.
    let real = std::fs::canonicalize(path).map_err(fail)?;
    let name = real
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = real.with_file_name(format!(".{name}.endmark-{}.tmp", std::process::id()));
    let meta = std::fs::metadata(&real).map_err(fail)?;
    if meta.permissions().readonly() {
        return Err(format!("{path}: the file is read-only"));
    }
    // The temporary file gets the mode of the target before it gets the
    // content, so a private file is never readable by others on the way.
    let written = std::fs::File::create(&temp)
        .and_then(|mut file| {
            file.set_permissions(meta.permissions())?;
            file.write_all(new.as_bytes())?;
            file.sync_all()
        })
        .map_err(fail)
        .and_then(|()| match std::fs::read(&real) {
            Ok(current) if current == read.as_bytes() => Ok(()),
            Ok(_) => Err(format!(
                "{path}: the file changed while this command ran; nothing was written"
            )),
            Err(e) => Err(fail(e)),
        })
        .and_then(|()| std::fs::rename(&temp, &real).map_err(fail));
    if written.is_err() {
        let _ = std::fs::remove_file(Path::new(&temp));
    }
    written
}

/// The arguments of a command that writes.
struct WriteArgs {
    by: Option<String>,
    person: Option<String>,
    note: Option<String>,
    id: Option<String>,
    at: Option<String>,
    dry_run: bool,
    /// `--in`, `--on`, `--replace` and `--with`, in the order given.
    texts: Vec<(String, String)>,
    positional: Vec<String>,
}

fn parse_write_args(args: &[String]) -> Result<WriteArgs, String> {
    let mut parsed = WriteArgs {
        by: std::env::var("ENDMARK_BY").ok(),
        person: None,
        note: None,
        id: None,
        at: None,
        dry_run: false,
        texts: Vec::new(),
        positional: Vec::new(),
    };
    let mut only_positional = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if only_positional || arg == "-" || !arg.starts_with('-') {
            parsed.positional.push(arg.clone());
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| it.next().cloned())
                .ok_or(format!("{name} needs a value"))
        };
        match name {
            "--" => only_positional = true,
            "--dry-run" => parsed.dry_run = true,
            "--by" => parsed.by = Some(value()?),
            "--for" => parsed.person = Some(value()?),
            "--note" | "-m" => parsed.note = Some(value()?),
            "--id" => parsed.id = Some(value()?),
            "--at" => parsed.at = Some(value()?),
            "--in" | "--on" | "--replace" | "--with" => parsed.texts.push((name[2..].to_string(), value()?)),
            _ => return Err(format!("unknown option \"{arg}\"; see endmark --help")),
        }
    }
    Ok(parsed)
}

/// `[--in TEXT] --on TEXT`, or nothing.
fn quote_from(texts: &[(String, String)], command: &str) -> Result<Option<Quote>, String> {
    let names: Vec<&str> = texts.iter().map(|(name, _)| name.as_str()).collect();
    match names.as_slice() {
        [] => Ok(None),
        ["on"] => Ok(Some(Quote {
            scope: None,
            on: texts[0].1.clone(),
        })),
        ["in", "on"] | ["on", "in"] => {
            let scope = texts
                .iter()
                .find(|(name, _)| name == "in")
                .map(|(_, text)| text.clone());
            let on = texts
                .iter()
                .find(|(name, _)| name == "on")
                .map(|(_, text)| text.clone());
            Ok(Some(Quote {
                scope,
                on: on.unwrap_or_default(),
            }))
        }
        _ => Err(format!("{command} takes --on TEXT, with an optional --in TEXT")),
    }
}

/// One or more `[--in TEXT] --replace TEXT --with TEXT`.
fn edits_from(texts: &[(String, String)], command: &str) -> Result<Vec<Edit>, String> {
    let usage = || format!("{command} takes one or more of: [--in TEXT] --replace TEXT --with TEXT");
    let mut edits = Vec::new();
    let mut scope: Option<String> = None;
    let mut replace: Option<String> = None;
    for (name, text) in texts {
        match (name.as_str(), &replace) {
            ("in", None) if scope.is_none() => scope = Some(text.clone()),
            ("replace", None) => replace = Some(text.clone()),
            ("with", Some(_)) => edits.push(Edit {
                scope: scope.take(),
                replace: replace.take().unwrap_or_default(),
                with: text.clone(),
            }),
            _ => return Err(usage()),
        }
    }
    if edits.is_empty() || scope.is_some() || replace.is_some() {
        return Err(usage());
    }
    Ok(edits)
}

fn write_command(command: &str, args: &[String]) -> Result<ExitCode, String> {
    let parsed = parse_write_args(args)?;
    let takes_id = !matches!(command, "comment" | "suggest" | "edit");
    let (path, target) = match (parsed.positional.as_slice(), takes_id) {
        ([path], false) => (path.as_str(), ""),
        ([path, target], true) => (path.as_str(), target.as_str()),
        (_, false) => return Err(format!("{command} needs a file; see endmark --help")),
        (_, true) => return Err(format!("{command} needs a file and an id; see endmark --help")),
    };
    let by = parsed.by.filter(|b| !b.is_empty()).ok_or(format!(
        "{command} needs to know who is writing: pass --by HANDLE or set ENDMARK_BY"
    ))?;
    let takes_texts = match command {
        "comment" | "reply" | "anchor" => "quote",
        "suggest" | "edit" => "edits",
        _ => "",
    };
    if takes_texts.is_empty() && !parsed.texts.is_empty() {
        return Err(format!("{command} takes no --{}", parsed.texts[0].0));
    }

    let source = read(path)?;
    let at = parsed.at.unwrap_or_else(now_utc);
    let stamp = Stamp {
        by: &by,
        at: &at,
        on_behalf_of: parsed.person.as_deref(),
        note: parsed.note.as_deref().unwrap_or(""),
    };
    let new_id = |letter| {
        parsed
            .id
            .clone()
            .unwrap_or_else(|| fresh_id(letter, &endmark_core::parse(&source)))
    };

    let (result, did) = match command {
        "comment" | "reply" => {
            let quote = quote_from(&parsed.texts, command)?;
            let re = (command == "reply").then_some(target);
            let done = write::comment(&source, &new_id('c'), &stamp, quote.as_ref(), re);
            let did = match re {
                Some(re) => format!("replied to {re} with comment"),
                None => "added comment".to_string(),
            };
            (done, did)
        }
        "suggest" => {
            let edits = edits_from(&parsed.texts, command)?;
            let done = write::suggest(&source, &new_id('s'), &stamp, &edits);
            (done, "added suggestion".to_string())
        }
        "edit" => {
            let edits = edits_from(&parsed.texts, command)?;
            let done = write::edit(&source, &new_id('e'), &stamp, &edits);
            (done, "applied edit".to_string())
        }
        "accept" => (write::accept(&source, target, &stamp), "accepted".to_string()),
        "anchor" => {
            let quote = quote_from(&parsed.texts, command)?.ok_or("anchor needs --on TEXT")?;
            (write::anchor(&source, target, &stamp, &quote), "re-pointed".to_string())
        }
        "revert" => (write::revert(&source, target, &new_id('r'), &stamp), String::new()),
        _ => {
            let verb = Verb::from_name(command).ok_or(format!("unknown command \"{command}\""))?;
            let did = match verb {
                Verb::Reject => "rejected",
                Verb::Retract => "retracted",
                Verb::Resolve => "resolved",
                _ => "reopened",
            };
            (write::act(&source, verb, target, &stamp), did.to_string())
        }
    };
    let done = match result {
        Ok(done) => done,
        Err(e) => {
            let subject = if target.is_empty() {
                String::new()
            } else {
                format!(" {target}")
            };
            eprintln!(
                "{}: refused[{}]{subject}: {e}\n  hint: {}",
                display_name(path),
                e.code(),
                e.hint()
            );
            return Ok(ExitCode::from(1));
        }
    };

    let to_stdout = parsed.dry_run || path == "-";
    if to_stdout {
        io::stdout()
            .write_all(done.source.as_bytes())
            .map_err(|e| format!("standard output: {e}"))?;
    } else {
        replace_file(path, &source, &done.source)?;
    }

    let mut summary = format!("{}: ", display_name(path));
    if parsed.dry_run {
        summary.push_str("dry run: ");
    }
    match &done.target {
        // A revert.
        Some(tip) => {
            summary.push_str(&format!("reverted {target}: appended {} (re={tip})", done.id));
            if tip != target {
                summary.push_str(&format!("; followed the chain from {target} to {tip}"));
            }
        }
        None => {
            summary.push_str(&format!("{did} {}", done.id));
            if takes_id && command != "reply" && done.id != target {
                summary.push_str(&format!(", the root of the thread of {target}"));
            }
        }
    }
    for (label, ids) in [
        ("re-pointed", &done.reanchored),
        ("detached", &done.detached),
        ("now stale", &done.stale),
    ] {
        if !ids.is_empty() {
            summary.push_str(&format!("; {label} {}", ids.join(", ")));
        }
    }
    // The summary goes to standard error when the file itself is on standard output.
    // The file is already written, so a closed pipe here is not a failure.
    if to_stdout {
        let _ = writeln!(io::stderr(), "{summary}");
    } else {
        let _ = writeln!(io::stdout(), "{summary}");
    }
    Ok(ExitCode::SUCCESS)
}

// ---- log ----

/// History, newest first: every record that took effect.
fn log(options: Options) -> Result<ExitCode, String> {
    let path = one_file(&options, "log")?;
    let doc = endmark_core::parse(&read(&path)?);
    let mut entries: Vec<Value> = Vec::new();
    for item in &doc.items {
        let verb = match item.detail {
            Detail::Comment(_) => "comment",
            Detail::Suggestion(_) => "suggest",
            Detail::Edit(_) => "edit",
            Detail::Revert(_) => "revert",
        };
        entries.push(json!({
            "line": item.line, "at": item.at, "by": item.by, "for": item.on_behalf_of,
            "verb": verb, "id": item.id, "re": item.re, "note": item.note,
        }));
        // A revert is an item of its own, listed above; skip its event on the target.
        for event in item.events.iter().filter(|e| e.verb != Verb::Revert) {
            entries.push(json!({
                "line": event.line, "at": event.at, "by": event.by, "for": event.on_behalf_of,
                "verb": event.verb.as_str(), "id": item.id, "re": null, "note": event.note,
            }));
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e["line"].as_u64()));

    let mut out = io::stdout().lock();
    let written = match options.format {
        Format::Json => writeln!(out, "{}", json!({"file": display_name(&path), "entries": entries})),
        Format::Text => {
            if entries.is_empty() {
                writeln!(out, "no records").map_err(|e| format!("standard output: {e}"))?;
            }
            entries.iter().try_for_each(|e| {
                let text = |key: &str| e[key].as_str().unwrap_or_default().to_string();
                let author = match e["for"].as_str() {
                    Some(person) => format!("{} for {person}", text("by")),
                    None => text("by"),
                };
                let mut about = text("note").lines().next().unwrap_or_default().to_string();
                if let Some(re) = e["re"].as_str() {
                    let link = if text("verb") == "revert" { "reverts" } else { "re" };
                    about = format!("{link} {re}{}{about}", if about.is_empty() { "" } else { ": " });
                }
                let about: String = about.chars().take(72).collect();
                writeln!(
                    out,
                    "{:<22} {:<8} {:<10} {:<24} {}",
                    text("at"),
                    text("verb"),
                    text("id"),
                    author,
                    about
                )
            })
        }
    };
    written.map_err(|e| format!("standard output: {e}"))?;
    Ok(ExitCode::SUCCESS)
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
