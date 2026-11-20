//! The one escape rule (RFC 5.3).
//!
//! Record content must never contain `-->` or `--!>`, because either closes
//! the HTML comment early. They are written `--\>` and `--!\>`.

/// Reading: in every `--`, optional `!`, one or more backslashes, `>`,
/// remove one backslash.
pub fn unescape(s: &str) -> String {
    transform(s, false)
}

/// Writing: in every `--`, optional `!`, zero or more backslashes, `>`,
/// insert one backslash before the `>`.
pub fn escape(s: &str) -> String {
    transform(s, true)
}

fn transform(s: &str, add: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() + 4);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'-' && b.get(i + 1) == Some(&b'-') {
            let mut j = i + 2;
            if b.get(j) == Some(&b'!') {
                j += 1;
            }
            let slashes_at = j;
            while b.get(j) == Some(&b'\\') {
                j += 1;
            }
            let slashes = j - slashes_at;
            if b.get(j) == Some(&b'>') && (add || slashes >= 1) {
                out.extend_from_slice(&b[i..slashes_at]);
                let keep = if add { slashes + 1 } else { slashes - 1 };
                out.extend(std::iter::repeat(b'\\').take(keep));
                out.push(b'>');
                i = j + 1;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    // Only ASCII bytes are inserted or removed, so the result is still UTF-8.
    String::from_utf8(out).expect("escape transform keeps UTF-8 intact")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_both_terminators() {
        assert_eq!(escape("A --> B"), r"A --\> B");
        assert_eq!(escape("x --!> y"), r"x --!\> y");
    }

    #[test]
    fn unescapes_both_terminators() {
        assert_eq!(unescape(r"A --\> B"), "A --> B");
        assert_eq!(unescape(r"x --!\> y"), "x --!> y");
    }

    #[test]
    fn a_literal_escape_gains_a_backslash() {
        assert_eq!(escape(r"--\>"), r"--\\>");
        assert_eq!(unescape(r"--\\>"), r"--\>");
    }

    #[test]
    fn long_dash_runs_match_at_the_last_two_dashes() {
        assert_eq!(escape("--->"), r"---\>");
        assert_eq!(unescape(r"---\>"), "--->");
    }

    #[test]
    fn text_without_a_terminator_is_untouched() {
        for s in ["", "->", "-- >", r"-\>", "a -- b", "<!--", r"\>", "--!", "é --é> ü"] {
            assert_eq!(escape(s), s);
            assert_eq!(unescape(s), s);
        }
    }

    #[test]
    fn escaped_text_never_contains_a_terminator_and_round_trips() {
        let samples = [
            "-->",
            "--!>",
            "---->-->",
            r"--\>--\\>--!\\\>",
            "A[Webhook] --> B[Relay] --> C[Consumer]",
            "<!-- nested --> text --!> more",
            "--!-->",
            "日本語 --> é",
        ];
        for s in samples {
            let e = escape(s);
            assert!(!e.contains("-->"), "{e:?}");
            assert!(!e.contains("--!>"), "{e:?}");
            assert_eq!(unescape(&e), s);
        }
    }
}
