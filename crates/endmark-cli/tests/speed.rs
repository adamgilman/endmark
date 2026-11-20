//! A budget for the linter on a large file (RFC 12.4). It only means
//! something in a release build, so it is ignored by default; CI runs it
//! with `cargo test --release --test speed -- --ignored`.

use std::fmt::Write as _;
use std::process::Command;
use std::time::{Duration, Instant};

/// About 1 MB of body with 2,000 comments, each quoting a unique phrase.
fn large_file() -> String {
    let mut body = String::new();
    let mut log = String::new();
    for n in 0..2_000 {
        writeln!(
            body,
            "Paragraph {n} talks about item-{n:05} at some length, so that the file reaches a realistic size. {}\n",
            "Filler sentence with ordinary words in it. ".repeat(9)
        )
        .unwrap();
        writeln!(
            log,
            "\n<!--endmark comment c-{n} by=ana at=2026-11-13T09:00:00Z\n```on\nitem-{n:05}\n```\nNote {n}.\n-->"
        )
        .unwrap();
    }
    body + &log
}

#[test]
#[ignore = "a timing test: run it in a release build"]
fn lint_stays_within_its_budget() {
    let budget = Duration::from_secs(3);
    let path = std::env::temp_dir().join(format!("endmark-speed-{}.md", std::process::id()));
    let source = large_file();
    assert!(source.len() > 900_000, "the file is only {} bytes", source.len());
    std::fs::write(&path, source).unwrap();

    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_endmark"))
        .arg("lint")
        .arg(&path)
        .output()
        .expect("run endmark");
    let took = started.elapsed();
    std::fs::remove_file(&path).unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    assert!(took < budget, "lint took {took:?}; the budget is {budget:?}");
    eprintln!("lint of a 1 MB file with 2,000 comments took {took:?}");
}
