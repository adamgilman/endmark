//! `spec/endmark.md` is the normative part of the RFC, cut out so that an
//! implementer can read the format without the plan around it. This test
//! keeps the two from drifting apart.
//!
//! To regenerate after changing the RFC: `UPDATE_SPEC=1 cargo test --test spec`.

use std::path::Path;

const HEADER: &str = "\
# Endmark: the format

This is the normative part of [RFC 0001](../rfcs/0001-endmark.md): its sections 3 to 11 and its worked example, under the same section numbers. It describes the format as of version VERSION, which is a draft: the format may change before 0.1.0.

This file is generated from the RFC. To change it, change the RFC and run `UPDATE_SPEC=1 cargo test --test spec`.

Sections 12 to 16, and the decisions D-1 to D-10, are in the RFC, which also holds the rationale, the plan for the reference implementation and the prior art. Rules marked **(new)** were not in the first draft of the format.

";

fn generate(rfc: &str) -> String {
    let start = rfc.find("\n## 3. ").expect("section 3") + 1;
    let end = rfc.find("\n## 12. ").expect("section 12") + 1;
    let appendix = rfc.find("\n## Appendix A.").expect("appendix A") + 1;
    let mut spec = HEADER.replace("VERSION", env!("CARGO_PKG_VERSION"));
    spec.push_str(&rfc[start..end]);
    spec.push_str(&rfc[appendix..]);
    spec
}

#[test]
fn the_spec_matches_the_rfc() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rfc = std::fs::read_to_string(root.join("rfcs/0001-endmark.md"))
        .expect("the RFC")
        .replace("\r\n", "\n");
    let expected = generate(&rfc);
    let path = root.join("spec/endmark.md");
    if std::env::var_os("UPDATE_SPEC").is_some() {
        std::fs::write(&path, &expected).expect("write the spec");
    }
    let actual = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    assert!(
        actual == expected,
        "spec/endmark.md is out of date; run: UPDATE_SPEC=1 cargo test --test spec"
    );
}
