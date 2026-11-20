//! Runs every fixture in `conformance/` through the `endmark` binary.
//!
//! A fixture is `NAME.md` and `NAME.json`. The harness runs
//! `endmark status --format json NAME.md` and compares the result with the
//! expectation, as described in `conformance/README.md`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn fixtures() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(fixture_dir())
        .expect("conformance directory")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "md") && path.with_extension("json").exists())
        .collect();
    found.sort();
    found
}

fn expectation(md: &Path) -> Value {
    let path = md.with_extension("json");
    let text = std::fs::read_to_string(&path).expect("expectation file");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn actual(md: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_endmark"))
        .args(["status", "--format", "json"])
        .arg(md)
        .output()
        .expect("run endmark");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{}: output is not JSON: {e}", md.display()))
}

/// Every key the expectation gives must match. A null expectation means the
/// key is absent or null.
fn mismatches(what: &str, expected: &Value, actual: &Value, problems: &mut Vec<String>) {
    let (Some(expected), Some(actual)) = (expected.as_array(), actual.as_array()) else {
        problems.push(format!("{what}: not an array"));
        return;
    };
    if expected.len() != actual.len() {
        problems.push(format!(
            "{what}: expected {} but found {}: {}",
            expected.len(),
            actual.len(),
            Value::Array(actual.clone())
        ));
        return;
    }
    for (n, (e, a)) in expected.iter().zip(actual).enumerate() {
        for (key, want) in e.as_object().expect("expectation entry is an object") {
            let got = a.get(key).unwrap_or(&Value::Null);
            if got != want {
                problems.push(format!("{what}[{n}].{key}: expected {want} but found {got}"));
            }
        }
    }
}

#[test]
fn fixtures_pass() {
    let all = fixtures();
    assert!(all.len() >= 40, "found only {} fixtures", all.len());
    let mut failures = Vec::new();
    for md in &all {
        let (expected, actual) = (expectation(md), actual(md));
        let mut problems = Vec::new();
        if let Some(body) = expected.get("body") {
            if Some(body) != actual.get("body") {
                problems.push(format!("body: expected {body} but found {}", actual["body"]));
            }
        }
        mismatches("items", &expected["items"], &actual["items"], &mut problems);
        mismatches(
            "diagnostics",
            &expected["diagnostics"],
            &actual["diagnostics"],
            &mut problems,
        );
        if !problems.is_empty() {
            let name = md.file_stem().unwrap().to_string_lossy();
            failures.push(format!("{name}:\n  {}", problems.join("\n  ")));
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

/// The suite must exercise every finding code (RFC 12.6).
#[test]
fn every_code_has_a_fixture() {
    let mut seen = BTreeSet::new();
    for md in fixtures() {
        for d in expectation(&md)["diagnostics"].as_array().expect("diagnostics") {
            seen.insert(d["code"].as_str().expect("code").to_string());
        }
    }
    let missing: Vec<&str> = endmark_core::Code::ALL
        .iter()
        .map(|c| c.as_str())
        .filter(|c| !seen.contains(*c))
        .collect();
    assert!(missing.is_empty(), "no fixture raises: {missing:?}");
}
