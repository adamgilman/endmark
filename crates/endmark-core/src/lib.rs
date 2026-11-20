//! Reader and writer for Endmark, a collaboration layer for Markdown.
//!
//! An Endmark file is a Markdown body plus an append-only log of records,
//! each one an HTML comment that starts `<!--endmark `. This crate finds the
//! records, parses them, folds them into items with state, and checks every
//! anchor against the current body.
//!
//! The rules are in `rfcs/0001-endmark.md`. Section numbers in the comments
//! of this crate refer to that document.
//!
//! ```
//! let doc = endmark_core::parse("Hello world.\n\n<!--endmark comment c-1 by=ana at=2026-11-13T09:00:00Z\n```on\nworld\n```\nWhich one?\n-->\n");
//! assert_eq!(doc.body, "Hello world.\n\n");
//! assert_eq!(doc.items.len(), 1);
//! assert!(doc.diagnostics.is_empty());
//! ```

/// Compiles the example in the crate's README as a test.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExample;

mod anchor;
#[cfg(feature = "commonmark")]
mod commonmark;
pub mod diag;
pub mod escape;
#[cfg(feature = "json")]
pub mod json;
mod record;
mod scan;
mod state;
pub mod write;

pub use diag::{Code, Diagnostic, Severity};
pub use scan::START;

/// The ten verbs (RFC 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Comment,
    Suggest,
    Edit,
    Revert,
    Accept,
    Reject,
    Retract,
    Resolve,
    Reopen,
    Anchor,
}

impl Verb {
    pub fn from_name(name: &str) -> Option<Verb> {
        Some(match name {
            "comment" => Verb::Comment,
            "suggest" => Verb::Suggest,
            "edit" => Verb::Edit,
            "revert" => Verb::Revert,
            "accept" => Verb::Accept,
            "reject" => Verb::Reject,
            "retract" => Verb::Retract,
            "resolve" => Verb::Resolve,
            "reopen" => Verb::Reopen,
            "anchor" => Verb::Anchor,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Verb::Comment => "comment",
            Verb::Suggest => "suggest",
            Verb::Edit => "edit",
            Verb::Revert => "revert",
            Verb::Accept => "accept",
            Verb::Reject => "reject",
            Verb::Retract => "retract",
            Verb::Resolve => "resolve",
            Verb::Reopen => "reopen",
            Verb::Anchor => "anchor",
        }
    }
}

/// Body text a comment points at: the `on` fence, with an optional `in` scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    pub scope: Option<String>,
    pub on: String,
}

/// One replace/with pair of a suggestion, with an optional `in` scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub scope: Option<String>,
    pub replace: String,
    pub with: String,
}

/// Where a comment stands against the current body (RFC 8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The comment has no quote. It applies to the whole document or to its `re` target.
    Unquoted,
    /// The quote matches exactly once, at this byte range of the body.
    Attached { start: usize, end: usize },
    /// The quote, or its scope, is not in the body.
    Detached,
    /// The quote, or its scope, matches more than once.
    Ambiguous,
}

impl Anchor {
    pub fn as_str(self) -> &'static str {
        match self {
            Anchor::Unquoted => "unquoted",
            Anchor::Attached { .. } => "attached",
            Anchor::Detached => "detached",
            Anchor::Ambiguous => "ambiguous",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Open,
    Resolved,
}

impl ThreadState {
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadState::Open => "open",
            ThreadState::Resolved => "resolved",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionState {
    Open,
    Accepted,
    Rejected,
    Retracted,
}

impl SuggestionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SuggestionState::Open => "open",
            SuggestionState::Accepted => "accepted",
            SuggestionState::Rejected => "rejected",
            SuggestionState::Retracted => "retracted",
        }
    }
}

/// Why an open suggestion cannot be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    Detached,
    Ambiguous,
    Overlap,
}

impl StaleReason {
    pub fn as_str(self) -> &'static str {
        match self {
            StaleReason::Detached => "detached",
            StaleReason::Ambiguous => "ambiguous",
            StaleReason::Overlap => "overlap",
        }
    }
}

/// Whether an open suggestion can be applied to the current body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applicability {
    Applicable,
    Stale(StaleReason),
}

/// Whether the new text of an applied change can still be found in the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    Located,
    Unlocatable,
}

impl Landing {
    pub fn as_str(self) -> &'static str {
        match self {
            Landing::Located => "located",
            Landing::Unlocatable => "unlocatable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// The quote in force: the latest `anchor` record, or the original.
    pub quote: Option<Quote>,
    pub original_quote: Option<Quote>,
    /// Retraction is final and withdraws this one comment only.
    pub retracted: bool,
    /// The id of the thread's root comment. A root names itself.
    pub root: String,
    /// The thread state. Present on root comments only.
    pub thread: Option<ThreadState>,
    pub anchor: Anchor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub edits: Vec<Edit>,
    pub state: SuggestionState,
    /// Present while the suggestion is open.
    pub applicability: Option<Applicability>,
    /// Present while the suggestion is accepted and in effect.
    pub landing: Option<Landing>,
    /// One entry per edit: the byte range in the body that an open
    /// suggestion would replace, or where the new text of a change in effect
    /// is now. `None` when it cannot be found.
    pub ranges: Vec<Option<(usize, usize)>>,
    /// The id of the `revert` that undid this suggestion, if any.
    pub reverted_by: Option<String>,
    /// False when the change has been undone and not brought back (RFC 8.2).
    pub in_effect: bool,
}

/// A change already made to the body and logged with the `edit` or the
/// `revert` verb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedEdit {
    pub edits: Vec<Edit>,
    /// Present while the change is in effect.
    pub landing: Option<Landing>,
    /// One entry per edit: the byte range in the body that an open
    /// suggestion would replace, or where the new text of a change in effect
    /// is now. `None` when it cannot be found.
    pub ranges: Vec<Option<(usize, usize)>>,
    /// The id of the `revert` that undid this change, if any.
    pub reverted_by: Option<String>,
    /// False when the change has been undone and not brought back (RFC 8.2).
    pub in_effect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    Comment(Comment),
    Suggestion(Suggestion),
    Edit(AppliedEdit),
    /// A change that undoes the change named by the item's `re`.
    Revert(AppliedEdit),
}

/// An action record that took effect on an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub verb: Verb,
    pub line: usize,
    pub by: String,
    pub at: String,
    pub on_behalf_of: Option<String>,
    pub note: String,
}

/// A comment, a suggestion, a logged edit or a revert, with its stored and derived state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    /// 1-based line of the record that created the item.
    pub line: usize,
    pub by: String,
    pub at: String,
    /// The `for` attribute: the person an agent acted for.
    pub on_behalf_of: Option<String>,
    pub re: Option<String>,
    pub note: String,
    pub detail: Detail,
    pub events: Vec<Event>,
}

impl Item {
    pub fn kind(&self) -> &'static str {
        match self.detail {
            Detail::Comment(_) => "comment",
            Detail::Suggestion(_) => "suggestion",
            Detail::Edit(_) => "edit",
            Detail::Revert(_) => "revert",
        }
    }
}

/// A parsed Endmark file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The file with every record line removed, with CRLF normalised to LF.
    pub body: String,
    /// Items in file order.
    pub items: Vec<Item>,
    /// Findings in line order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Document {
    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    fn count(&self, severity: Severity) -> usize {
        self.diagnostics.iter().filter(|d| d.severity() == severity).count()
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }
}

/// Reads a file. This never fails: anything wrong with the file is reported
/// in [`Document::diagnostics`].
pub fn parse(source: &str) -> Document {
    let text = source.replace("\r\n", "\n");
    let scanned = scan::scan(&text);

    let mut diagnostics = Vec::new();
    let records: Vec<record::Record> = scanned
        .records
        .iter()
        .filter_map(|raw| record::parse_record(raw, &mut diagnostics))
        .collect();

    if let Some(line) = scanned.body_after_log {
        diagnostics.push(Diagnostic::new(
            Code::BodyAfterLog,
            line,
            None,
            "body text follows a record",
        ));
    }
    #[cfg(feature = "commonmark")]
    commonmark::check(&text, &scanned.records, &mut diagnostics);

    let mut items = state::fold(records, &mut diagnostics);
    anchor::derive(&scanned.body, &scanned.cuts, &mut items, &mut diagnostics);

    diagnostics.sort_by_key(|d| d.line);
    Document {
        body: scanned.body,
        items,
        diagnostics,
    }
}
