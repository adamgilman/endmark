//! `endmark lint --suggest` and `endmark repair` (RFC 6.1 and 12.4).

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn endmark(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_endmark"))
        .args(args)
        .env_remove("ENDMARK_BY")
        .env_remove("ENDMARK_FOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("endmark runs");
    // A command that fails on its arguments exits without reading, which
    // closes the pipe. That is not the test's failure.
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

const AT: &str = "2026-11-20T10:00:00Z";

/// A document with three comments, then edited by a tool that knows
/// nothing of Endmark: a line added on top, the port changed, a list item
/// reworded.
fn edited() -> String {
    let body = "# Relay\n\nRelay forwards webhook events to internal consumers. It listens on port 8080 by default.\n\n- TODO\n- other\n- TODO\n";
    let mut file = body.to_string();
    for args in [
        vec!["comment", "--id", "c-1", "--on", "port 8080", "--note", "Still right?"],
        vec![
            "comment",
            "--id",
            "c-2",
            "--in",
            "- other\n- TODO",
            "--on",
            "- TODO",
            "--note",
            "This one.",
        ],
        vec!["comment", "--id", "c-3", "--on", "webhook events", "--note", "Which?"],
    ] {
        let mut all = args.clone();
        all.extend(["--by", "maria", "--at", AT, "-"]);
        let out = endmark(&all, &file);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        file = text(&out.stdout);
    }
    let (body, log) = file.split_at(file.find("<!--endmark").unwrap());
    let body = body
        .replace("# Relay\n", "# Relay\n\nA line added above.\n")
        .replace("port 8080", "port 9090")
        .replace("- other", "- another");
    format!("{body}{log}")
}

#[test]
fn the_writer_stores_guideposts() {
    let file = edited();
    assert!(file.contains("<!--endmark comment c-1 by=maria at=2026-11-20T10:00:00Z line=3\n"));
    assert!(file.contains(
        "```near\nRelay forwards webhook events to internal consumers. It listens on port 8080 by default.\n```"
    ));
    assert!(file.contains("comment c-2 by=maria at=2026-11-20T10:00:00Z line=7\n"));
}

#[test]
fn lint_reports_the_same_with_and_without_suggestions() {
    let file = edited();
    let plain = endmark(&["lint", "-"], &file);
    let suggest = endmark(&["lint", "--suggest", "-"], &file);
    assert_eq!(plain.status.code(), Some(0));
    assert_eq!(suggest.status.code(), Some(0));
    let (plain, suggest) = (text(&plain.stdout), text(&suggest.stdout));
    assert!(!plain.contains("repair:"));
    assert_eq!(plain.matches("warning[detached]").count(), 2, "{plain}");
    // The suggestions are extra lines; everything else is the same.
    let without: String = suggest
        .lines()
        .filter(|l| !l.starts_with("  repair:"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(without, plain);
    assert_eq!(suggest.matches("  repair: ").count(), 2, "{suggest}");
}

#[test]
fn suggestions_come_with_a_quote_in_json() {
    let out = endmark(&["lint", "--suggest", "--format", "json", "-"], &edited());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let findings = report["findings"].as_array().unwrap();
    let repair = |id: &str| &findings.iter().find(|f| f["id"] == id).unwrap()["repair"];
    assert_eq!(repair("c-1")["reason"], "surrounding-lines");
    assert_eq!(repair("c-1")["line"], 5);
    // c-2 was the TODO after "other". Its line hint, 7, is now the line of
    // the other TODO. The scope decides, not the line.
    assert_eq!(repair("c-2")["reason"], "exact-text");
    assert_eq!(repair("c-2")["line"], 9);
    assert_eq!(repair("c-2")["on"], "- TODO");
}

#[test]
fn repair_writes_anchors_only_when_sure() {
    let file = edited();
    let out = endmark(&["repair", "--by", "maria", "--at", AT, "-"], &file);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let (repaired, said) = (text(&out.stdout), text(&out.stderr));
    assert!(said.contains("re-pointed c-1 to line 5"), "{said}");
    assert!(said.contains("left c-2 alone"), "{said}");
    assert!(repaired.starts_with(&file), "repair only appends");
    assert_eq!(repaired.matches("<!--endmark anchor ").count(), 1);
    assert!(repaired.contains("Re-pointed by endmark repair"));

    // With a lower bar, the second one is written too, and the file is clean.
    let out = endmark(
        &["repair", "--min-score", "60", "--by", "maria", "--at", AT, "-"],
        &file,
    );
    let repaired = text(&out.stdout);
    assert_eq!(
        repaired.matches("<!--endmark anchor ").count(),
        2,
        "{}",
        text(&out.stderr)
    );
    let lint = endmark(&["lint", "--strict", "-"], &repaired);
    assert_eq!(lint.status.code(), Some(0), "{}", text(&lint.stdout));
    let status = text(&endmark(&["status", "--format", "json", "-"], &repaired).stdout);
    assert!(!status.contains("detached"));
}

#[test]
fn repair_needs_an_author_and_changes_nothing_when_nothing_is_wrong() {
    let clean = "Relay listens on port 8080.\n";
    assert_eq!(endmark(&["repair", "-"], clean).status.code(), Some(2));
    let out = endmark(&["repair", "--by", "maria", "-"], clean);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(text(&out.stdout), clean);
    assert!(text(&out.stderr).contains("nothing to repair"));
}
