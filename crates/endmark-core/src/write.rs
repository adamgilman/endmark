//! The write path (RFC 9): one function per verb.
//!
//! A write takes the source of a file and returns the new source, or fails
//! and changes nothing. Putting the result on disk is the caller's job.
//!
//! Every write is read back before it is returned. A write that a reader
//! would ignore, or would understand differently, is refused.

use std::fmt;

use crate::anchor::{self, Resolved};
use crate::escape::escape;
use crate::scan::{self, Scan, START};
use crate::{
    parse, Anchor, Applicability, Detail, Document, Edit, Item, Quote, StaleReason, SuggestionState, ThreadState, Verb,
};

/// Who is writing, when, and what they have to say.
#[derive(Debug, Clone, Copy)]
pub struct Stamp<'a> {
    pub by: &'a str,
    /// RFC 3339 in UTC, such as `2026-11-13T09:30:00Z`.
    pub at: &'a str,
    /// The `for` attribute: the person an agent acts for.
    pub on_behalf_of: Option<&'a str>,
    /// The note, in Markdown. May be empty.
    pub note: &'a str,
}

/// The result of a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The whole file after the write.
    pub source: String,
    /// The id of the item created or acted on. For `resolve` and `reopen`
    /// given a reply, this is the thread root.
    pub id: String,
    /// For a revert, the change the record names in `re`: the tip of the
    /// chain of the id that was given.
    pub target: Option<String>,
    /// Comments re-pointed by an `anchor` record in the same write.
    pub reanchored: Vec<String>,
    /// Open comments this write detached and could not re-point.
    pub detached: Vec<String>,
    /// Open suggestions this write made stale.
    pub stale: Vec<String>,
}

/// Why a write was refused. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// No item has this id.
    NotFound { id: String },
    /// The item is a comment, or a suggestion that was never accepted.
    NotAChange { id: String, what: String },
    /// The change is already undone. Reverting `tip` would bring it back.
    NotInEffect { id: String, tip: String },
    /// The target deleted text without an `in` scope, so the place is not recorded.
    UnscopedDeletion { target: String },
    /// The text the target wrote can no longer be found exactly once.
    TextChanged {
        target: String,
        text: String,
        why: &'static str,
    },
    /// The text to change sits after the log, where a writer does not edit.
    AfterLog,
    /// The id for the new record is already taken.
    IdInUse { id: String },
    /// A value cannot be written in a header.
    BadValue { what: &'static str, value: String },
    /// The new records would not be read back as written.
    Unreadable { why: &'static str },
    /// Quoted text is not in the body.
    QuoteNotFound { text: String },
    /// Quoted text, or its scope, is in the body more than once.
    QuoteAmbiguous { text: String },
    /// Two edits of one change touch the same text.
    Overlap,
    /// The suggestion cannot be applied to the current body.
    Stale { id: String, why: &'static str },
    /// The record would say nothing: no quote, no note, no edits.
    Empty { what: &'static str },
    /// The write would change nothing.
    NoChange { id: String, what: &'static str },
    /// A reader would ignore the record. `code` is the linter's code for why.
    Ignored { code: &'static str, message: String },
}

impl WriteError {
    /// A stable code, for tools and for the writer fixtures.
    pub fn code(&self) -> &'static str {
        match self {
            WriteError::NotFound { .. } => "not-found",
            WriteError::NotAChange { .. } => "not-a-change",
            WriteError::NotInEffect { .. } => "not-in-effect",
            WriteError::UnscopedDeletion { .. } => "unscoped-deletion",
            WriteError::TextChanged { .. } => "text-changed",
            WriteError::AfterLog => "after-log",
            WriteError::IdInUse { .. } => "id-in-use",
            WriteError::BadValue { .. } => "bad-value",
            WriteError::Unreadable { .. } => "unreadable",
            WriteError::QuoteNotFound { .. } => "quote-not-found",
            WriteError::QuoteAmbiguous { .. } => "quote-ambiguous",
            WriteError::Overlap => "overlap",
            WriteError::Stale { .. } => "stale",
            WriteError::Empty { .. } => "empty",
            WriteError::NoChange { .. } => "no-change",
            WriteError::Ignored { code, .. } => code,
        }
    }

    /// What to do about it, in one line.
    pub fn hint(&self) -> String {
        match self {
            WriteError::NotFound { .. } => "run endmark status to list the items in the file".into(),
            WriteError::NotAChange { .. } => "only an accepted suggestion, an edit or a revert can be reverted".into(),
            WriteError::NotInEffect { tip, .. } => format!("to bring it back, revert \"{tip}\""),
            WriteError::UnscopedDeletion { .. } => {
                "put the text back with a suggestion; a deletion needs an in fence to be revertible".into()
            }
            WriteError::TextChanged { .. } => {
                "the document changed after this was applied; make the change by hand or with a suggestion".into()
            }
            WriteError::AfterLog => "move the body text above the first record, then try again".into(),
            WriteError::IdInUse { .. } => "choose another id, or leave it out to get a random one".into(),
            WriteError::BadValue { .. } => "header values cannot be empty or contain spaces or \"-->\"".into(),
            WriteError::Unreadable { .. } => "run endmark lint on the file and fix what it reports first".into(),
            WriteError::QuoteNotFound { .. } => {
                "quote the Markdown source exactly as it is in the file, not the rendered text".into()
            }
            WriteError::QuoteAmbiguous { .. } => {
                "quote more text, or give the surrounding text as the scope (--in)".into()
            }
            WriteError::Overlap => "merge the two edits into one".into(),
            WriteError::Stale { .. } => "retract the suggestion and suggest again against the current text".into(),
            WriteError::Empty { .. } => "give a note, quoted text, or both".into(),
            WriteError::NoChange { .. } => "nothing to do".into(),
            WriteError::Ignored { .. } => "a reader would ignore this record, so it was not written".into(),
        }
    }
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::NotFound { id } => write!(f, "no item \"{id}\" in the file"),
            WriteError::NotAChange { id, what } => write!(f, "\"{id}\" is {what}, so there is no change to revert"),
            WriteError::NotInEffect { id, tip } => {
                if id == tip {
                    write!(f, "\"{id}\" is not in effect")
                } else {
                    write!(
                        f,
                        "\"{id}\" is already undone; the latest revert in its chain is \"{tip}\""
                    )
                }
            }
            WriteError::UnscopedDeletion { target } => write!(
                f,
                "\"{target}\" deleted text without recording where, so it cannot be put back automatically"
            ),
            WriteError::TextChanged { target, text, why } => {
                write!(f, "the text written by \"{target}\", {}, {why}", shorten(text))
            }
            WriteError::AfterLog => write!(f, "the text to change is after the first record in the file"),
            WriteError::IdInUse { id } => write!(f, "id \"{id}\" is already used in the file"),
            WriteError::BadValue { what, value } => {
                write!(f, "{what} \"{value}\" cannot be written in a record header")
            }
            WriteError::Unreadable { why } => write!(f, "the new record would not be read back as written: {why}"),
            WriteError::QuoteNotFound { text } => write!(f, "{} is not in the document", shorten(text)),
            WriteError::QuoteAmbiguous { text } => write!(f, "{} is in the document more than once", shorten(text)),
            WriteError::Overlap => write!(f, "two of the edits touch the same text"),
            WriteError::Stale { id, why } => write!(f, "suggestion \"{id}\" cannot be applied: {why}"),
            WriteError::Empty { what } => write!(f, "{what} would be empty"),
            WriteError::NoChange { id, what } => write!(f, "\"{id}\" is already {what}"),
            WriteError::Ignored { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for WriteError {}

fn shorten(text: &str) -> String {
    let flat = text.replace('\n', "\\n");
    if flat.chars().count() <= 40 {
        format!("\"{flat}\"")
    } else {
        format!("\"{}…\"", flat.chars().take(39).collect::<String>())
    }
}

/// Edits a write applied to the body, and where each landed.
type Applied<'a> = (&'a [Edit], &'a [(usize, usize)]);

// ---- reading the file ----

struct File {
    crlf: bool,
    text: String,
    doc: Document,
    scan: Scan,
}

fn open(source: &str, stamp: &Stamp, new_id: Option<&str>) -> Result<File, WriteError> {
    check_value("author", stamp.by, false)?;
    check_value("time", stamp.at, false)?;
    if let Some(person) = stamp.on_behalf_of {
        check_value("for", person, false)?;
    }
    let text = source.replace("\r\n", "\n");
    let file = File {
        crlf: source.contains("\r\n"),
        doc: parse(&text),
        scan: scan::scan(&text),
        text,
    };
    if let Some(id) = new_id {
        check_value("id", id, true)?;
        if file.doc.item(id).is_some() {
            return Err(WriteError::IdInUse { id: id.to_string() });
        }
    }
    Ok(file)
}

impl File {
    fn item(&self, id: &str) -> Result<&Item, WriteError> {
        self.doc
            .item(id)
            .ok_or_else(|| WriteError::NotFound { id: id.to_string() })
    }

    fn resolve_quote(&self, quote: &Quote) -> Result<(), WriteError> {
        if quote.on.is_empty() || quote.scope.as_deref() == Some("") {
            return Err(WriteError::Empty { what: "the quote" });
        }
        match anchor::resolve(&self.scan.body, &self.scan.cuts, &quote.on, quote.scope.as_deref()) {
            Resolved::At(..) => Ok(()),
            Resolved::Detached => Err(WriteError::QuoteNotFound { text: quote.on.clone() }),
            Resolved::Ambiguous => Err(WriteError::QuoteAmbiguous { text: quote.on.clone() }),
        }
    }

    /// The range each edit replaces. `on_miss` builds the error for an edit
    /// that does not match exactly once; its flag says the match is ambiguous.
    fn locate(
        &self,
        edits: &[Edit],
        on_miss: impl Fn(&Edit, bool) -> WriteError,
    ) -> Result<Vec<(usize, usize)>, WriteError> {
        let (body, cuts) = (&self.scan.body, &self.scan.cuts);
        let mut ranges = Vec::with_capacity(edits.len());
        for edit in edits {
            match anchor::edit_range(body, cuts, edit) {
                Resolved::At(start, end) => ranges.push((start, end)),
                Resolved::Detached => return Err(on_miss(edit, false)),
                Resolved::Ambiguous => return Err(on_miss(edit, true)),
            }
        }
        if anchor::applicability(body, cuts, edits) == Applicability::Stale(StaleReason::Overlap) {
            return Err(WriteError::Overlap);
        }
        Ok(ranges)
    }

    /// Applies all edits together. Returns the new text and where each edit landed.
    fn apply(&self, edits: &[Edit], ranges: &[(usize, usize)]) -> Result<(String, Vec<(usize, usize)>), WriteError> {
        // Offsets in the body equal offsets in the file up to the first record.
        let limit = self.scan.cuts.first().copied().unwrap_or(self.scan.body.len());
        if ranges.iter().any(|r| r.1 > limit) {
            return Err(WriteError::AfterLog);
        }
        let mut order: Vec<usize> = (0..edits.len()).collect();
        order.sort_by_key(|&n| ranges[n]);
        let mut out = self.text.clone();
        // Last first, so earlier offsets stay valid.
        for &n in order.iter().rev() {
            out.replace_range(ranges[n].0..ranges[n].1, &edits[n].with);
        }
        let mut landed = vec![(0usize, 0usize); edits.len()];
        let mut shift = 0isize;
        for &n in &order {
            let start = ranges[n].0.wrapping_add_signed(shift);
            landed[n] = (start, start + edits[n].with.len());
            shift += edits[n].with.len() as isize - (ranges[n].1 - ranges[n].0) as isize;
        }
        Ok((out, landed))
    }

    /// Appends the record, repairs anchors if the body changed (RFC 9.4),
    /// reads the result back, and reports what the write did to other items.
    fn finish(
        &self,
        mut out: String,
        record: &str,
        stamp: &Stamp,
        applied: Option<Applied>,
        id: &str,
        expect: impl Fn(&Document) -> Result<(), &'static str>,
    ) -> Result<Written, WriteError> {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        let old_lines = out.matches('\n').count();
        append(&mut out, record);

        let mut reanchored = Vec::new();
        if let Some((edits, landed)) = applied {
            let middle = parse(&out);
            let rescanned = scan::scan(&out);
            for item in &self.doc.items {
                if let Some(quote) = repair(item, &middle, &rescanned, edits, landed) {
                    append(&mut out, &anchor_record(&item.id, stamp, &quote, ""));
                    reanchored.push(item.id.clone());
                }
            }
        }

        let after = parse(&out);
        // Anything a reader reports about the new records means the write is wrong.
        if let Some(d) = after.diagnostics.iter().find(|d| d.line > old_lines) {
            return Err(WriteError::Ignored {
                code: d.code.as_str(),
                message: d.message.clone(),
            });
        }
        expect(&after).map_err(|why| WriteError::Unreadable { why })?;
        if after.errors() > self.doc.errors() {
            return Err(WriteError::Unreadable {
                why: "it would add errors to the file",
            });
        }
        let attached = |doc: &Document, id: &str| {
            matches!(
                doc.item(id).map(|i| &i.detail),
                Some(Detail::Comment(c)) if matches!(c.anchor, Anchor::Attached { .. })
            )
        };
        if !reanchored.iter().all(|id| attached(&after, id)) {
            return Err(WriteError::Unreadable {
                why: "a re-pointed comment would not attach",
            });
        }

        // What the write did to items it did not name.
        let warned = |doc: &Document, id: &str| doc.diagnostics.iter().any(|d| d.id.as_deref() == Some(id));
        let mut detached = Vec::new();
        let mut stale = Vec::new();
        for item in &self.doc.items {
            match &item.detail {
                Detail::Comment(_) if attached(&self.doc, &item.id) && !attached(&after, &item.id) => {
                    if warned(&after, &item.id) {
                        detached.push(item.id.clone());
                    }
                }
                Detail::Suggestion(s) if s.applicability == Some(Applicability::Applicable) => {
                    let now = after.item(&item.id).map(|i| &i.detail);
                    if matches!(now, Some(Detail::Suggestion(n)) if matches!(n.applicability, Some(Applicability::Stale(_))))
                    {
                        stale.push(item.id.clone());
                    }
                }
                _ => {}
            }
        }

        Ok(Written {
            source: if self.crlf { out.replace('\n', "\r\n") } else { out },
            id: id.to_string(),
            target: None,
            reanchored,
            detached,
            stale,
        })
    }
}

// ---- writing records ----

/// A fence around literal text. Longer than any run of backticks inside it.
fn fence(out: &mut String, info: &str, text: &str) {
    let (mut longest, mut run) = (0, 0);
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let ticks = "`".repeat(longest.max(2) + 1);
    out.push_str(&ticks);
    out.push_str(info);
    out.push('\n');
    if !text.is_empty() {
        out.push_str(&escape(text));
        out.push('\n');
    }
    out.push_str(&ticks);
    out.push('\n');
}

fn header(verb: &str, id: &str, re: Option<&str>, stamp: &Stamp) -> String {
    let mut h = format!("{START}{verb} {id}");
    if let Some(re) = re {
        h.push_str(&format!(" re={re}"));
    }
    h.push_str(&format!(" by={}", stamp.by));
    if let Some(person) = stamp.on_behalf_of {
        h.push_str(&format!(" for={person}"));
    }
    h.push_str(&format!(" at={}", stamp.at));
    h
}

/// A `suggest`, `edit` or `revert` record: the note, then the edits.
fn change_record(verb: &str, id: &str, re: Option<&str>, stamp: &Stamp, edits: &[Edit]) -> String {
    let mut out = header(verb, id, re, stamp);
    out.push('\n');
    let note = stamp.note.trim();
    if !note.is_empty() {
        out.push_str(&escape(note));
        out.push('\n');
    }
    for (n, edit) in edits.iter().enumerate() {
        if !note.is_empty() || n > 0 {
            out.push('\n');
        }
        if let Some(scope) = &edit.scope {
            fence(&mut out, "in", scope);
        }
        fence(&mut out, "replace", &edit.replace);
        fence(&mut out, "with", &edit.with);
    }
    out.push_str("-->\n");
    out
}

/// A `comment` record: the quote, then the note.
fn comment_record(id: &str, re: Option<&str>, stamp: &Stamp, quote: Option<&Quote>) -> String {
    let mut out = header("comment", id, re, stamp);
    out.push('\n');
    push_quote(&mut out, quote);
    push_note(&mut out, stamp.note);
    out.push_str("-->\n");
    out
}

fn anchor_record(id: &str, stamp: &Stamp, quote: &Quote, note: &str) -> String {
    let mut out = header("anchor", id, None, stamp);
    out.push('\n');
    push_quote(&mut out, Some(quote));
    push_note(&mut out, note);
    out.push_str("-->\n");
    out
}

/// An action with no content is written on one line.
fn action_record(verb: &str, id: &str, stamp: &Stamp) -> String {
    let mut out = header(verb, id, None, stamp);
    if stamp.note.trim().is_empty() {
        out.push_str(" -->\n");
    } else {
        out.push('\n');
        push_note(&mut out, stamp.note);
        out.push_str("-->\n");
    }
    out
}

fn push_quote(out: &mut String, quote: Option<&Quote>) {
    if let Some(quote) = quote {
        if let Some(scope) = &quote.scope {
            fence(out, "in", scope);
        }
        fence(out, "on", &quote.on);
    }
}

fn push_note(out: &mut String, note: &str) {
    let note = note.trim();
    if !note.is_empty() {
        out.push_str(&escape(note));
        out.push('\n');
    }
}

/// RFC 9.1: a blank line, then the record, at the end of the file.
fn append(text: &mut String, record: &str) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push('\n');
    text.push_str(record);
}

fn check_value(what: &'static str, value: &str, is_id: bool) -> Result<(), WriteError> {
    let ok = !value.is_empty()
        && !value.chars().any(char::is_whitespace)
        && !value.contains("-->")
        && !value.contains("--!>")
        && (!is_id
            || value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
    if ok {
        Ok(())
    } else {
        Err(WriteError::BadValue {
            what,
            value: value.to_string(),
        })
    }
}

fn same_note(read: &str, written: &str) -> Result<(), &'static str> {
    if read == written.trim() {
        Ok(())
    } else {
        Err("the note contains lines that a reader takes for record syntax")
    }
}

const NOT_FOUND: &str = "a reader would not find it; the file may end inside an open code fence";

fn missing(edit: &Edit, ambiguous: bool) -> WriteError {
    let text = edit.replace.clone();
    if ambiguous {
        WriteError::QuoteAmbiguous { text }
    } else {
        WriteError::QuoteNotFound { text }
    }
}

// ---- the verbs ----

/// Adds a comment (RFC 7). With a quote it points at body text, which must
/// match exactly once. With `re` it replies to a comment or remarks on
/// another item. It needs a quote, a note, or both.
pub fn comment(
    source: &str,
    new_id: &str,
    stamp: &Stamp,
    quote: Option<&Quote>,
    re: Option<&str>,
) -> Result<Written, WriteError> {
    let file = open(source, stamp, Some(new_id))?;
    if let Some(re) = re {
        file.item(re)?;
    }
    if let Some(quote) = quote {
        file.resolve_quote(quote)?;
    }
    if quote.is_none() && stamp.note.trim().is_empty() {
        return Err(WriteError::Empty { what: "the comment" });
    }
    let record = comment_record(new_id, re, stamp, quote);
    file.finish(file.text.clone(), &record, stamp, None, new_id, |after| {
        let item = after.item(new_id).ok_or(NOT_FOUND)?;
        match &item.detail {
            Detail::Comment(c) if c.quote.as_ref() == quote && item.re.as_deref() == re => {}
            _ => return Err("its quote would be read differently"),
        }
        same_note(&item.note, stamp.note)
    })
}

/// Adds a suggestion (RFC 7). Every edit must match exactly once, and no
/// two may overlap. The body is not changed.
pub fn suggest(source: &str, new_id: &str, stamp: &Stamp, edits: &[Edit]) -> Result<Written, WriteError> {
    let file = open(source, stamp, Some(new_id))?;
    if edits.is_empty() {
        return Err(WriteError::Empty { what: "the suggestion" });
    }
    file.locate(edits, missing)?;
    let record = change_record("suggest", new_id, None, stamp, edits);
    file.finish(file.text.clone(), &record, stamp, None, new_id, |after| {
        let item = after.item(new_id).ok_or(NOT_FOUND)?;
        match &item.detail {
            Detail::Suggestion(s) if s.edits == edits => {}
            _ => return Err("its edits would be read differently"),
        }
        same_note(&item.note, stamp.note)
    })
}

/// Changes the body and logs the change (RFC 7, `edit`).
pub fn edit(source: &str, new_id: &str, stamp: &Stamp, edits: &[Edit]) -> Result<Written, WriteError> {
    let file = open(source, stamp, Some(new_id))?;
    if edits.is_empty() {
        return Err(WriteError::Empty { what: "the edit" });
    }
    let ranges = file.locate(edits, missing)?;
    let (out, landed) = file.apply(edits, &ranges)?;
    let record = change_record("edit", new_id, None, stamp, edits);
    file.finish(out, &record, stamp, Some((edits, &landed)), new_id, |after| {
        let item = after.item(new_id).ok_or(NOT_FOUND)?;
        match &item.detail {
            Detail::Edit(e) if e.edits == edits => {}
            _ => return Err("its edits would be read differently"),
        }
        same_note(&item.note, stamp.note)
    })
}

/// Accepts a suggestion: applies its edits and appends the record (RFC 9.2).
pub fn accept(source: &str, id: &str, stamp: &Stamp) -> Result<Written, WriteError> {
    let file = open(source, stamp, None)?;
    let item = file.item(id)?;
    let Detail::Suggestion(suggestion) = &item.detail else {
        return Err(WriteError::Ignored {
            code: "wrong-target",
            message: format!("accept does not apply to {} \"{id}\"", item.kind()),
        });
    };
    if suggestion.state != SuggestionState::Open {
        return Err(WriteError::Ignored {
            code: "already-final",
            message: format!("suggestion \"{id}\" is already {}", suggestion.state.as_str()),
        });
    }
    let edits = &suggestion.edits;
    let stale = |why| WriteError::Stale {
        id: id.to_string(),
        why,
    };
    let ranges = file
        .locate(edits, |_, ambiguous| {
            stale(if ambiguous {
                "the text of an edit matches more than once"
            } else {
                "the text of an edit is not in the document"
            })
        })
        .map_err(|e| match e {
            WriteError::Overlap => stale("two of its edits overlap"),
            other => other,
        })?;
    let (out, landed) = file.apply(edits, &ranges)?;
    let record = action_record("accept", id, stamp);
    file.finish(out, &record, stamp, Some((edits, &landed)), id, |after| {
        match after.item(id).map(|i| &i.detail) {
            Some(Detail::Suggestion(s)) if s.state == SuggestionState::Accepted => Ok(()),
            _ => Err(NOT_FOUND),
        }
    })
}

/// Appends `reject`, `retract`, `resolve` or `reopen` (RFC 7 and 8.1).
///
/// For `resolve` and `reopen`, `id` may be any comment of the thread: the
/// record names the root.
pub fn act(source: &str, verb: Verb, id: &str, stamp: &Stamp) -> Result<Written, WriteError> {
    let file = open(source, stamp, None)?;
    let item = file.item(id)?;
    let mut id = id;
    if let (Verb::Resolve | Verb::Reopen, Detail::Comment(c)) = (verb, &item.detail) {
        id = &c.root;
        let root = file.item(id)?;
        let state = match &root.detail {
            Detail::Comment(c) => c.thread,
            _ => None,
        };
        let same = match verb {
            Verb::Resolve => state == Some(ThreadState::Resolved),
            _ => state == Some(ThreadState::Open),
        };
        if same {
            return Err(WriteError::NoChange {
                id: id.to_string(),
                what: if verb == Verb::Resolve { "resolved" } else { "open" },
            });
        }
    }
    if !matches!(verb, Verb::Reject | Verb::Retract | Verb::Resolve | Verb::Reopen) {
        return Err(WriteError::Ignored {
            code: "wrong-target",
            message: format!("{} is not written this way", verb.as_str()),
        });
    }
    let record = action_record(verb.as_str(), id, stamp);
    // A record a reader would ignore, such as a retract by someone else, is
    // caught when the result is read back.
    file.finish(file.text.clone(), &record, stamp, None, id, |after| {
        let event = after.item(id).and_then(|i| i.events.last()).ok_or(NOT_FOUND)?;
        if event.verb != verb {
            return Err(NOT_FOUND);
        }
        same_note(&event.note, stamp.note)
    })
}

/// Re-points a comment at new text (RFC 7, `anchor`).
pub fn anchor(source: &str, id: &str, stamp: &Stamp, quote: &Quote) -> Result<Written, WriteError> {
    let file = open(source, stamp, None)?;
    file.item(id)?;
    file.resolve_quote(quote)?;
    let record = anchor_record(id, stamp, quote, stamp.note);
    file.finish(file.text.clone(), &record, stamp, None, id, |after| {
        match after.item(id).map(|i| &i.detail) {
            Some(Detail::Comment(c)) if c.quote.as_ref() == Some(quote) => Ok(()),
            _ => Err("its quote would be read differently"),
        }
    })
}

/// The edits of a change, the revert that names it, and whether it is in
/// effect. `Err` says what the item is instead.
fn change(item: &Item) -> Result<(&[Edit], Option<&str>, bool), String> {
    match &item.detail {
        Detail::Suggestion(s) if s.state == SuggestionState::Accepted => {
            Ok((&s.edits, s.reverted_by.as_deref(), s.in_effect))
        }
        Detail::Suggestion(s) => Err(format!("a suggestion that is {}, not accepted", s.state.as_str())),
        Detail::Edit(e) | Detail::Revert(e) => Ok((&e.edits, e.reverted_by.as_deref(), e.in_effect)),
        Detail::Comment(_) => Err("a comment".into()),
    }
}

/// The edit that undoes `edit`, or `None` when the place of a deletion was
/// not recorded (RFC 9.3).
fn invert(edit: &Edit) -> Option<Edit> {
    if edit.with.is_empty() {
        // A deletion. Its plain inverse would append to the end of the body.
        // With a scope, replace the scope as it reads now with the scope as
        // it was written.
        let scope = edit.scope.as_ref()?;
        if edit.replace.is_empty() {
            return None;
        }
        let now = scope.replacen(&edit.replace, "", 1);
        if now.is_empty() {
            return None;
        }
        return Some(Edit {
            scope: None,
            replace: now,
            with: scope.clone(),
        });
    }
    // The scope is rewritten with the edit applied. An append has nothing to
    // rewrite, and its inverse gets a scope of its own below.
    let scope = if edit.replace.is_empty() {
        None
    } else {
        edit.scope.as_ref().map(|s| s.replacen(&edit.replace, &edit.with, 1))
    };
    Some(Edit {
        scope,
        replace: edit.with.clone(),
        with: edit.replace.clone(),
    })
}

/// The whole lines around `start..end`, widened upwards until they hold
/// something besides the range itself. Used as the `in` scope of a deletion,
/// so that the deletion can be reverted in turn.
fn lines_around(body: &str, start: usize, end: usize, need_more: bool) -> Option<&str> {
    let mut from = body[..start].rfind('\n').map_or(0, |p| p + 1);
    let to = body[end..].find('\n').map_or(body.len(), |p| end + p);
    if need_more {
        while body[from..start].trim().is_empty() && body[end..to].trim().is_empty() {
            if from == 0 {
                return None;
            }
            from = body[..from - 1].rfind('\n').map_or(0, |p| p + 1);
        }
    }
    Some(&body[from..to])
}

/// Reverts a change (RFC 9.3 and 9.4).
///
/// `id` may be any link of a chain: the change itself or any revert in its
/// chain. If that link is in effect, the tip of the chain is reverted, which
/// undoes it. If it is not in effect, the write is refused.
///
/// `new_id` is the id for the new revert record.
pub fn revert(source: &str, id: &str, new_id: &str, stamp: &Stamp) -> Result<Written, WriteError> {
    let file = open(source, stamp, Some(new_id))?;
    let (body, cuts) = (&file.scan.body, &file.scan.cuts);
    let given = file.item(id)?;
    let (_, _, in_effect) = change(given).map_err(|what| WriteError::NotAChange {
        id: id.to_string(),
        what,
    })?;

    // Follow the chain to its tip: the one link nothing has reverted.
    let mut tip = given;
    while let Ok((_, Some(next), _)) = change(tip) {
        match file.doc.item(next) {
            Some(item) => tip = item,
            None => break,
        }
    }
    if !in_effect {
        return Err(WriteError::NotInEffect {
            id: id.to_string(),
            tip: tip.id.clone(),
        });
    }
    let target = tip.id.as_str();
    let (edits, _, _) = change(tip).expect("a link of a chain is a change");

    // The inverse of every edit, resolved against the current body.
    let mut inverse = Vec::with_capacity(edits.len());
    for edit in edits {
        inverse.push(invert(edit).ok_or_else(|| WriteError::UnscopedDeletion {
            target: target.to_string(),
        })?);
    }
    let changed = |text: &str, why| WriteError::TextChanged {
        target: target.to_string(),
        text: text.to_string(),
        why,
    };
    let ranges = file
        .locate(&inverse, |edit, ambiguous| {
            changed(
                &edit.replace,
                if ambiguous {
                    "now appears more than once"
                } else {
                    "is no longer in the document"
                },
            )
        })
        .map_err(|e| match e {
            WriteError::Overlap => changed(&inverse[0].replace, "overlaps another edit of the same change"),
            other => other,
        })?;
    // Give a deletion a scope, so that this revert can be reverted too.
    for (inv, &(start, end)) in inverse.iter_mut().zip(&ranges) {
        if inv.with.is_empty() && inv.scope.is_none() {
            if let Some(scope) = lines_around(body, start, end, true) {
                if anchor::resolve(body, cuts, &inv.replace, Some(scope)) == Resolved::At(start, end) {
                    inv.scope = Some(scope.to_string());
                }
            }
        }
    }
    let (out, landed) = file.apply(&inverse, &ranges)?;
    let record = change_record("revert", new_id, Some(target), stamp, &inverse);
    let mut written = file.finish(out, &record, stamp, Some((&inverse, &landed)), new_id, |after| {
        let item = after.item(new_id).ok_or(NOT_FOUND)?;
        match &item.detail {
            Detail::Revert(r) if r.edits == inverse && item.re.as_deref() == Some(target) => {}
            _ => return Err("its edits would be read differently"),
        }
        same_note(&item.note, stamp.note)
    })?;
    written.target = Some(target.to_string());
    Ok(written)
}

/// The new quote for a comment that was attached before the write and is not
/// now, when its quote was exactly the `replace` text of an applied edit.
fn repair(item: &Item, after: &Document, scanned: &Scan, applied: &[Edit], landed: &[(usize, usize)]) -> Option<Quote> {
    let Detail::Comment(comment) = &item.detail else {
        return None;
    };
    let was_attached = matches!(comment.anchor, Anchor::Attached { .. });
    let still_attached = match &after.item(&item.id)?.detail {
        Detail::Comment(c) => matches!(c.anchor, Anchor::Attached { .. }),
        _ => true,
    };
    if comment.retracted || !was_attached || still_attached {
        return None;
    }
    let quote = comment.quote.as_ref()?;
    let n = applied
        .iter()
        .position(|e| e.replace == quote.on && !e.with.is_empty())?;
    let (edit, (start, end)) = (&applied[n], landed[n]);
    let (body, cuts) = (&scanned.body, &scanned.cuts);

    // The new text alone, then inside the edit's own scope, then inside its lines.
    let rewritten = edit.scope.as_ref().map(|s| s.replacen(&edit.replace, &edit.with, 1));
    let lines = lines_around(body, start, end, false).map(str::to_string);
    [None, rewritten, lines]
        .into_iter()
        .enumerate()
        .filter(|(k, scope)| *k == 0 || scope.is_some())
        .map(|(_, scope)| scope)
        .find(|scope| anchor::resolve(body, cuts, &edit.with, scope.as_deref()) == Resolved::At(start, end))
        .map(|scope| Quote {
            scope,
            on: edit.with.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: &str = "2026-11-20T10:00:00Z";

    fn stamp(note: &'static str) -> Stamp<'static> {
        Stamp {
            by: "ana",
            at: AT,
            on_behalf_of: None,
            note,
        }
    }

    fn edit_file(body: &str, replace: &str, with: &str) -> String {
        format!("{body}\n<!--endmark edit e-1 by=ben at=2026-11-13T09:00:00Z\n```replace\n{replace}\n```\n```with\n{with}\n```\n-->\n")
    }

    #[test]
    fn back_and_forth_any_number_of_times() {
        let mut source = edit_file("Limit is 20.\n", "Limit is 10.", "Limit is 20.");
        let mut last = "e-1".to_string();
        for n in 0..6 {
            let id = format!("r-{n}");
            // Always name the original edit when it is in effect, to follow the chain.
            let name = if n % 2 == 0 { "e-1".to_string() } else { last.clone() };
            let done = revert(&source, &name, &id, &stamp("")).unwrap();
            assert_eq!(done.target.as_deref(), Some(last.as_str()), "step {n}");
            let doc = parse(&done.source);
            assert_eq!(doc.errors(), 0);
            let want = if n % 2 == 0 { "Limit is 10.\n" } else { "Limit is 20.\n" };
            assert!(doc.body.starts_with(want), "step {n}: {}", doc.body);
            source = done.source;
            last = id;
        }
    }

    #[test]
    fn refuses_a_link_that_is_not_in_effect() {
        let source = edit_file("Limit is 20.\n", "Limit is 10.", "Limit is 20.");
        let once = revert(&source, "e-1", "r-1", &stamp("")).unwrap().source;
        let err = revert(&once, "e-1", "r-2", &stamp("")).unwrap_err();
        assert_eq!(
            err,
            WriteError::NotInEffect {
                id: "e-1".into(),
                tip: "r-1".into()
            }
        );
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let source = edit_file("Limit is 20.\n", "Limit is 10.", "Limit is 20.").replace('\n', "\r\n");
        let done = revert(&source, "e-1", "r-1", &stamp("Why not.")).unwrap();
        assert!(!done.source.replace("\r\n", "").contains('\n'));
        assert!(done.source.starts_with("Limit is 10.\r\n"));
    }

    #[test]
    fn a_record_is_not_written_into_an_open_fence() {
        let source = "Limit is 20.\n\n<!--endmark edit e-1 by=ben at=2026-11-13T09:00:00Z\n```replace\nLimit is 10.\n```\n```with\nLimit is 20.\n```\n-->\n\n```\nunclosed\n";
        let err = revert(source, "e-1", "r-1", &stamp("")).unwrap_err();
        assert_eq!(err.code(), "unreadable");
    }

    #[test]
    fn a_note_that_reads_as_structure_is_refused() {
        let source = edit_file("Limit is 20.\n", "Limit is 10.", "Limit is 20.");
        let err = revert(&source, "e-1", "r-1", &stamp("```on\nLimit\n```")).unwrap_err();
        assert_eq!(err.code(), "unexpected-fence");
        // A fence that fits the verb would be read as an extra edit.
        let extra = "```replace\nLimit\n```\n```with\nCap\n```";
        assert_eq!(
            revert(&source, "e-1", "r-1", &stamp(extra)).unwrap_err().code(),
            "unreadable"
        );
    }

    #[test]
    fn text_with_backticks_and_terminators_survives() {
        let old = "run ```x``` then -->";
        let new = "run `y` then --!>";
        let source = format!("Do: {new}\n\n<!--endmark edit e-1 by=ben at=2026-11-13T09:00:00Z\n````replace\n{}\n````\n```with\n{}\n```\n-->\n", escape(old), escape(new));
        assert_eq!(parse(&source).errors(), 0);
        let done = revert(&source, "e-1", "r-1", &stamp("")).unwrap();
        assert_eq!(parse(&done.source).body, format!("Do: {old}\n\n\n"));
    }
}
