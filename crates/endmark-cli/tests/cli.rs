//! What the `endmark` command prints and how it exits, for the cases the
//! fixtures do not cover: warnings, refusals inside `repair`, and the
//! commands that read.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn endmark_env(env: &[(&str, &str)], args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_endmark"))
        .args(args)
        .env_remove("ENDMARK_BY")
        .env_remove("ENDMARK_FOR")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("endmark runs");
    // A command that fails on its arguments exits without reading.
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    child.wait_with_output().unwrap()
}

fn endmark(args: &[&str], stdin: &str) -> Output {
    endmark_env(&[], args, stdin)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Runs a write, with `-` as its file, on standard input. Returns the file and what was said.
fn write(args: &[&str], file: &str) -> (String, String) {
    let mut all = vec![args[0], "--at", AT];
    all.extend(&args[1..]);
    let out = endmark(&all, file);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", text(&out.stderr));
    (text(&out.stdout), text(&out.stderr))
}

const AT: &str = "2026-11-20T10:00:00Z";
const WORKED: &str = include_str!("../../../conformance/worked-example.md");
const DOC: &str = "# Relay\n\nRelay listens on port 8080 by default.\n\nIt retries 3 times.\n";

// ---- a deletion that cannot be undone ----

#[test]
fn a_deletion_that_could_not_be_reverted_is_written_with_a_warning() {
    let warning =
        "<stdin>: warning: a deletion cannot be reverted later: give --in with text wider than what is deleted\n";
    let unscoped = ["--replace", " by default", "--with", ""];
    let whole_scope = ["--in", " by default", "--replace", " by default", "--with", ""];
    let wide_scope = ["--in", "port 8080 by default", "--replace", " by default", "--with", ""];
    for command in ["suggest", "edit"] {
        for edit in [&unscoped[..], &whole_scope[..]] {
            let mut args = vec![command, "--by", "ana"];
            args.extend(edit);
            args.push("-");
            let (file, said) = write(&args, DOC);
            assert!(file.contains("<!--endmark "), "{command}: the record is still written");
            assert!(said.ends_with(warning), "{command} {edit:?}: {said}");
        }
        let mut args = vec![command, "--by", "ana"];
        args.extend(wide_scope);
        args.push("-");
        assert!(!write(&args, DOC).1.contains("warning"), "{command} with a wide scope");
        // A replacement is always revertible.
        let said = write(
            &[command, "--by", "ana", "--replace", "8080", "--with", "8443", "-"],
            DOC,
        )
        .1;
        assert!(!said.contains("warning"), "{said}");
    }
    // Two of them are counted.
    let two = [
        "edit",
        "--by",
        "ana",
        "--replace",
        " by default",
        "--with",
        "",
        "--replace",
        " 3 times",
        "--with",
        "",
        "-",
    ];
    assert!(write(&two, DOC)
        .1
        .contains("warning: 2 deletions cannot be reverted later"));
}

#[test]
fn the_refusal_to_revert_a_deletion_says_what_was_missing() {
    let hint = "  hint: put the text back with a suggestion; to be revertible, a deletion needs an in fence wider than the text it deletes\n";
    let (file, _) = write(
        &[
            "edit",
            "--by",
            "ana",
            "--id",
            "e-1",
            "--replace",
            " by default",
            "--with",
            "",
            "-",
        ],
        DOC,
    );
    let out = endmark(&["revert", "--by", "ana", "-", "e-1"], &file);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        text(&out.stderr),
        format!("<stdin>: refused[unscoped-deletion] e-1: \"e-1\" deleted text without recording where, so it cannot be put back automatically\n{hint}")
    );

    let scoped = [
        "edit",
        "--by",
        "ana",
        "--id",
        "e-1",
        "--in",
        " by default",
        "--replace",
        " by default",
        "--with",
        "",
        "-",
    ];
    let (file, _) = write(&scoped, DOC);
    let out = endmark(&["revert", "--by", "ana", "-", "e-1"], &file);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        text(&out.stderr),
        format!("<stdin>: refused[unscoped-deletion] e-1: \"e-1\" deleted all of the text it gave as its scope, which does not record where the text was, so it cannot be put back automatically\n{hint}")
    );
}

// ---- refusals with a linter's code ----

#[test]
fn a_refusal_with_a_linter_code_carries_the_hint_of_that_code() {
    let not_written = ". Nothing was written: a reader would ignore this record\n";
    for (args, code) in [
        (
            &["accept", "--by", "adam", "-", "c-7f3k"][..],
            endmark_core::Code::WrongTarget,
        ),
        (
            &["retract", "--by", "zed", "-", "c-7f3k"],
            endmark_core::Code::NotAuthor,
        ),
        (
            &["reject", "--by", "adam", "-", "s-3c7j"],
            endmark_core::Code::AlreadyFinal,
        ),
    ] {
        let out = endmark(args, WORKED);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(out.stdout.is_empty());
        let said = text(&out.stderr);
        assert!(said.contains(&format!("refused[{}]", code.as_str())), "{said}");
        assert!(
            said.ends_with(&format!("  hint: {}{not_written}", code.hint())),
            "{said}"
        );
    }
}

// ---- repair ----

/// A comment whose quote was then changed by a tool that wrote no record.
fn detached() -> String {
    let (file, _) = write(
        &[
            "comment",
            "--by",
            "maria",
            "--id",
            "c-1",
            "--on",
            "port 8080",
            "--note",
            "Still right?",
            "-",
        ],
        DOC,
    );
    file.replace("port 8080 by", "port 9090 by")
}

#[test]
fn repair_exits_1_and_writes_nothing_when_an_anchor_is_refused() {
    let file = detached();
    let out = endmark(&["repair", "--by", "maria", "--for", "a b", "--at", AT, "-"], &file);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "a refused repair printed a file");
    let said = text(&out.stderr);
    // The same shape as the refusal of the anchor command.
    let anchor = endmark(
        &[
            "anchor",
            "--by",
            "maria",
            "--for",
            "a b",
            "--on",
            "Relay listens",
            "-",
            "c-1",
        ],
        &file,
    );
    assert_eq!(anchor.status.code(), Some(1));
    assert_eq!(said, text(&anchor.stderr));
    assert_eq!(
        said,
        "<stdin>: refused[bad-value] c-1: for \"a b\" cannot be written in a record header\n  hint: header values cannot be empty or contain spaces or \"-->\"\n"
    );
}

#[test]
fn repair_exits_0_when_it_leaves_a_comment_alone() {
    // Too little is left of the line for a place to stand out.
    let gone = detached().replace("Relay listens on port 9090 by default.", "Rewritten.");
    let out = endmark(&["repair", "--by", "maria", "-"], &gone);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(text(&out.stdout), gone);
    assert!(text(&out.stderr).contains("left c-1 alone"), "{}", text(&out.stderr));
}

// ---- who an agent writes for ----

#[test]
fn an_agent_that_names_no_person_is_warned_on_standard_error() {
    let warning = "warning: agent:claude is writing with no person named: pass --for PERSON or set ENDMARK_FOR\n";
    let comment = ["comment", "--note", "Hi.", "--id", "c-1", "--at", AT];

    let mut args = comment.to_vec();
    args.extend(["--by", "agent:claude", "-"]);
    let out = endmark(&args, DOC);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        text(&out.stderr),
        format!("<stdin>: added comment c-1\n<stdin>: {warning}")
    );
    // Standard output is the file and nothing else.
    let file = text(&out.stdout);
    assert!(file.starts_with(DOC) && file.ends_with("Hi.\n-->\n") && !file.contains("warning"));

    // ENDMARK_BY counts like --by, and a file on disk gets the warning on standard error too.
    let dir = std::env::temp_dir().join(format!("endmark-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notes.md");
    std::fs::write(&path, DOC).unwrap();
    let mut on_disk = comment.to_vec();
    on_disk.push(path.to_str().unwrap());
    let out = endmark_env(&[("ENDMARK_BY", "agent:claude")], &on_disk, "");
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        text(&out.stdout).ends_with("added comment c-1\n"),
        "{}",
        text(&out.stdout)
    );
    assert!(text(&out.stderr).ends_with(warning), "{}", text(&out.stderr));

    // A person, or an author who is not an agent: no warning.
    for extra in [
        &["--by", "agent:claude", "--for", "adam", "-"][..],
        &["--by", "maria", "-"],
    ] {
        let mut args = comment.to_vec();
        args.extend(extra);
        assert_eq!(text(&endmark(&args, DOC).stderr), "<stdin>: added comment c-1\n");
    }
    // repair writes too.
    let out = endmark(&["repair", "--by", "agent:claude", "-"], &detached());
    assert!(text(&out.stderr).ends_with(warning), "{}", text(&out.stderr));
}

#[test]
fn endmark_for_stands_in_for_the_flag() {
    let env = [("ENDMARK_BY", "agent:claude"), ("ENDMARK_FOR", "adam")];
    let comment = ["comment", "--note", "Hi.", "--id", "c-1", "--at", AT];

    let mut args = comment.to_vec();
    args.push("-");
    let out = endmark_env(&env, &args, DOC);
    assert_eq!(text(&out.stderr), "<stdin>: added comment c-1\n");
    assert!(text(&out.stdout).contains("<!--endmark comment c-1 by=agent:claude for=adam at="));

    // The flag wins.
    let mut args = comment.to_vec();
    args.extend(["--for", "maria", "-"]);
    let out = endmark_env(&env, &args, DOC);
    assert!(text(&out.stdout).contains(" by=agent:claude for=maria at="));

    // Set but empty is the same as not set.
    let mut args = comment.to_vec();
    args.push("-");
    let out = endmark_env(&[("ENDMARK_BY", "maria"), ("ENDMARK_FOR", "")], &args, DOC);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("comment c-1 by=maria at="));
}

// ---- body ----

#[test]
fn body_prints_no_blank_lines_where_the_records_were() {
    let out = endmark(&["body", "-"], WORKED);
    assert_eq!(out.status.code(), Some(0));
    let printed = text(&out.stdout);
    assert!(printed.ends_with("B[Relay]\n```\n"), "{printed:?}");
    // The body itself is unchanged: it keeps them (RFC 4.2).
    let body = endmark_core::parse(WORKED).body;
    assert!(body.ends_with("```\n\n\n\n\n\n\n\n"));
    assert_eq!(printed, format!("{}\n", body.trim_end()));

    for (file, printed) in [
        ("One.\n", "One.\n"),
        ("One.", "One."),
        ("One.\n\n\nTwo.\n\n\n", "One.\n\n\nTwo.\n"),
        // The body of a CRLF file has LF endings, here as everywhere.
        (
            "One.\r\n\r\n<!--endmark resolve c-1 by=a at=2026-11-20T10:00:00Z -->\r\n",
            "One.\n",
        ),
        ("\n<!--endmark resolve c-1 by=a at=2026-11-20T10:00:00Z -->\n", ""),
        ("", ""),
    ] {
        assert_eq!(text(&endmark(&["body", "-"], file).stdout), printed, "{file:?}");
    }
}

#[test]
fn body_takes_no_options() {
    for args in [
        &["body", "--format", "json", "-"][..],
        &["body", "--format=json", "-"],
        &["body", "--strict", "-"],
    ] {
        let out = endmark(args, WORKED);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert!(
            text(&out.stderr).starts_with("endmark: unknown option \"--"),
            "{}",
            text(&out.stderr)
        );
    }
    // The commands that have a format still take it.
    for command in ["status", "log", "lint"] {
        let out = endmark(&[command, "--format", "json", "-"], WORKED);
        assert_eq!(out.status.code(), Some(0), "{command}");
        assert!(serde_json::from_slice::<serde_json::Value>(&out.stdout).is_ok());
    }
}

// ---- help ----

#[test]
fn help_is_listed_and_prints_what_the_option_prints() {
    let help = endmark(&["--help"], "");
    let word = endmark(&["help"], "");
    assert_eq!(word.status.code(), Some(0));
    assert_eq!(text(&word.stdout), text(&help.stdout));
    assert!(text(&help.stdout).contains("\n  endmark help | --help | --version | --licenses\n"));
}

#[test]
fn licenses_prints_endmarks_license_and_the_third_party_notices() {
    let out = endmark(&["--licenses"], "");
    assert_eq!(out.status.code(), Some(0));
    let printed = text(&out.stdout);
    assert!(
        printed.starts_with("MIT License\n"),
        "Endmark's own license comes first"
    );
    assert!(printed.contains("\nThird-party software in endmark\n"));
    // One from each side: a crate linked into the binary, and a package
    // bundled into the review page.
    assert!(printed.contains("\npulldown-cmark "));
    assert!(printed.contains("\nmermaid "));
    let extra = endmark(&["--licenses", "now"], "");
    assert_eq!(extra.status.code(), Some(2));
    assert!(text(&extra.stderr).contains("--licenses takes no arguments"));
}

#[test]
fn the_usage_does_not_say_a_reply_needs_a_note() {
    let help = text(&endmark(&["--help"], "").stdout);
    assert!(help.contains("  endmark reply   [--on TEXT [--in TEXT]] [--note TEXT] FILE ID\n"));
    // It needs a note or a quote, like a comment.
    let bare = endmark(&["reply", "--by", "adam", "-", "c-7f3k"], WORKED);
    assert_eq!(bare.status.code(), Some(1));
    assert!(text(&bare.stderr).contains("refused[empty] c-7f3k: the comment would be empty"));
    let quoted = endmark(
        &["reply", "--by", "adam", "--on", "5 second delay", "-", "c-7f3k"],
        WORKED,
    );
    assert_eq!(quoted.status.code(), Some(0), "{}", text(&quoted.stderr));
}

// ---- status ----

#[test]
fn status_shows_no_place_for_a_retracted_comment() {
    let (file, _) = write(&["retract", "--by", "maria", "-", "c-7f3k"], WORKED);
    let status = text(&endmark(&["status", "-"], &file).stdout);
    let row = status.lines().find(|l| l.starts_with("c-7f3k")).unwrap();
    let cells: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(cells, ["c-7f3k", "comment", "retracted", "maria", "port", "8080"]);
    // The live comment beside it keeps its place, and the columns line up.
    let live = status.lines().find(|l| l.starts_with("c-9d1t")).unwrap();
    assert!(live.contains(" open             attached         adam "), "{live}");
    assert_eq!(row.find("maria"), live.find("adam"));
    // JSON still carries the derived state.
    let json: serde_json::Value =
        serde_json::from_slice(&endmark(&["status", "--format", "json", "-"], &file).stdout).unwrap();
    let item = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "c-7f3k")
        .unwrap();
    assert_eq!(item["anchor"], "attached", "{item}");
}

// ---- resolve and reopen from a reply ----

#[test]
fn a_no_change_refusal_says_which_id_was_given_and_which_thread_it_is_in() {
    let (resolved, _) = write(&["resolve", "--by", "adam", "-", "c-7f3k"], WORKED);
    let from_reply = endmark(&["resolve", "--by", "adam", "-", "c-2m9x"], &resolved);
    assert_eq!(from_reply.status.code(), Some(1));
    assert_eq!(
        text(&from_reply.stderr),
        "<stdin>: refused[no-change] c-2m9x: \"c-2m9x\" is in the thread of \"c-7f3k\", which is already resolved\n  hint: nothing to do\n"
    );
    let from_root = endmark(&["resolve", "--by", "adam", "-", "c-7f3k"], &resolved);
    assert_eq!(
        text(&from_root.stderr),
        "<stdin>: refused[no-change] c-7f3k: \"c-7f3k\" is already resolved\n  hint: nothing to do\n"
    );
    let open = endmark(&["reopen", "--by", "adam", "-", "c-2m9x"], WORKED);
    assert!(text(&open.stderr).contains("\"c-2m9x\" is in the thread of \"c-7f3k\", which is already open"));
}

// ---- anchor ----

#[test]
fn anchor_says_when_the_comment_was_attached_already() {
    let (_, said) = write(
        &["anchor", "--by", "adam", "--on", "5 second delay", "-", "c-7f3k"],
        WORKED,
    );
    assert_eq!(said, "<stdin>: moved attached comment c-7f3k\n");
    let (_, said) = write(
        &["anchor", "--by", "maria", "--on", "port 9090", "-", "c-1"],
        &detached(),
    );
    assert_eq!(said, "<stdin>: re-pointed c-1\n");
}
