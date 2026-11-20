//! Repair suggestions against the edits people really make (RFC 6.1, 12.4).
//!
//! One document, `repair/base.md`, carries ten review comments placed with
//! the real writer. Each scenario then edits the body the way someone who
//! knows nothing of Endmark would, and says what should become of every
//! comment the edit disturbs. The kinds of edit come from replaying the
//! histories of real documents (see `eval/` at the top of the repository):
//! re-wrapped paragraphs, re-aligned tables, renamed things, changed values,
//! deleted and moved sections, and near-identical sibling paragraphs.
//!
//! Every scenario also checks, for every comment:
//! - a reader alone never moves a comment: one whose quote no longer matches
//!   exactly once is unanchored, whatever its guideposts say;
//! - nothing is proposed for a comment that is still attached;
//! - every proposal can be written as an `anchor` record, after which the
//!   comment is attached and the file has no findings about it.

use endmark_core::repair::{candidates, Candidate, Reason};
use endmark_core::write::{self, Stamp};
use endmark_core::{parse, Anchor, Detail, Quote};

const BASE: &str = include_str!("repair/base.md");
const AT: &str = "2026-11-20T10:00:00Z";

/// The comments: id, quoted text, and a scope where the text is not unique.
const COMMENTS: &[(&str, &str, Option<&str>)] = &[
    ("c-port", "port 8080", None),
    (
        "c-ack",
        "Each consumer acknowledges an event\nwithin 30 seconds or the delivery counts as failed.",
        None,
    ),
    ("c-idem", "idempotent", None),
    ("c-heading", "## Retries", None),
    ("c-three", "3", Some("retried 3 times")),
    ("c-yaml", "max_attempts: 3", None),
    (
        "c-runbook",
        "See the runbook for details.",
        Some("Operators can replay the dead-letter queue by hand.\n\nSee the runbook for details."),
    ),
    ("c-sig", "`verify_signature` is on", None),
    ("c-rotate", "- Rotate the signing secret every 90 days.", None),
    (
        "c-timeout",
        "| `RELAY_TIMEOUT` | 30s | How long to wait for a consumer |",
        None,
    ),
];

/// What should become of a comment.
#[derive(Debug, Clone, Copy)]
enum Expect {
    /// Lost its place, and a proposal `endmark repair` would write by
    /// default (80 or more), found this way, whose quote contains this.
    Sure(Reason, &'static str),
    /// Lost its place, and a proposal below 80: shown by `lint --suggest`,
    /// not written by `repair` unless asked.
    Unsure(Reason, &'static str),
    /// Lost its place, and nothing is proposed.
    Nothing,
}
use Expect::{Nothing, Sure, Unsure};
use Reason::{ExactText, SimilarText, SurroundingLines};

struct Scenario {
    name: &'static str,
    /// What the person did, in a sentence.
    what: &'static str,
    edit: fn(&str) -> String,
    /// The comments the edit disturbs. Every other comment must stay attached.
    expect: &'static [(&'static str, Expect)],
}

/// Replaces text that must be in the body exactly once.
fn swap(body: &str, old: &str, new: &str) -> String {
    assert_eq!(body.matches(old).count(), 1, "the scenario expects {old:?} once");
    body.replacen(old, new, 1)
}

/// The text of one section, from its heading to the next.
fn section<'a>(body: &'a str, heading: &str) -> &'a str {
    let start = body.find(heading).expect("the heading");
    let rest = &body[start + heading.len()..];
    let end = rest
        .find("\n## ")
        .map_or(body.len(), |at| start + heading.len() + at + 1);
    &body[start..end]
}

/// Re-flows every prose paragraph to a line width, as an editor's "rewrap"
/// command or a formatter does.
fn rewrap(body: &str, width: usize) -> String {
    let mut out = Vec::new();
    let mut in_code = false;
    for block in body.split("\n\n") {
        let first = block.trim_start();
        if first.starts_with("```") {
            in_code = !block.trim_end().ends_with("```") || block.trim().len() == 3;
        }
        let prose = !in_code && !first.starts_with(['#', '|', '-', '`']);
        if !prose {
            out.push(block.to_string());
            continue;
        }
        let mut lines: Vec<String> = vec![String::new()];
        for word in block.split_whitespace() {
            let line = lines.last_mut().unwrap();
            if !line.is_empty() && line.len() + 1 + word.len() > width {
                lines.push(word.to_string());
            } else {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
        }
        out.push(lines.join("\n"));
    }
    let mut text = out.join("\n\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "paragraph-added-above",
        what: "Someone adds an introduction at the top. Every line number shifts and no quoted text changes.",
        edit: |b| swap(b, "# Relay\n", "# Relay\n\n> Status: under review. Owner: platform team.\n\nThis page is the design of record.\n"),
        expect: &[],
    },
    Scenario {
        name: "rewrapped-narrower",
        what: "The whole document is re-flowed to 60 columns. The words are the same; the line breaks are not.",
        edit: |b| rewrap(b, 60),
        expect: &[("c-ack", Sure(ExactText, "Each consumer acknowledges an event"))],
    },
    Scenario {
        name: "rewrapped-wider",
        what: "The whole document is re-flowed to 100 columns.",
        edit: |b| rewrap(b, 100),
        expect: &[("c-ack", Sure(ExactText, "Each consumer acknowledges an event"))],
    },
    Scenario {
        name: "value-changed",
        what: "The default port changes from 8080 to 9090 in the prose. The quoted words are gone; the sentence is not.",
        edit: |b| swap(b, "It listens on port 8080 by default", "It listens on port 9090 by default"),
        expect: &[("c-port", Sure(SurroundingLines, "port 9090"))],
    },
    Scenario {
        name: "sentence-reworded",
        what: "An editor tightens the sentence a comment quotes in full.",
        edit: |b| {
            swap(
                b,
                "Each consumer acknowledges an event\nwithin 30 seconds or the delivery counts as failed.",
                "Each consumer must acknowledge an event\nwithin 30 seconds or the delivery counts as failed.",
            )
        },
        expect: &[("c-ack", Sure(SimilarText, "must acknowledge an event"))],
    },
    Scenario {
        name: "sentence-rewritten",
        what: "The same sentence is replaced by one that says the same thing in other words.",
        edit: |b| {
            swap(
                b,
                "Each consumer acknowledges an event\nwithin 30 seconds or the delivery counts as failed.",
                "A consumer that stays silent for half a minute\nis treated as down.",
            )
        },
        // Nothing like the sentence is left, but its paragraph is, and
        // that is offered without confidence.
        expect: &[("c-ack", Unsure(SurroundingLines, "A consumer that stays silent"))],
    },
    Scenario {
        name: "word-changed-in-short-quote",
        what: "A one-word quote is edited: \"idempotent\" becomes \"safe to repeat\".",
        edit: |b| swap(b, "Handlers must be\nidempotent,", "Handlers must be\nsafe to repeat,"),
        expect: &[("c-idem", Sure(SurroundingLines, "safe to repeat"))],
    },
    Scenario {
        name: "heading-renamed",
        what: "A heading that a comment quotes is renamed. A heading has no lines around it to go by.",
        edit: |b| swap(b, "## Retries\n", "## Retry policy\n"),
        expect: &[("c-heading", Nothing)],
    },
    Scenario {
        name: "code-value-changed",
        what: "A value in a configuration example changes, and with it the prose that states it.",
        edit: |b| {
            let b = swap(b, "max_attempts: 3", "max_attempts: 5");
            swap(&b, "retried 3 times", "retried 5 times")
        },
        expect: &[
            ("c-yaml", Sure(SurroundingLines, "max_attempts: 5")),
            ("c-three", Sure(SurroundingLines, "retried 5 times")),
        ],
    },
    Scenario {
        name: "table-realigned",
        what: "A formatter pads the table's columns. Only spaces change.",
        edit: |b| {
            let b = swap(b, "| `RELAY_PORT` | 8080 | The port Relay listens on |", "| `RELAY_PORT`      | 8080           | The port Relay listens on       |");
            let b = swap(&b, "| `RELAY_TIMEOUT` | 30s | How long to wait for a consumer |", "| `RELAY_TIMEOUT`   | 30s            | How long to wait for a consumer |");
            swap(&b, "| `RELAY_QUEUE_DIR` | /var/lib/relay | Where the delivery queue lives |", "| `RELAY_QUEUE_DIR` | /var/lib/relay | Where the delivery queue lives  |")
        },
        expect: &[("c-timeout", Sure(ExactText, "`RELAY_TIMEOUT`"))],
    },
    Scenario {
        name: "list-item-reworded",
        what: "A list item a comment quotes in full gets a different number and an extra clause.",
        edit: |b| swap(b, "- Rotate the signing secret every 90 days.", "- Rotate the signing secret every 30 days, or at once after a leak."),
        // Nearly half the line is new, so this one is offered, not written.
        expect: &[("c-rotate", Unsure(SimilarText, "Rotate the signing secret every 30 days, or at once after a leak."))],
    },
    Scenario {
        name: "section-deleted",
        what: "The Retries section is cut. Three comments were on it. The right answer for each is silence.",
        edit: |b| b.replacen(section(b, "## Retries\n"), "", 1),
        expect: &[("c-heading", Nothing), ("c-three", Nothing), ("c-yaml", Nothing), ("c-runbook", Nothing)],
    },
    Scenario {
        name: "section-moved",
        what: "The Security section moves above Delivery. Its text is unchanged, so nothing detaches.",
        edit: |b| {
            let security = section(b, "## Security\n").to_string();
            let without = b.replacen(&security, "", 1);
            swap(&without, "## Delivery\n", &format!("{security}## Delivery\n"))
        },
        expect: &[],
    },
    Scenario {
        name: "quoted-text-repeated-elsewhere",
        what: "Someone repeats \"port 8080\" in another section, so the quote now matches twice.",
        edit: |b| swap(b, "Operators can replay the dead-letter queue by hand.", "Operators can replay the dead-letter queue by hand, through the admin\nendpoint on port 8080."),
        expect: &[
            ("c-port", Sure(ExactText, "port 8080")),
            // Its scope held the sentence that changed. The words are in
            // the body twice; what came before them says which.
            ("c-runbook", Sure(ExactText, "See the runbook for details.")),
        ],
    },
    Scenario {
        name: "repeated-line-and-shifted-lines",
        what: "A comment sits on the second of two identical lines. The line before it is reworded and a paragraph is added far above, so the stored line number now points nearer the first.",
        edit: |b| {
            let b = swap(b, "# Relay\n", "# Relay\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n\nSix.\n\nSeven.\n\nEight.\n");
            swap(&b, "Operators can replay the dead-letter queue by hand.", "Operators may replay the dead-letter queue by hand.")
        },
        expect: &[("c-runbook", Sure(ExactText, "See the runbook for details."))],
    },
    Scenario {
        name: "sibling-paragraph-survives",
        what: "Two paragraphs differ by a few words. The one a comment is on is deleted. Its sibling must not be offered with confidence.",
        edit: |b| {
            swap(
                b,
                "If `verify_signature` is on, Relay rejects any event whose signature does\nnot match the shared secret, and answers 401 without queueing it.\n\n",
                "",
            )
        },
        // A known limit: from the stored lines alone, a deleted paragraph
        // with a near twin looks like a paragraph that was edited. The
        // twin is offered, but under 80, so `repair` does not write it.
        expect: &[("c-sig", Unsure(SurroundingLines, "`verify_timestamp` is on"))],
    },
    Scenario {
        name: "paragraph-split",
        what: "A paragraph is split in two in the middle of a quoted sentence.",
        edit: |b| swap(b, "Each consumer acknowledges an event\nwithin 30 seconds", "Each consumer acknowledges an event\n\nwithin 30 seconds"),
        expect: &[("c-ack", Sure(ExactText, "Each consumer acknowledges an event"))],
    },
    Scenario {
        name: "several-edits-at-once",
        what: "A review round: an introduction is added, the document is re-flowed, the port changes and a list item is reworded.",
        edit: |b| {
            let b = swap(b, "# Relay\n", "# Relay\n\nThis page is the design of record.\n");
            let b = swap(&b, "It listens on port 8080 by default", "It listens on port 9090 by default");
            let b = swap(&b, "- Rotate the signing secret every 90 days.", "- Rotate the signing secret every 30 days.");
            rewrap(&b, 64)
        },
        expect: &[
            ("c-port", Sure(SurroundingLines, "port 9090")),
            ("c-ack", Sure(ExactText, "Each consumer acknowledges an event")),
            ("c-rotate", Sure(SimilarText, "Rotate the signing secret every 30 days")),
        ],
    },
];

fn stamp() -> Stamp<'static> {
    Stamp {
        by: "maria",
        at: AT,
        on_behalf_of: None,
        note: "A review comment.",
    }
}

/// The base document with every comment written by the real writer, split
/// into its body and its log.
fn reviewed() -> (String, String) {
    let mut source = BASE.to_string();
    for (id, on, scope) in COMMENTS {
        let quote = Quote {
            scope: scope.map(str::to_string),
            on: on.to_string(),
        };
        source = write::comment(&source, id, &stamp(), Some(&quote), None)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
            .source;
    }
    assert!(source.starts_with(BASE));
    (BASE.to_string(), source[BASE.len()..].to_string())
}

fn anchor_of(source: &str, id: &str) -> Anchor {
    match &parse(source).item(id).expect("the comment").detail {
        Detail::Comment(c) => c.anchor,
        _ => panic!("{id} is not a comment"),
    }
}

/// What happened to one comment, in the words of `Expect`, or why it is wrong.
fn check(source: &str, body: &str, id: &str, expect: Option<Expect>, found: Option<&Candidate>) -> Result<(), String> {
    let attached = matches!(anchor_of(source, id), Anchor::Attached { .. });
    let Some(expect) = expect else {
        return match (attached, found) {
            (true, None) => Ok(()),
            (true, Some(_)) => Err("still attached, and yet something was proposed".into()),
            (false, _) => Err(format!(
                "lost its place; the scenario expects it to stay attached (proposal: {found:?})"
            )),
        };
    };
    if attached {
        return Err(format!("still attached; the scenario expects {expect:?}"));
    }
    let Some(c) = found else {
        return match expect {
            Nothing => Ok(()),
            _ => Err(format!("nothing proposed; expected {expect:?}")),
        };
    };
    let (reason, text, sure) = match expect {
        Nothing => return Err(format!("expected no proposal, found {c:?}")),
        Sure(reason, text) => (reason, text, true),
        Unsure(reason, text) => (reason, text, false),
    };
    // Whitespace aside, the proposed quote holds the expected words.
    let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    if !flat(&c.quote.on).contains(&flat(text)) {
        return Err(format!("proposed {:?}, which does not contain {text:?}", c.quote.on));
    }
    if c.reason != reason {
        return Err(format!("found by {:?}, expected {reason:?}", c.reason));
    }
    if (c.score >= 80) != sure {
        return Err(format!(
            "score {}, expected {}",
            c.score,
            if sure { "80 or more" } else { "under 80" }
        ));
    }
    // The proposal is on the line it claims.
    let line = body.lines().nth(c.line - 1).unwrap_or("");
    let first = c.quote.on.lines().next().unwrap_or("");
    if !line.contains(first) {
        return Err(format!("says line {}, which is {line:?}", c.line));
    }
    // And it can be written, after which the comment is attached.
    let written =
        write::anchor(source, id, &stamp(), &c.quote).map_err(|e| format!("the proposal cannot be written: {e}"))?;
    if !matches!(anchor_of(&written.source, id), Anchor::Attached { .. }) {
        return Err("the proposal was written and the comment is still unanchored".into());
    }
    Ok(())
}

#[test]
fn the_base_document_is_clean() {
    let (body, log) = reviewed();
    let source = format!("{body}{log}");
    let doc = parse(&source);
    assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
    assert_eq!(doc.items.len(), COMMENTS.len());
    assert!(candidates(&source).is_empty());
    // Every comment carries a line hint. All but two carry a near fence:
    // the heading and the runbook line stand alone, with no lines around them.
    assert_eq!(log.matches(" line=").count(), COMMENTS.len());
    assert_eq!(log.matches("```near\n").count(), COMMENTS.len() - 2);
}

#[test]
fn every_scenario_ends_as_described() {
    let (base, log) = reviewed();
    let mut failures = Vec::new();
    for scenario in SCENARIOS {
        let body = (scenario.edit)(&base);
        assert_ne!(body, base, "{}: the edit changed nothing", scenario.name);
        let source = format!("{body}{log}");
        let found = candidates(&source);
        for (id, _, _) in COMMENTS {
            let expect = scenario.expect.iter().find(|e| e.0 == *id).map(|e| e.1);
            let candidate = found.iter().find(|c| c.id == *id);
            if let Err(why) = check(&source, &body, id, expect, candidate) {
                failures.push(format!("{} / {id}: {why}\n    ({})", scenario.name, scenario.what));
            }
        }
        for (id, _) in scenario.expect {
            assert!(
                COMMENTS.iter().any(|c| c.0 == *id),
                "{}: no comment {id}",
                scenario.name
            );
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn scenario_names_are_unique_and_every_outcome_is_covered() {
    let mut names: Vec<&str> = SCENARIOS.iter().map(|s| s.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), SCENARIOS.len());
    let all = || SCENARIOS.iter().flat_map(|s| s.expect.iter().map(|e| e.1));
    for reason in [ExactText, SimilarText, SurroundingLines] {
        assert!(
            all().any(|e| matches!(e, Sure(r, _) if r == reason)),
            "no scenario is sure by {reason:?}"
        );
    }
    assert!(all().any(|e| matches!(e, Nothing)));
    assert!(
        SCENARIOS.iter().any(|s| s.expect.is_empty()),
        "no scenario leaves every comment attached"
    );
    assert!(all().any(|e| matches!(e, Unsure(..))));
}
