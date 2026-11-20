//! Stored state (RFC 8.1): the log folded in file order.

use std::collections::HashMap;

use crate::diag::{Code, Diagnostic};
use crate::record::Record;
use crate::{Anchor, AppliedEdit, Comment, Detail, Event, Item, Suggestion, SuggestionState, ThreadState, Verb};

pub(crate) fn fold(records: Vec<Record>, diags: &mut Vec<Diagnostic>) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for rec in records {
        match rec.verb {
            Verb::Comment | Verb::Suggest | Verb::Edit | Verb::Revert => {
                if let Some(&first) = index.get(&rec.id) {
                    diags.push(Diagnostic::new(
                        Code::DuplicateId,
                        rec.line,
                        Some(&rec.id),
                        format!("id \"{}\" is already used on line {}", rec.id, items[first].line),
                    ));
                    continue;
                }
                if rec.verb == Verb::Revert {
                    // A revert that names nothing it can undo is ignored, and its id stays free.
                    if let Err((code, message)) = revert(&mut items, &index, &rec) {
                        diags.push(Diagnostic::new(code, rec.line, Some(&rec.id), message));
                        continue;
                    }
                }
                let applied = |edits| AppliedEdit {
                    edits,
                    landing: None,
                    ranges: Vec::new(),
                    reverted_by: None,
                    in_effect: true,
                };
                let detail = match rec.verb {
                    Verb::Comment => Detail::Comment(new_comment(&rec, &items, &index, diags)),
                    Verb::Suggest => Detail::Suggestion(Suggestion {
                        edits: rec.edits,
                        state: SuggestionState::Open,
                        applicability: None,
                        landing: None,
                        ranges: Vec::new(),
                        reverted_by: None,
                        in_effect: true,
                    }),
                    Verb::Revert => Detail::Revert(applied(rec.edits)),
                    _ => Detail::Edit(applied(rec.edits)),
                };
                index.insert(rec.id.clone(), items.len());
                items.push(Item {
                    id: rec.id,
                    line: rec.line,
                    by: rec.by,
                    at: rec.at,
                    on_behalf_of: rec.on_behalf_of,
                    re: rec.re,
                    note: rec.note,
                    detail,
                    events: Vec::new(),
                });
            }
            _ => {
                // An action must name an item that appears earlier in the file.
                let Some(&target) = index.get(&rec.id) else {
                    diags.push(Diagnostic::new(
                        Code::UnknownTarget,
                        rec.line,
                        Some(&rec.id),
                        format!(
                            "no item \"{}\" appears before this {} record",
                            rec.id,
                            rec.verb.as_str()
                        ),
                    ));
                    continue;
                };
                let item = &mut items[target];
                match act(item, &rec) {
                    Ok(()) => item.events.push(Event {
                        verb: rec.verb,
                        line: rec.line,
                        by: rec.by,
                        at: rec.at,
                        on_behalf_of: rec.on_behalf_of,
                        note: rec.note,
                    }),
                    Err((code, message)) => diags.push(Diagnostic::new(code, rec.line, Some(&rec.id), message)),
                }
            }
        }
    }
    items
}

/// Marks the change named by a revert's `re` as reverted, or says why a
/// reader ignores the revert. Only a change can be reverted: an accepted
/// suggestion, an edit or another revert, and each only once.
fn revert(items: &mut [Item], index: &HashMap<String, usize>, rec: &Record) -> Result<(), (Code, String)> {
    let re = rec.re.as_deref().unwrap_or_default();
    let Some(&target) = index.get(re) else {
        return Err((
            Code::UnknownTarget,
            format!("re=\"{re}\" names no item that appears before this revert"),
        ));
    };
    let item = &mut items[target];
    let kind = item.kind();
    let reverted_by = match &mut item.detail {
        Detail::Suggestion(s) if s.state == SuggestionState::Accepted => &mut s.reverted_by,
        Detail::Suggestion(s) => {
            return Err((
                Code::WrongTarget,
                format!(
                    "suggestion \"{re}\" is {}, not accepted, so there is no change to revert",
                    s.state.as_str()
                ),
            ))
        }
        Detail::Edit(e) | Detail::Revert(e) => &mut e.reverted_by,
        Detail::Comment(_) => return Err((Code::WrongTarget, format!("revert does not apply to comment \"{re}\""))),
    };
    if let Some(earlier) = reverted_by {
        return Err((
            Code::AlreadyFinal,
            format!("{kind} \"{re}\" is already reverted, by \"{earlier}\"; to undo that, revert \"{earlier}\""),
        ));
    }
    *reverted_by = Some(rec.id.clone());
    item.events.push(Event {
        verb: Verb::Revert,
        line: rec.line,
        by: rec.by.clone(),
        at: rec.at.clone(),
        on_behalf_of: rec.on_behalf_of.clone(),
        note: rec.note.clone(),
    });
    Ok(())
}

fn new_comment(rec: &Record, items: &[Item], index: &HashMap<String, usize>, diags: &mut Vec<Diagnostic>) -> Comment {
    // The root of a thread is a comment whose `re` is absent or names
    // something other than a comment.
    let root = match &rec.re {
        None => rec.id.clone(),
        Some(re) => match index.get(re).map(|&k| &items[k].detail) {
            Some(Detail::Comment(parent)) => parent.root.clone(),
            Some(_) => rec.id.clone(),
            None => {
                diags.push(Diagnostic::new(
                    Code::DanglingReply,
                    rec.line,
                    Some(&rec.id),
                    format!("re=\"{re}\" names no item that appears before this comment"),
                ));
                rec.id.clone()
            }
        },
    };
    let thread = (root == rec.id).then_some(ThreadState::Open);
    Comment {
        quote: rec.quote.clone(),
        original_quote: rec.quote.clone(),
        retracted: false,
        root,
        thread,
        anchor: Anchor::Unquoted,
    }
}

/// Applies an action record to its target, or says why a reader ignores it.
fn act(item: &mut Item, rec: &Record) -> Result<(), (Code, String)> {
    let verb = rec.verb.as_str();
    let not_author = |owner: &str| {
        (
            Code::NotAuthor,
            format!("\"{}\" was written by {owner}, not by {}", rec.id, rec.by),
        )
    };
    match (&mut item.detail, rec.verb) {
        (Detail::Suggestion(s), Verb::Accept | Verb::Reject | Verb::Retract) => {
            if rec.verb == Verb::Retract && rec.by != item.by {
                return Err(not_author(&item.by));
            }
            if s.state != SuggestionState::Open {
                return Err((
                    Code::AlreadyFinal,
                    format!("suggestion \"{}\" is already {}", rec.id, s.state.as_str()),
                ));
            }
            s.state = match rec.verb {
                Verb::Accept => SuggestionState::Accepted,
                Verb::Reject => SuggestionState::Rejected,
                _ => SuggestionState::Retracted,
            };
            Ok(())
        }
        (Detail::Comment(c), Verb::Retract) => {
            if rec.by != item.by {
                return Err(not_author(&item.by));
            }
            if c.retracted {
                return Err((
                    Code::AlreadyFinal,
                    format!("comment \"{}\" is already retracted", rec.id),
                ));
            }
            c.retracted = true;
            Ok(())
        }
        (Detail::Comment(c), Verb::Resolve | Verb::Reopen) => {
            if c.thread.is_none() {
                return Err((
                    Code::WrongTarget,
                    format!(
                        "\"{}\" is a reply; {verb} names the thread root, \"{}\"",
                        rec.id, c.root
                    ),
                ));
            }
            c.thread = Some(if rec.verb == Verb::Resolve {
                ThreadState::Resolved
            } else {
                ThreadState::Open
            });
            Ok(())
        }
        (Detail::Comment(c), Verb::Anchor) => {
            if c.retracted {
                return Err((Code::AlreadyFinal, format!("comment \"{}\" is retracted", rec.id)));
            }
            c.quote = rec.quote.clone();
            Ok(())
        }
        _ => Err((
            Code::WrongTarget,
            format!("{verb} does not apply to {} \"{}\"", item.kind(), rec.id),
        )),
    }
}
