//! Anchoring and derived state (RFC 6 and 8.2): every quote is checked
//! against the current body on every read.

use std::collections::HashMap;

use crate::diag::{Code, Diagnostic};
use crate::{Anchor, Applicability, Detail, Edit, Item, Landing, StaleReason, SuggestionState, ThreadState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolved {
    At(usize, usize),
    Detached,
    Ambiguous,
}

/// Start offsets of `needle` in `hay`, counted at every start position, so
/// `aa` occurs twice in `aaa`. A match that spans a removed record is not a
/// match. `base` is the offset of `hay` in the body. Stops after two.
fn occurrences(hay: &str, needle: &str, base: usize, cuts: &[usize]) -> Vec<usize> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let start = from + rel;
        let (abs_start, abs_end) = (base + start, base + start + needle.len());
        if !cuts.iter().any(|&c| abs_start < c && c < abs_end) {
            found.push(start);
            if found.len() == 2 {
                break;
            }
        }
        // Step one character, not one match, so overlapping matches count.
        from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

fn unique(hay: &str, needle: &str, base: usize, cuts: &[usize]) -> Resolved {
    match occurrences(hay, needle, base, cuts).as_slice() {
        [] => Resolved::Detached,
        [start] => Resolved::At(base + start, base + start + needle.len()),
        _ => Resolved::Ambiguous,
    }
}

/// Resolves a non-empty quote, inside its scope when it has one.
pub(crate) fn resolve(body: &str, cuts: &[usize], quote: &str, scope: Option<&str>) -> Resolved {
    match scope {
        None => unique(body, quote, 0, cuts),
        Some(scope) => match unique(body, scope, 0, cuts) {
            Resolved::At(start, _) => unique(scope, quote, start, cuts),
            other => other,
        },
    }
}

/// Where an empty `replace` inserts: immediately after the last non-blank
/// body line and its line ending.
pub(crate) fn append_point(body: &str) -> usize {
    let mut offset = 0;
    let mut point = 0;
    for line in body.split_inclusive('\n') {
        offset += line.len();
        if !line.trim().is_empty() {
            point = offset;
        }
    }
    point
}

/// The range an edit of an open suggestion would replace.
pub(crate) fn edit_range(body: &str, cuts: &[usize], edit: &Edit) -> Resolved {
    if !edit.replace.is_empty() {
        return resolve(body, cuts, &edit.replace, edit.scope.as_deref());
    }
    // An append. A scope has nothing to narrow, but it must still resolve.
    if let Some(scope) = &edit.scope {
        match unique(body, scope, 0, cuts) {
            Resolved::At(..) => {}
            other => return other,
        }
    }
    let point = append_point(body);
    Resolved::At(point, point)
}

pub(crate) fn applicability(body: &str, cuts: &[usize], edits: &[Edit]) -> Applicability {
    let mut ranges = Vec::with_capacity(edits.len());
    for edit in edits {
        match edit_range(body, cuts, edit) {
            Resolved::At(start, end) => ranges.push((start, end)),
            Resolved::Detached => return Applicability::Stale(StaleReason::Detached),
            Resolved::Ambiguous => return Applicability::Stale(StaleReason::Ambiguous),
        }
    }
    ranges.sort_unstable();
    // Ranges that only touch do not overlap. Two insertions at one point do,
    // because their order would be undefined.
    let overlap = ranges.windows(2).any(|w| {
        let (a, b) = (w[0], w[1]);
        a.1 > b.0 || (a == b && a.0 == a.1)
    });
    if overlap {
        Applicability::Stale(StaleReason::Overlap)
    } else {
        Applicability::Applicable
    }
}

/// Where the new text of each edit of an applied change is now, if it can
/// still be found. Best effort (RFC 8.3).
fn landed_ranges(body: &str, cuts: &[usize], edits: &[Edit]) -> Vec<Option<(usize, usize)>> {
    edits
        .iter()
        .map(|edit| {
            if edit.with.is_empty() {
                return None; // a deletion leaves nothing to find
            }
            // The scope is rewritten with the edit applied.
            let scope = edit.scope.as_ref().map(|scope| {
                if edit.replace.is_empty() {
                    scope.clone()
                } else {
                    scope.replacen(&edit.replace, &edit.with, 1)
                }
            });
            match resolve(body, cuts, &edit.with, scope.as_deref()) {
                Resolved::At(start, end) => Some((start, end)),
                _ => None,
            }
        })
        .collect()
}

fn landing(ranges: &[Option<(usize, usize)>]) -> Landing {
    if ranges.iter().all(Option::is_some) {
        Landing::Located
    } else {
        Landing::Unlocatable
    }
}

/// The range each edit of an open suggestion would replace.
fn replace_ranges(body: &str, cuts: &[usize], edits: &[Edit]) -> Vec<Option<(usize, usize)>> {
    edits
        .iter()
        .map(|edit| match edit_range(body, cuts, edit) {
            Resolved::At(start, end) => Some((start, end)),
            _ => None,
        })
        .collect()
}

pub(crate) fn derive(body: &str, cuts: &[usize], items: &mut [Item], diags: &mut Vec<Diagnostic>) {
    let threads: HashMap<String, ThreadState> = items
        .iter()
        .filter_map(|item| match &item.detail {
            Detail::Comment(c) => c.thread.map(|t| (item.id.clone(), t)),
            _ => None,
        })
        .collect();

    // A change is in effect unless it was reverted by a revert that is itself
    // in effect. A revert always comes after its target, so walk backwards.
    let mut in_effect: HashMap<String, bool> = HashMap::new();
    for item in items.iter().rev() {
        let reverted_by = match &item.detail {
            Detail::Suggestion(s) => s.reverted_by.as_ref(),
            Detail::Edit(e) | Detail::Revert(e) => e.reverted_by.as_ref(),
            Detail::Comment(_) => None,
        };
        let undone = reverted_by.is_some_and(|by| in_effect.get(by).copied().unwrap_or(false));
        let live = !undone;
        in_effect.insert(item.id.clone(), live);
    }

    for item in items.iter_mut() {
        let live = in_effect[&item.id];
        match &mut item.detail {
            Detail::Comment(c) => {
                c.anchor = match &c.quote {
                    None => Anchor::Unquoted,
                    Some(q) => match resolve(body, cuts, &q.on, q.scope.as_deref()) {
                        Resolved::At(start, end) => Anchor::Attached { start, end },
                        Resolved::Detached => Anchor::Detached,
                        Resolved::Ambiguous => Anchor::Ambiguous,
                    },
                };
                // Only comments that are still live, in an open thread, need attention.
                let open = !c.retracted && threads.get(&c.root) == Some(&ThreadState::Open);
                let quoted = c.quote.as_ref().map_or("", |q| q.on.as_str());
                match c.anchor {
                    Anchor::Detached if open => diags.push(Diagnostic::new(
                        Code::Detached,
                        item.line,
                        Some(&item.id),
                        format!("quote {} is not in the body", shorten(quoted)),
                    )),
                    Anchor::Ambiguous if open => diags.push(Diagnostic::new(
                        Code::Ambiguous,
                        item.line,
                        Some(&item.id),
                        format!("quote {} matches more than once", shorten(quoted)),
                    )),
                    _ => {}
                }
            }
            Detail::Suggestion(s) => match s.state {
                SuggestionState::Open => {
                    let a = applicability(body, cuts, &s.edits);
                    s.applicability = Some(a);
                    s.ranges = replace_ranges(body, cuts, &s.edits);
                    if let Applicability::Stale(reason) = a {
                        let why = match reason {
                            StaleReason::Detached => "the text of an edit is not in the body",
                            StaleReason::Ambiguous => "the text of an edit matches more than once",
                            StaleReason::Overlap => "two of its edits overlap",
                        };
                        diags.push(Diagnostic::new(
                            Code::Stale,
                            item.line,
                            Some(&item.id),
                            format!("suggestion cannot be applied: {why}"),
                        ));
                    }
                }
                SuggestionState::Accepted => {
                    s.in_effect = live;
                    // A change that is not in effect has no new text to find.
                    if live {
                        s.ranges = landed_ranges(body, cuts, &s.edits);
                        s.landing = Some(landing(&s.ranges));
                    }
                }
                // Rejected and retracted suggestions are not located.
                SuggestionState::Rejected | SuggestionState::Retracted => {}
            },
            Detail::Edit(e) | Detail::Revert(e) => {
                e.in_effect = live;
                if live {
                    e.ranges = landed_ranges(body, cuts, &e.edits);
                    e.landing = Some(landing(&e.ranges));
                }
            }
        }
    }
}

/// A quote for a message: one line, at most 40 characters.
fn shorten(text: &str) -> String {
    let flat = text.replace('\n', "\\n");
    if flat.chars().count() <= 40 {
        format!("\"{flat}\"")
    } else {
        format!("\"{}…\"", flat.chars().take(39).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occurrences_overlap() {
        assert_eq!(resolve("aaa", &[], "aa", None), Resolved::Ambiguous);
        assert_eq!(resolve("aab", &[], "aa", None), Resolved::At(0, 2));
        assert_eq!(resolve("abc", &[], "x", None), Resolved::Detached);
    }

    #[test]
    fn scope_narrows_the_search() {
        let body = "retried 3 times, 3 nodes";
        assert_eq!(resolve(body, &[], "3", None), Resolved::Ambiguous);
        assert_eq!(resolve(body, &[], "3", Some("retried 3 times")), Resolved::At(8, 9));
        assert_eq!(resolve(body, &[], "3", Some("absent")), Resolved::Detached);
    }

    #[test]
    fn a_match_cannot_span_a_removed_record() {
        // "one\n" then a record was removed at offset 4, then "two".
        let body = "one\ntwo";
        assert_eq!(resolve(body, &[4], "one\ntwo", None), Resolved::Detached);
        assert_eq!(resolve(body, &[4], "one\n", None), Resolved::At(0, 4));
        assert_eq!(resolve(body, &[4], "two", None), Resolved::At(4, 7));
    }

    #[test]
    fn multibyte_text_is_matched_on_character_boundaries() {
        assert_eq!(resolve("ééé", &[], "éé", None), Resolved::Ambiguous);
        assert_eq!(resolve("日本語", &[], "本", None), Resolved::At(3, 6));
    }

    #[test]
    fn append_point_is_after_the_last_non_blank_line() {
        assert_eq!(append_point("a\nb\n\n\n"), 4);
        assert_eq!(append_point("a\nb"), 3);
        assert_eq!(append_point("\n\n"), 0);
        assert_eq!(append_point(""), 0);
    }
}
