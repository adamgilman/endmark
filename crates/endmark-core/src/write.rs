//! The write path (RFC 9). So far it holds one operation: `revert`.
//!
//! A write takes the source of a file and returns the new source, or fails
//! and changes nothing. Putting the result on disk is the caller's job.

use std::fmt;

use crate::anchor::{self, Resolved};
use crate::escape::escape;
use crate::scan::{self, START};
use crate::{parse, Anchor, Applicability, Detail, Document, Edit, Item, Quote, StaleReason, SuggestionState};

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

/// The result of a revert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reverted {
    /// The whole file after the write.
    pub source: String,
    /// The id of the new revert record.
    pub revert_id: String,
    /// The change the record names in `re`: the tip of the chain of the id
    /// that was given. It differs from the given id when the chain was followed.
    pub target: String,
    /// Comments re-pointed by an `anchor` record in the same write.
    pub reanchored: Vec<String>,
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
    AfterLog { target: String },
    /// The id for the new record is already taken.
    IdInUse { id: String },
    /// A value cannot be written in a header.
    BadValue { what: &'static str, value: String },
    /// The new records would not be read back as written.
    Unreadable { why: &'static str },
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
            WriteError::AfterLog { .. } => "after-log",
            WriteError::IdInUse { .. } => "id-in-use",
            WriteError::BadValue { .. } => "bad-value",
            WriteError::Unreadable { .. } => "unreadable",
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
            WriteError::AfterLog { .. } => "move the body text above the first record, then try again".into(),
            WriteError::IdInUse { .. } => "choose another id, or leave it out to get a random one".into(),
            WriteError::BadValue { .. } => "header values cannot be empty or contain spaces or \"-->\"".into(),
            WriteError::Unreadable { .. } => "run endmark lint on the file and fix what it reports first".into(),
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
            WriteError::AfterLog { target } => {
                write!(
                    f,
                    "the text written by \"{target}\" is after the first record in the file"
                )
            }
            WriteError::IdInUse { id } => write!(f, "id \"{id}\" is already used in the file"),
            WriteError::BadValue { what, value } => {
                write!(f, "{what} \"{value}\" cannot be written in a record header")
            }
            WriteError::Unreadable { why } => write!(f, "the new record would not be read back as written: {why}"),
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

fn revert_record(id: &str, target: &str, stamp: &Stamp, edits: &[Edit]) -> String {
    let mut out = header("revert", id, Some(target), stamp);
    out.push('\n');
    let note = stamp.note.trim();
    if !note.is_empty() {
        out.push_str(&escape(note));
        out.push('\n');
    }
    for edit in edits {
        if !note.is_empty() || !std::ptr::eq(edit, &edits[0]) {
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

fn anchor_record(id: &str, stamp: &Stamp, quote: &Quote) -> String {
    let mut out = header("anchor", id, None, stamp);
    out.push('\n');
    if let Some(scope) = &quote.scope {
        fence(&mut out, "in", scope);
    }
    fence(&mut out, "on", &quote.on);
    out.push_str("-->\n");
    out
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

/// Reverts a change (RFC 9.3 and 9.4).
///
/// `id` may be any link of a chain: the change itself or any revert in its
/// chain. If that link is in effect, the tip of the chain is reverted, which
/// undoes it. If it is not in effect, the write is refused.
///
/// `new_id` is the id for the new revert record.
pub fn revert(source: &str, id: &str, new_id: &str, stamp: &Stamp) -> Result<Reverted, WriteError> {
    check_value("id", new_id, true)?;
    check_value("author", stamp.by, false)?;
    check_value("time", stamp.at, false)?;
    if let Some(person) = stamp.on_behalf_of {
        check_value("for", person, false)?;
    }

    let crlf = source.contains("\r\n");
    let text = source.replace("\r\n", "\n");
    let before = parse(&text);
    let scanned = scan::scan(&text);
    let (body, cuts) = (&scanned.body, &scanned.cuts);

    if before.item(new_id).is_some() {
        return Err(WriteError::IdInUse { id: new_id.to_string() });
    }
    let given = before
        .item(id)
        .ok_or_else(|| WriteError::NotFound { id: id.to_string() })?;
    let (_, _, in_effect) = change(given).map_err(|what| WriteError::NotAChange {
        id: id.to_string(),
        what,
    })?;

    // Follow the chain to its tip: the one link nothing has reverted.
    let mut tip = given;
    while let Ok((_, Some(next), _)) = change(tip) {
        match before.item(next) {
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
    let target = tip.id.clone();
    let (edits, _, _) = change(tip).expect("a link of a chain is a change");

    // The inverse of every edit, resolved against the current body.
    let mut inverse = Vec::with_capacity(edits.len());
    for edit in edits {
        inverse.push(invert(edit).ok_or_else(|| WriteError::UnscopedDeletion { target: target.clone() })?);
    }
    // Offsets in the body equal offsets in the file up to the first record.
    let limit = cuts.first().copied().unwrap_or(body.len());
    let mut ranges = Vec::with_capacity(inverse.len());
    for inv in &mut inverse {
        let changed = |why| WriteError::TextChanged {
            target: target.clone(),
            text: inv.replace.clone(),
            why,
        };
        let (start, end) = match anchor::edit_range(body, cuts, inv) {
            Resolved::At(start, end) => (start, end),
            Resolved::Detached => return Err(changed("is no longer in the document")),
            Resolved::Ambiguous => return Err(changed("now appears more than once")),
        };
        if end > limit {
            return Err(WriteError::AfterLog { target });
        }
        // Give a deletion a scope, so that this revert can be reverted too.
        if inv.with.is_empty() && inv.scope.is_none() {
            if let Some(scope) = lines_around(body, start, end, true) {
                if anchor::resolve(body, cuts, &inv.replace, Some(scope)) == Resolved::At(start, end) {
                    inv.scope = Some(scope.to_string());
                }
            }
        }
        ranges.push((start, end));
    }
    if anchor::applicability(body, cuts, &inverse) == Applicability::Stale(StaleReason::Overlap) {
        return Err(WriteError::TextChanged {
            target,
            text: inverse[0].replace.clone(),
            why: "overlaps another edit of the same change",
        });
    }

    // Apply all edits together, last first so earlier offsets stay valid.
    let mut order: Vec<usize> = (0..inverse.len()).collect();
    order.sort_by_key(|&n| ranges[n]);
    let mut out = text.clone();
    for &n in order.iter().rev() {
        out.replace_range(ranges[n].0..ranges[n].1, &inverse[n].with);
    }
    // Where each edit landed in the new body.
    let mut landed = vec![(0usize, 0usize); inverse.len()];
    let mut shift = 0isize;
    for &n in &order {
        let start = ranges[n].0.wrapping_add_signed(shift);
        landed[n] = (start, start + inverse[n].with.len());
        shift += inverse[n].with.len() as isize - (ranges[n].1 - ranges[n].0) as isize;
    }

    append(&mut out, &revert_record(new_id, &target, stamp, &inverse));

    // RFC 9.4: re-point the comments this write detached, where it is certain.
    let middle = parse(&out);
    let rescanned = scan::scan(&out);
    let mut reanchored = Vec::new();
    for item in &before.items {
        if let Some(quote) = repair(item, &middle, &rescanned, &inverse, &landed) {
            append(&mut out, &anchor_record(&item.id, stamp, &quote));
            reanchored.push(item.id.clone());
        }
    }

    verify(&before, &out, new_id, &target, stamp, &inverse, &reanchored)?;

    Ok(Reverted {
        source: if crlf { out.replace('\n', "\r\n") } else { out },
        revert_id: new_id.to_string(),
        target,
        reanchored,
    })
}

/// The new quote for a comment that was attached before the write and is not
/// now, when its quote was exactly the `replace` text of an applied edit.
fn repair(
    item: &Item,
    after: &Document,
    scanned: &scan::Scan,
    applied: &[Edit],
    landed: &[(usize, usize)],
) -> Option<Quote> {
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

/// Reads the result back. A write that a reader would not understand as
/// intended is refused. This also covers RFC 9.1: a body that ends inside an
/// open code fence would swallow the new record.
fn verify(
    before: &Document,
    out: &str,
    new_id: &str,
    target: &str,
    stamp: &Stamp,
    edits: &[Edit],
    reanchored: &[String],
) -> Result<(), WriteError> {
    let after = parse(out);
    let unreadable = |why| Err(WriteError::Unreadable { why });
    let Some(item) = after.item(new_id) else {
        return unreadable("a reader would not find it; the file may end inside an open code fence");
    };
    match &item.detail {
        Detail::Revert(r) if r.edits == edits && item.re.as_deref() == Some(target) => {}
        _ => return unreadable("its edits would be read differently"),
    }
    if item.note != stamp.note.trim() {
        return unreadable("the note contains lines that a reader takes for record syntax");
    }
    if after.errors() > before.errors() {
        return unreadable("it would add errors to the file");
    }
    let attached = |id: &String| {
        matches!(
            after.item(id).map(|i| &i.detail),
            Some(Detail::Comment(c)) if matches!(c.anchor, Anchor::Attached { .. })
        )
    };
    if !reanchored.iter().all(attached) {
        return unreadable("a re-pointed comment would not attach");
    }
    Ok(())
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
            assert_eq!(done.target, last, "step {n}");
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
        assert_eq!(err.code(), "unreadable");
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
