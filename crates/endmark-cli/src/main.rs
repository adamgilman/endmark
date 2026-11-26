//! The `endmark` command: lint and inspect Endmark files.
//!
//! Commands that read (`lint`, `status`, `body`, `log`), commands that
//! write, one per verb of RFC 0001, and `skill`, which prints instructions
//! for AI agents.

use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use endmark_core::json;
use endmark_core::repair::Candidate;
use endmark_core::write::{self, Stamp};
use endmark_core::{Anchor, Applicability, Code, Detail, Document, Edit, Item, Landing, Quote, Verb};
use serde_json::{json, Value};

mod view;

const USAGE: &str = "\
endmark: comments, suggestions and history for Markdown files

Usage:
  endmark lint [--strict] [--suggest] [--format text|json] FILE...
  endmark status [--format text|json] FILE
  endmark log [--format text|json] FILE
  endmark body FILE

  endmark comment [--on TEXT [--in TEXT]] [--note TEXT] FILE
  endmark reply   [--on TEXT [--in TEXT]] [--note TEXT] FILE ID
  endmark suggest ([--in TEXT] --replace TEXT --with TEXT)... [--note TEXT] FILE
  endmark edit    ([--in TEXT] --replace TEXT --with TEXT)... [--note TEXT] FILE
  endmark accept | reject | retract | resolve | reopen [--note TEXT] FILE ID
  endmark anchor  --on TEXT [--in TEXT] FILE ID
  endmark revert  [--note TEXT] FILE ID
  endmark repair  [--min-score N] FILE [ID...]

  endmark view [--port N] [--read-only] FILE

  endmark skill
  endmark help | --help | --version | --licenses

AI agents: run endmark skill first. It prints instructions for using
this tool, as a SKILL.md document.

FILE may be - to read standard input. A command that writes then prints
the resulting file to standard output.

Commands that read:
  lint     Check files. Reports records a reader would ignore, and items
           that need attention. Never writes.
  status   List every item with its state.
  log      Print the history, newest first.
  body     Print the document without its records, and without the
           blank lines the records leave at its end.

Commands that write. Each appends one record. Only accept, edit and
revert change the document itself.
  comment  Comment on quoted text (--on), or on the whole document.
           Needs a note, quoted text, or both.
  reply    Reply to a comment, or remark on a suggestion. Needs a note,
           quoted text, or both.
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

  repair   Re-point comments that lost their place, where one place in
           the document clearly stands out. Writes an anchor record for
           each. Run lint --suggest first to see what it would do.

In a browser:
  view     Serve FILE as a review page at http://127.0.0.1:4173/ until
           interrupted: the document with its comments and suggestions,
           and its history. What you do on the page is written to FILE
           as --by. The page follows the file when it changes on disk.
           An example for one person: it listens on this computer only.

For AI agents:
  skill    Print what Endmark is and how to use this tool, as a SKILL.md
           document. To install it as a skill, write it to a file:
           endmark skill > .claude/skills/endmark/SKILL.md

Quoted text (--on, --in, --replace) is Markdown source, exactly as in
the file, and must match exactly once. --in gives surrounding text to
search within. An empty --with deletes; an empty --replace appends to
the end of the document. A deletion can be reverted later only if it
has an --in with text wider than what it deletes.

Options:
  --strict         lint: exit 1 on warnings as well as errors
  --suggest        lint: for a comment that lost its place, say where it
                   probably belongs. Nothing is changed.
  --min-score N    repair: how sure to be, 0 to 100. Default: 80
  --format FORMAT  text (default) or json
  --by HANDLE      who is writing. Default: $ENDMARK_BY
  --for PERSON     the person an agent is acting for. Default:
                   $ENDMARK_FOR. An agent (--by agent:NAME) that names
                   no one gets a warning on standard error.
  --note TEXT, -m  the note, in Markdown
  --dry-run        print the resulting file, do not write it
  --id ID          id for a new item. Default: random
  --at TIME        RFC 3339 UTC time. Default: now
  --port N         view: the port. Default: 4173. 0 picks a free port
  --read-only      view: show the file and write nothing. Needs no --by
  --licenses       print endmark's license, and the licenses of the
                   software built into it

Exit status:
  0  no errors; for a write: the file was written; for repair: also
     when there was nothing to repair, or a comment was left alone
  1  at least one error, or a warning under --strict; for a write or
     repair: refused, and nothing was written
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
    suggest: bool,
    format: Format,
    files: Vec<String>,
}

/// `lint` says whether `--strict` and `--suggest` are taken, and `format`
/// whether `--format` is.
fn parse_options(args: &[String], allow_strict: bool, allow_format: bool) -> Result<Options, String> {
    let mut options = Options {
        strict: false,
        suggest: false,
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
        } else if arg == "--suggest" && allow_strict {
            options.suggest = true;
        } else if arg == "--format" && allow_format {
            let value = it.next().ok_or("--format needs a value: text or json")?;
            options.format = format_value(value)?;
        } else if let Some(value) = arg.strip_prefix("--format=").filter(|_| allow_format) {
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
        "--licenses" => licenses(rest),
        "lint" => lint(parse_options(rest, true, true)?),
        "status" => status(parse_options(rest, false, true)?),
        "body" => body(parse_options(rest, false, false)?),
        "log" => log(parse_options(rest, false, true)?),
        "repair" => repair(rest),
        "skill" => skill(rest),
        "view" => view::run(rest),
        "comment" | "reply" | "suggest" | "edit" | "accept" | "reject" | "retract" | "resolve" | "reopen"
        | "anchor" | "revert" => write_command(command, rest),
        other => Err(format!("unknown command \"{other}\"; see endmark --help")),
    }
}

/// Instructions for an AI agent, as a SKILL.md document. The tests in
/// `tests/skill.rs` hold it to what this file does.
const SKILL: &str = include_str!("skill.md");

fn skill(args: &[String]) -> Result<ExitCode, String> {
    if !args.is_empty() {
        return Err("skill takes no arguments; see endmark --help".into());
    }
    io::stdout()
        .write_all(SKILL.as_bytes())
        .map_err(|e| format!("standard output: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

/// Endmark's own license, then the notices of the crates and JavaScript
/// packages built into the binary, which `scripts/notices.py` writes.
const LICENSE: &str = include_str!("../LICENSE");
const THIRD_PARTY_NOTICES: &str = include_str!("../THIRD_PARTY_NOTICES.txt");

fn licenses(args: &[String]) -> Result<ExitCode, String> {
    if !args.is_empty() {
        return Err("--licenses takes no arguments; see endmark --help".into());
    }
    let text = format!("{LICENSE}\n{THIRD_PARTY_NOTICES}");
    io::stdout()
        .write_all(text.as_bytes())
        .map_err(|e| format!("standard output: {e}"))?;
    Ok(ExitCode::SUCCESS)
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
        // Repair proposals are only worked out when asked for (RFC 6.1).
        let repairs = if options.suggest {
            endmark_core::repair::candidates(&source)
        } else {
            Vec::new()
        };
        let repair_for = |d: &endmark_core::Diagnostic| {
            let unanchored = matches!(d.code, Code::Detached | Code::Ambiguous);
            repairs.iter().find(|c| unanchored && Some(&c.id) == d.id.as_ref())
        };
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
                    "findings": doc.diagnostics.iter().map(|d| {
                        let mut finding = json::finding(d);
                        if let Some(c) = repair_for(d) {
                            finding["repair"] = repair_json(c);
                        }
                        finding
                    }).collect::<Vec<_>>(),
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
                )?;
                match repair_for(d) {
                    Some(c) => writeln!(
                        out,
                        "  repair: line {}, {}% sure: {}. To accept it: endmark repair {name} {}",
                        c.line,
                        c.score,
                        c.reason.describe(),
                        c.id
                    ),
                    None => Ok(()),
                }
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
        person: std::env::var("ENDMARK_FOR").ok().filter(|p| !p.is_empty()),
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

/// The warning for an agent that names no person to act for. It goes to
/// standard error, which never carries the file.
fn warn_no_person(name: &str, by: &str, person: Option<&str>) {
    if by.starts_with("agent:") && person.is_none() {
        let _ = writeln!(
            io::stderr(),
            "{name}: warning: {by} is writing with no person named: pass --for PERSON or set ENDMARK_FOR"
        );
    }
}

/// A refused write, in the form every command prints it.
fn refusal(name: &str, id: &str, e: &write::WriteError) -> String {
    let subject = if id.is_empty() { String::new() } else { format!(" {id}") };
    format!("{name}: refused[{}]{subject}: {e}\n  hint: {}", e.code(), e.hint())
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
            // Say so when the comment had a place already and was moved from it.
            let did = match endmark_core::parse(&source).item(target).map(|i| &i.detail) {
                Some(Detail::Comment(c)) if matches!(c.anchor, Anchor::Attached { .. }) => "moved attached comment",
                _ => "re-pointed",
            };
            (write::anchor(&source, target, &stamp, &quote), did.to_string())
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
            eprintln!("{}", refusal(display_name(path), target, &e));
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
    if done.not_revertible > 0 {
        let _ = writeln!(
            io::stderr(),
            "{}: warning: {} cannot be reverted later: give --in with text wider than what is deleted",
            display_name(path),
            if done.not_revertible == 1 {
                "a deletion".to_string()
            } else {
                format!("{} deletions", done.not_revertible)
            }
        );
    }
    warn_no_person(display_name(path), &by, parsed.person.as_deref());
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

// ---- repair ----

fn repair_json(c: &Candidate) -> Value {
    json!({
        "line": c.line,
        "score": c.score,
        "reason": c.reason.as_str(),
        "on": c.quote.on,
        "in": c.quote.scope,
    })
}

/// Writes an `anchor` record for each unanchored comment that has a clear
/// candidate. The reader stays strict: this command is the recorded act
/// that moves a comment (RFC 6.1).
fn repair(args: &[String]) -> Result<ExitCode, String> {
    let mut min_score = 80u8;
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let value = match arg.split_once('=') {
            Some(("--min-score", v)) => Some(v.to_string()),
            _ if arg == "--min-score" => Some(it.next().cloned().ok_or("--min-score needs a value")?),
            _ => None,
        };
        match value {
            Some(v) => {
                min_score = v
                    .parse()
                    .ok()
                    .filter(|n| *n <= 100)
                    .ok_or("--min-score takes a number from 0 to 100")?;
            }
            None => rest.push(arg.clone()),
        }
    }
    let parsed = parse_write_args(&rest)?;
    if !parsed.texts.is_empty() || parsed.id.is_some() {
        return Err("repair takes no --on, --in, --replace, --with or --id".into());
    }
    let Some((path, ids)) = parsed.positional.split_first() else {
        return Err("repair needs a file; see endmark --help".into());
    };
    let by = parsed
        .by
        .filter(|b| !b.is_empty())
        .ok_or("repair needs to know who is writing: pass --by HANDLE or set ENDMARK_BY")?;
    let source = read(path)?;
    let at = parsed.at.unwrap_or_else(now_utc);
    let name = display_name(path);

    let found = endmark_core::repair::candidates(&source);
    let doc = endmark_core::parse(&source);
    for id in ids {
        if doc.item(id).is_none() {
            return Err(format!("{name}: no item \"{id}\""));
        }
    }
    let mut current = source.clone();
    let mut lines: Vec<String> = Vec::new();
    let mut repaired = 0;
    for c in found.iter().filter(|c| ids.is_empty() || ids.contains(&c.id)) {
        if c.score < min_score {
            lines.push(format!(
                "{name}: left {} alone: the candidate on line {} is only {}% sure (--min-score {min_score})",
                c.id, c.line, c.score
            ));
            continue;
        }
        let note = match &parsed.note {
            Some(note) => note.clone(),
            None => format!(
                "Re-pointed by endmark repair: {}, {}% sure.",
                c.reason.describe(),
                c.score
            ),
        };
        let stamp = Stamp {
            by: &by,
            at: &at,
            on_behalf_of: parsed.person.as_deref(),
            note: &note,
        };
        match write::anchor(&current, &c.id, &stamp, &c.quote) {
            Ok(done) => {
                current = done.source;
                repaired += 1;
                lines.push(format!(
                    "{name}: re-pointed {} to line {} ({}% sure)",
                    c.id, c.line, c.score
                ));
            }
            // Like any refused write: nothing is written, not even the
            // anchors that went before.
            Err(e) => {
                eprintln!("{}", refusal(name, &c.id, &e));
                return Ok(ExitCode::from(1));
            }
        }
    }
    // Say so when a comment that was asked about, or any at all, has no candidate.
    let unanchored = doc
        .diagnostics
        .iter()
        .filter(|d| matches!(d.code, Code::Detached | Code::Ambiguous));
    for d in unanchored {
        let id = d.id.clone().unwrap_or_default();
        if (ids.is_empty() || ids.contains(&id)) && !found.iter().any(|c| c.id == id) {
            lines.push(format!("{name}: left {id} alone: no place in the document stands out"));
        }
    }
    if lines.is_empty() {
        lines.push(format!("{name}: nothing to repair"));
    }

    let to_stdout = parsed.dry_run || path == "-";
    if to_stdout {
        io::stdout()
            .write_all(current.as_bytes())
            .map_err(|e| format!("standard output: {e}"))?;
    } else if repaired > 0 {
        replace_file(path, &source, &current)?;
    }
    for line in lines {
        let line = if parsed.dry_run {
            format!("dry run: {line}")
        } else {
            line
        };
        if to_stdout {
            let _ = writeln!(io::stderr(), "{line}");
        } else {
            let _ = writeln!(io::stdout(), "{line}");
        }
    }
    if repaired > 0 {
        warn_no_person(name, &by, parsed.person.as_deref());
    }
    Ok(ExitCode::SUCCESS)
}

// ---- body ----

/// The body as `endmark body` prints it. The body of RFC 4.2 keeps the blank
/// line that separated each record from the next, so a file with records
/// ends in a run of them. They are left out here, down to one line ending.
/// A body that has no blank line at its end is printed as it is.
fn printed_body(body: &str) -> &str {
    let text = body.trim_end_matches('\n');
    let ending = usize::from(!text.is_empty() && text.len() < body.len());
    &body[..text.len() + ending]
}

fn body(options: Options) -> Result<ExitCode, String> {
    let path = one_file(&options, "body")?;
    let doc = endmark_core::parse(&read(&path)?);
    io::stdout()
        .write_all(printed_body(&doc.body).as_bytes())
        .map_err(|e| format!("standard output: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

// ---- status ----

fn status(options: Options) -> Result<ExitCode, String> {
    let path = one_file(&options, "status")?;
    let doc = endmark_core::parse(&read(&path)?);
    let mut out = io::stdout().lock();
    let written = match options.format {
        Format::Json => writeln!(out, "{}", json::document(display_name(&path), &doc)),
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
            // Where a comment is matters while it is live. A retracted one
            // shows nothing there, like a closed suggestion.
            let derived = if c.retracted { "" } else { c.anchor.as_str() };
            (state, derived.to_string(), quote)
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
