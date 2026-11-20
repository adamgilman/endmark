//! Runs every writer fixture in `conformance/write/` through the `endmark`
//! binary, as described in `conformance/README.md`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn endmark(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_endmark"))
        .args(args)
        .env_remove("ENDMARK_BY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run endmark");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    child.wait_with_output().expect("wait for endmark")
}

fn cases() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/write");
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("conformance/write")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.join("op.json").exists())
        .collect();
    found.sort();
    found
}

#[test]
fn writer_fixtures_pass() {
    let all = cases();
    assert!(all.len() >= 15, "found only {} writer fixtures", all.len());
    let mut failures = Vec::new();
    for dir in &all {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let op: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("op.json")).unwrap())
            .unwrap_or_else(|e| panic!("{name}/op.json: {e}"));
        let field = |key: &str| op[key].as_str();
        assert_eq!(field("op"), Some("revert"), "{name}: unknown op");
        let mut args = vec!["revert"];
        for (flag, key) in [
            ("--by", "by"),
            ("--at", "at"),
            ("--id", "new_id"),
            ("--for", "for"),
            ("--note", "note"),
        ] {
            if let Some(value) = field(key) {
                args.extend([flag, value]);
            }
        }
        args.extend(["-", field("id").expect("id")]);

        let input = std::fs::read(dir.join("input.md")).unwrap();
        let out = endmark(&args, &input);
        let stderr = String::from_utf8_lossy(&out.stderr);
        match field("fails") {
            Some(code) => {
                if out.status.code() != Some(1) || !stderr.contains(&format!("refused[{code}]")) {
                    failures.push(format!(
                        "{name}: expected refused[{code}] and exit 1, found {:?}: {stderr}",
                        out.status.code()
                    ));
                }
                if !out.stdout.is_empty() {
                    failures.push(format!("{name}: a refused write printed a file"));
                }
            }
            None => {
                let expected = std::fs::read(dir.join("expected.md")).unwrap();
                if out.status.code() != Some(0) {
                    failures.push(format!("{name}: exit {:?}: {stderr}", out.status.code()));
                } else if out.stdout != expected {
                    failures.push(format!(
                        "{name}: output differs from expected.md:\n{}",
                        String::from_utf8_lossy(&out.stdout)
                    ));
                }
                // What a writer produces must lint clean.
                let lint = endmark(&["lint", "--strict", "-"], &expected);
                if lint.status.code() != Some(0) {
                    failures.push(format!(
                        "{name}: expected.md has findings:\n{}",
                        String::from_utf8_lossy(&lint.stdout)
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

/// The file on disk is replaced in one step, and only when the write succeeds.
#[test]
fn the_file_is_replaced_only_on_success() {
    let dir = std::env::temp_dir().join(format!("endmark-write-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notes.md");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/write/worked-example");
    let input = std::fs::read(fixture.join("input.md")).unwrap();
    std::fs::write(&file, &input).unwrap();
    let path = file.to_str().unwrap();

    let refused = endmark(&["revert", "--by", "maria", path, "c-7f3k"], b"");
    assert_eq!(refused.status.code(), Some(1));
    assert_eq!(std::fs::read(&file).unwrap(), input, "a refused write changed the file");

    let no_author = endmark(&["revert", path, "s-4q8d"], b"");
    assert_eq!(no_author.status.code(), Some(2));
    assert_eq!(std::fs::read(&file).unwrap(), input);

    let dry = endmark(&["revert", "--by", "maria", "--dry-run", path, "s-4q8d"], b"");
    assert_eq!(dry.status.code(), Some(0));
    assert_eq!(std::fs::read(&file).unwrap(), input, "a dry run changed the file");

    let done = endmark(
        &[
            "revert",
            "--by=maria",
            "--id=r-6w2p",
            "--at=2026-11-12T09:15:00Z",
            "-m",
            "The v2 rollout is postponed. Staging is back on 8080.",
            path,
            "s-4q8d",
        ],
        b"",
    );
    assert_eq!(done.status.code(), Some(0), "{}", String::from_utf8_lossy(&done.stderr));
    assert_eq!(
        std::fs::read(&file).unwrap(),
        std::fs::read(fixture.join("expected.md")).unwrap()
    );
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "a temporary file was left behind"
    );

    // With no --id and no --at, the command picks them, and the result is valid.
    let again = endmark(&["revert", "--by", "adam", path, "r-6w2p"], b"");
    assert_eq!(again.status.code(), Some(0));
    let lint = endmark(&["lint", "--strict", path], b"");
    assert_eq!(lint.status.code(), Some(0), "{}", String::from_utf8_lossy(&lint.stdout));

    std::fs::remove_dir_all(&dir).unwrap();
}
