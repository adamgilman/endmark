//! `endmark skill` prints instructions for AI agents. An agent follows them
//! to the letter, so these tests hold the text to what the binary does:
//! its commands, its options, its codes, and one session that is run.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use endmark_core::write::WriteError;
use endmark_core::Code;

const SKILL: &str = include_str!("../src/skill.md");

fn endmark_in(dir: Option<&Path>, env: &[(String, String)], args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_endmark"));
    command
        .args(args)
        .env_remove("ENDMARK_BY")
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().expect("run endmark");
    // Nothing on standard input: a command that waits for it would hang.
    drop(child.stdin.take());
    child.wait_with_output().expect("wait for endmark")
}

fn endmark(args: &[&str]) -> Output {
    endmark_in(None, &[], args)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn help() -> String {
    let out = endmark(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    text(&out.stdout)
}

/// Every `--option` in a text. `<!--endmark` and `-->` are not options.
fn options_in(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut before = ' ';
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let starts = rest.starts_with("--") && !(before.is_alphanumeric() || before == '-' || before == '!');
        if starts {
            let name: String = rest[2..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                .collect();
            let name = name.trim_end_matches('-');
            if name.starts_with(|c: char| c.is_ascii_lowercase()) {
                found.insert(format!("--{name}"));
            }
        }
        before = c;
        rest = &rest[c.len_utf8()..];
    }
    found
}

/// Every lower-case word that follows `endmark` and a space, which is how
/// the skill and the help text name a command.
fn commands_in(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (at, _) in text.match_indices("endmark ") {
        let before = text[..at].chars().next_back().unwrap_or(' ');
        if before.is_alphanumeric() || before == '_' {
            continue;
        }
        let word: String = text[at + "endmark ".len()..]
            .chars()
            .take_while(|c| c.is_ascii_lowercase())
            .collect();
        if !word.is_empty() {
            found.insert(word);
        }
    }
    found
}

/// The commands of the binary, from the usage lines of `--help`. A line
/// may name several: `endmark accept | reject | retract ...`.
fn real_commands() -> BTreeSet<String> {
    let help = help();
    let usage = help
        .split("Usage:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\nAI agents").next())
        .expect("the usage lines of --help");
    let mut found = BTreeSet::new();
    for line in usage.lines() {
        let Some(rest) = line.trim().strip_prefix("endmark ") else {
            continue;
        };
        for word in rest.split_whitespace() {
            if word == "|" {
                continue;
            }
            if !word.chars().all(|c| c.is_ascii_lowercase()) {
                break;
            }
            found.insert(word.to_string());
        }
    }
    found
}

/// The lines of the section under a `## ` heading.
fn section<'a>(heading: &str) -> &'a str {
    let start = SKILL
        .find(&format!("\n## {heading}\n"))
        .unwrap_or_else(|| panic!("the skill has no section \"{heading}\""));
    let rest = &SKILL[start + 1..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

#[test]
fn skill_prints_the_document() {
    let out = endmark(&["skill"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(out.stderr.is_empty());
    assert_eq!(text(&out.stdout), SKILL, "endmark skill prints skill.md exactly");

    // Frontmatter: the first line opens it, and it holds a name and a description.
    let rest = SKILL.strip_prefix("---\n").expect("the skill starts with frontmatter");
    let (front, body) = rest.split_once("\n---\n").expect("the frontmatter is closed");
    assert!(front.lines().any(|line| line == "name: endmark"));
    let description = front
        .lines()
        .find_map(|line| line.strip_prefix("description: "))
        .expect("a description on one line");
    assert!(description.len() > 80, "the description says when to use the skill");
    assert!(description.contains("<!--endmark"));
    // A colon and a space, or a space and a hash, would end a plain YAML value early.
    assert!(!description.contains(": ") && !description.contains(" #"));
    assert!(body.trim_start().starts_with("# Endmark\n"));
}

#[test]
fn skill_takes_no_arguments() {
    for args in [&["skill", "notes.md"][..], &["skill", "--format", "json"]] {
        let out = endmark(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty());
        assert!(text(&out.stderr).contains("skill takes no arguments"));
    }
}

#[test]
fn help_points_agents_at_the_skill() {
    let help = help();
    assert!(help.contains("  endmark skill\n"), "skill is in the usage lines");
    assert!(help.contains("AI agents: run endmark skill"));
}

#[test]
fn every_command_in_the_skill_is_real_and_every_real_command_is_in_the_skill() {
    let real = real_commands();
    assert!(real.len() >= 17, "found only {real:?} in --help");
    let named = commands_in(SKILL);

    for command in &named {
        // With no file every command fails, but only an unknown one says so.
        let out = endmark(&[command]);
        assert!(
            !text(&out.stderr).contains("unknown command"),
            "the skill names `endmark {command}`, which is not a command"
        );
        assert!(real.contains(command), "`endmark {command}` is not in --help");
    }
    // The reverse. A command counts as named when the skill has it as a
    // word in backticks, as in "`accept`, `reject`", or after `endmark`.
    for command in &real {
        let in_backticks = SKILL.contains(&format!("`{command}`"));
        assert!(
            named.contains(command) || in_backticks,
            "the skill does not mention `endmark {command}`"
        );
    }
    // The check itself works: a made-up command is refused by name.
    assert!(text(&endmark(&["frobnicate"]).stderr).contains("unknown command"));
}

#[test]
fn every_option_in_the_skill_is_in_the_help() {
    let known = options_in(&help());
    let named = options_in(SKILL);
    assert!(named.len() >= 12, "found only {named:?} in the skill");
    for option in &named {
        assert!(
            known.contains(option),
            "the skill names {option}, which --help does not"
        );
    }
    assert!(!named.contains("--endmark"));
}

/// One of every refusal, so that its code can be asked for. `WriteError`
/// has no list of its own; the `match` in `covered` fails to compile when
/// a variant is added, which is the reminder to add it here.
fn every_refusal() -> Vec<WriteError> {
    fn covered(e: &WriteError) {
        match e {
            WriteError::NotFound { .. }
            | WriteError::NotAChange { .. }
            | WriteError::NotInEffect { .. }
            | WriteError::UnscopedDeletion { .. }
            | WriteError::TextChanged { .. }
            | WriteError::AfterLog
            | WriteError::IdInUse { .. }
            | WriteError::BadValue { .. }
            | WriteError::Unreadable { .. }
            | WriteError::QuoteNotFound { .. }
            | WriteError::QuoteAmbiguous { .. }
            | WriteError::Overlap
            | WriteError::Stale { .. }
            | WriteError::Empty { .. }
            | WriteError::NoChange { .. }
            | WriteError::BreaksLog
            | WriteError::MixedLineEndings
            | WriteError::Ignored { .. } => {}
        }
    }
    let s = String::new;
    let all = vec![
        WriteError::NotFound { id: s() },
        WriteError::NotAChange { id: s(), what: s() },
        WriteError::NotInEffect { id: s(), tip: s() },
        WriteError::UnscopedDeletion { target: s() },
        WriteError::TextChanged {
            target: s(),
            text: s(),
            why: "",
        },
        WriteError::AfterLog,
        WriteError::IdInUse { id: s() },
        WriteError::BadValue { what: "", value: s() },
        WriteError::Unreadable { why: "" },
        WriteError::QuoteNotFound { text: s() },
        WriteError::QuoteAmbiguous { text: s() },
        WriteError::Overlap,
        WriteError::Stale { id: s(), why: "" },
        WriteError::Empty { what: "" },
        WriteError::NoChange { id: s(), what: "" },
        WriteError::BreaksLog,
        WriteError::MixedLineEndings,
    ];
    all.iter().for_each(covered);
    all
}

#[test]
fn every_code_in_the_skill_is_one_the_tool_produces() {
    // A write is refused with a code of its own, or, when a reader would
    // ignore the record, with the linter's code for that. Which linter
    // codes a write can meet is not enumerable, so any of them passes.
    let lint: BTreeSet<&str> = Code::ALL.iter().map(|c| c.as_str()).collect();
    let refusals: BTreeSet<&str> = every_refusal().iter().map(WriteError::code).collect();
    assert_eq!(refusals.len(), 17);

    // The table of refusals: the first cell of each row is a code.
    let mut refused: Vec<&str> = section("When a command refuses")
        .lines()
        .filter_map(|line| line.strip_prefix("| `")?.split('`').next())
        .collect();
    assert!(refused.len() >= 10, "found only {refused:?} in the table of refusals");
    // Codes written as refused[code], error[code] or warning[code], anywhere.
    let mut found: Vec<&str> = Vec::new();
    for (kind, list) in [("refused[", 0), ("error[", 1), ("warning[", 1)] {
        for (at, _) in SKILL.match_indices(kind) {
            let code = SKILL[at + kind.len()..].split(']').next().unwrap_or_default();
            // `refused[CODE]` describes the form and names no code.
            if code.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
                if list == 0 {
                    refused.push(code);
                } else {
                    found.push(code);
                }
            }
        }
    }
    assert!(!found.is_empty(), "the skill names no lint finding");

    for code in refused {
        assert!(
            refusals.contains(code) || lint.contains(code),
            "the skill names the refusal \"{code}\", which no write produces"
        );
    }
    for code in found {
        assert!(
            lint.contains(code),
            "the skill names the finding \"{code}\", which lint never reports"
        );
    }
}

/// A line of the session as arguments. The session keeps to what this
/// reads: words, and text in single or double quotes with no escapes.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.get_or_insert_with(String::new).push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                word.get_or_insert_with(String::new);
            }
            None if c.is_whitespace() => words.extend(word.take()),
            None => {
                assert!(
                    !matches!(c, '\\' | '$' | '|' | '>' | '<' | ';' | '&'),
                    "the session uses shell syntax this test does not read: {line}"
                );
                word.get_or_insert_with(String::new).push(c);
            }
        }
    }
    assert!(quote.is_none(), "unclosed quote: {line}");
    words.extend(word);
    words
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The skill promises that its session runs as written. Run it: the first
/// `sh` block of the section, on a copy of the worked example, which is
/// the file the section describes.
#[test]
fn the_session_in_the_skill_runs() {
    let tasks = section("Tasks, as one session");
    let block = tasks
        .split("```sh\n")
        .nth(1)
        .and_then(|rest| rest.split("\n```").next())
        .expect("a sh block in the section");

    let dir = TempDir(std::env::temp_dir().join(format!("endmark-skill-test-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/worked-example.md");
    let file = dir.0.join("notes.md");
    std::fs::copy(fixture, &file).unwrap();

    let mut env: Vec<(String, String)> = Vec::new();
    let mut ran: Vec<String> = Vec::new();
    let mut last = String::new();
    for line in block.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(setting) = line.strip_prefix("export ") {
            let (name, value) = setting.split_once('=').expect("export NAME=value");
            env.push((name.to_string(), value.to_string()));
            continue;
        }
        let words = words(line);
        assert_eq!(
            words[0], "endmark",
            "the session has a line that is not a command: {line}"
        );
        let args: Vec<&str> = words[1..].iter().map(String::as_str).collect();
        let out = endmark_in(Some(&dir.0), &env, &args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{line}\n{}{}",
            text(&out.stdout),
            text(&out.stderr)
        );
        // What the comments in the session say a command prints.
        if line.contains("listens on port 8443") {
            assert!(text(&out.stdout).contains("detached c-7f3k"), "{}", text(&out.stdout));
        }
        ran.push(words[1].clone());
        last = line.to_string();
    }

    // It shows every command that writes, and ends with the check that rule 4 asks for.
    for command in [
        "comment", "reply", "suggest", "edit", "accept", "reject", "retract", "resolve", "reopen", "anchor", "revert",
        "repair",
    ] {
        assert!(ran.iter().any(|c| c == command), "the session does not use {command}");
    }
    assert_eq!(last, "endmark lint --strict notes.md");
    let lint = endmark_in(Some(&dir.0), &[], &["lint", "--strict", "notes.md"]);
    assert_eq!(lint.status.code(), Some(0), "{}", text(&lint.stdout));
    assert_eq!(text(&lint.stdout), "checked 1 file: no findings\n");

    // The session did what it says: the revert was reverted, and the comment has a place again.
    let status = text(&endmark_in(Some(&dir.0), &[], &["status", "notes.md"]).stdout);
    let row = |id: &str| {
        status
            .lines()
            .find(|line| line.starts_with(id))
            .unwrap_or_else(|| panic!("no {id} in:\n{status}"))
            .to_string()
    };
    assert!(row("r-a001").contains("reverted"), "{status}");
    assert!(
        row("c-7f3k").contains("attached") && row("c-7f3k").contains("8443/tcp"),
        "{status}"
    );
    assert!(row("c-a001").contains("retracted"), "{status}");
    assert!(row("s-a001").contains("rejected"), "{status}");
    let body = text(&endmark_in(Some(&dir.0), &[], &["body", "notes.md"]).stdout);
    assert!(body.contains("listens on 8443/tcp") && body.contains("in the environment"));
    assert!(!body.contains("<!--endmark"));
}
