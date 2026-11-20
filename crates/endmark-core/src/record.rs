//! Record syntax (RFC 5): the header, the fences and the note.

use crate::diag::{Code, Diagnostic};
use crate::escape::unescape;
use crate::scan::{content_fence_close, content_fence_open, RawRecord};
use crate::{Edit, Hint, Quote, Verb};

/// A well-formed record with a known verb.
#[derive(Debug, Clone)]
pub(crate) struct Record {
    pub verb: Verb,
    pub id: String,
    pub line: usize,
    pub by: String,
    pub at: String,
    pub on_behalf_of: Option<String>,
    pub re: Option<String>,
    pub note: String,
    /// The quote of a `comment` or `anchor`.
    pub quote: Option<Quote>,
    /// Guideposts for the quote: where it was when written (RFC 6.1).
    pub hint: Hint,
    /// The edits of a `suggest`, `edit` or `revert`.
    pub edits: Vec<Edit>,
}

struct Header<'a> {
    verb: &'a str,
    id: &'a str,
    attrs: Vec<(&'a str, &'a str)>,
}

fn parse_header(header: &str) -> Result<Header<'_>, String> {
    let mut tokens = header.split_whitespace();
    let verb = tokens.next().ok_or("the header has no verb")?;
    if !verb.bytes().all(|c| c.is_ascii_lowercase() || c == b'-') {
        return Err(format!("\"{verb}\" is not a verb: use lower-case letters"));
    }
    let id = tokens.next().ok_or("the header has no id")?;
    if !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
        return Err(format!("\"{id}\" is not an id: use letters, digits, \"-\" and \"_\""));
    }
    let mut attrs = Vec::new();
    for token in tokens {
        let (key, value) = token
            .split_once('=')
            .ok_or_else(|| format!("\"{token}\" is not key=value"))?;
        let key_ok = key.bytes().next().is_some_and(|c| c.is_ascii_alphabetic())
            && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-');
        if !key_ok {
            return Err(format!("\"{key}\" is not an attribute name"));
        }
        if value.is_empty() {
            return Err(format!("attribute \"{key}\" has no value"));
        }
        attrs.push((key, value));
    }
    Ok(Header { verb, id, attrs })
}

/// RFC 3339 in UTC, ending in `Z`: `2026-11-13T09:30:00Z`, with optional fraction.
fn is_utc_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let part = b.get(r)?;
        if part.iter().all(u8::is_ascii_digit) {
            std::str::from_utf8(part).ok()?.parse().ok()
        } else {
            None
        }
    };
    let shape_ok = b.len() >= 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[b.len() - 1] == b'Z';
    if !shape_ok {
        return false;
    }
    let fraction = &b[19..b.len() - 1];
    let fraction_ok = fraction.is_empty()
        || (fraction[0] == b'.' && fraction.len() > 1 && fraction[1..].iter().all(u8::is_ascii_digit));
    let fields = (num(0..4), num(5..7), num(8..10), num(11..13), num(14..16), num(17..19));
    match fields {
        (Some(_), Some(mo), Some(d), Some(h), Some(mi), Some(s)) => {
            fraction_ok && (1..=12).contains(&mo) && (1..=31).contains(&d) && h < 24 && mi < 60 && s <= 60
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fence {
    On,
    In,
    Replace,
    With,
    Near,
}

impl Fence {
    fn from_info(info: &str) -> Option<Fence> {
        match info.trim_end() {
            "on" => Some(Fence::On),
            "in" => Some(Fence::In),
            "replace" => Some(Fence::Replace),
            "with" => Some(Fence::With),
            "near" => Some(Fence::Near),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Fence::On => "on",
            Fence::In => "in",
            Fence::Replace => "replace",
            Fence::With => "with",
            Fence::Near => "near",
        }
    }
}

/// Splits content into structural fences and the note. `Err` means a fence
/// was never closed.
fn parse_content(raw: &[String]) -> Result<(Vec<(Fence, String)>, String), ()> {
    let lines: Vec<String> = raw.iter().map(|l| unescape(l)).collect();
    let mut blocks = Vec::new();
    let mut note: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((len, info)) = content_fence_open(&lines[i]) {
            let open = i;
            i += 1;
            while i < lines.len() && !content_fence_close(&lines[i], len) {
                i += 1;
            }
            if i >= lines.len() {
                return Err(());
            }
            match Fence::from_info(info) {
                Some(kind) => blocks.push((kind, lines[open + 1..i].join("\n"))),
                // Any other fence belongs to the note, with its content.
                None => note.extend(lines[open..=i].iter().map(String::as_str)),
            }
        } else {
            note.push(&lines[i]);
        }
        i += 1;
    }
    Ok((blocks, note.join("\n").trim().to_string()))
}

type Structure = Result<(Option<Quote>, Option<String>, Vec<Edit>), (Code, String)>;

/// `[in] [on] [near]`, for `comment` and `anchor`.
fn quote_structure(blocks: Vec<(Fence, String)>, on_required: bool) -> Structure {
    let mut scope: Option<String> = None;
    let mut on: Option<String> = None;
    let mut near: Option<String> = None;
    for (kind, text) in blocks {
        match kind {
            Fence::In if scope.is_none() && on.is_none() => scope = Some(text),
            Fence::On if on.is_none() => on = Some(text),
            Fence::Near if near.is_none() => near = Some(text),
            Fence::In => {
                return Err((
                    Code::UnexpectedFence,
                    "an in fence must come once, before the on fence".into(),
                ))
            }
            Fence::On => return Err((Code::UnexpectedFence, "there is more than one on fence".into())),
            other => {
                return Err((
                    Code::UnexpectedFence,
                    format!("a {} fence does not belong in this record", other.name()),
                ))
            }
        }
    }
    if scope.as_deref() == Some("") {
        return Err((Code::MissingFence, "the in fence is empty".into()));
    }
    match on {
        Some(text) if text.is_empty() => Err((Code::MissingFence, "the on fence is empty".into())),
        Some(text) => Ok((
            Some(Quote { scope, on: text }),
            near.filter(|n| !n.is_empty()),
            Vec::new(),
        )),
        None if scope.is_some() => Err((Code::MissingFence, "an in fence needs an on fence after it".into())),
        None if on_required => Err((Code::MissingFence, "an anchor record needs an on fence".into())),
        None if near.is_some() => Err((
            Code::UnexpectedFence,
            "a near fence describes a quote, and there is no on fence".into(),
        )),
        None => Ok((None, None, Vec::new())),
    }
}

/// One or more `[in] replace with`, for `suggest`, `edit` and `revert`.
fn edit_structure(blocks: Vec<(Fence, String)>) -> Structure {
    let mut edits = Vec::new();
    let mut scope: Option<String> = None;
    let mut replace: Option<(Option<String>, String)> = None;
    let no_with = || {
        (
            Code::IncompleteEdit,
            "a replace fence is not followed by a with fence".to_string(),
        )
    };
    for (kind, text) in blocks {
        match kind {
            Fence::In => {
                if replace.is_some() {
                    return Err(no_with());
                }
                if scope.is_some() {
                    return Err((Code::UnexpectedFence, "two in fences in a row".into()));
                }
                if text.is_empty() {
                    return Err((Code::MissingFence, "the in fence is empty".into()));
                }
                scope = Some(text);
            }
            Fence::Replace => {
                if replace.is_some() {
                    return Err(no_with());
                }
                replace = Some((scope.take(), text));
            }
            Fence::With => match replace.take() {
                Some((scope, replace)) => edits.push(Edit {
                    scope,
                    replace,
                    with: text,
                }),
                None => {
                    return Err((
                        Code::UnexpectedFence,
                        "a with fence has no replace fence before it".into(),
                    ))
                }
            },
            Fence::On | Fence::Near => {
                return Err((
                    Code::UnexpectedFence,
                    format!("a {} fence does not belong in this record", kind.name()),
                ))
            }
        }
    }
    if replace.is_some() {
        return Err(no_with());
    }
    if scope.is_some() {
        return Err((Code::MissingFence, "an in fence needs a replace fence after it".into()));
    }
    if edits.is_empty() {
        return Err((Code::MissingFence, "the record has no replace and with fences".into()));
    }
    Ok((None, None, edits))
}

const KNOWN_ATTRIBUTES: [&str; 5] = ["by", "at", "for", "re", "line"];

/// Parses one raw record. Returns `None`, with a diagnostic, for anything a
/// reader must ignore: malformed records and unknown verbs.
pub(crate) fn parse_record(raw: &RawRecord, diags: &mut Vec<Diagnostic>) -> Option<Record> {
    let header = parse_header(&raw.header);
    let id = header.as_ref().ok().map(|h| h.id);

    if let Some(fault) = &raw.fault {
        diags.push(Diagnostic::new(fault.code, fault.line, id, fault.message.clone()));
        return None;
    }
    let header = match header {
        Ok(h) => h,
        Err(message) => {
            diags.push(Diagnostic::new(Code::BadHeader, raw.line, None, message));
            return None;
        }
    };
    let id = Some(header.id);

    let Some(verb) = Verb::from_name(header.verb) else {
        diags.push(Diagnostic::new(
            Code::UnknownVerb,
            raw.line,
            id,
            format!("unknown verb \"{}\"", header.verb),
        ));
        return None;
    };

    let mut by = None;
    let mut at = None;
    let mut on_behalf_of = None;
    let mut re = None;
    let mut line_hint: Option<String> = None;
    let mut seen: Vec<&str> = Vec::new();
    for (key, value) in &header.attrs {
        if seen.contains(key) {
            continue; // the first value wins
        }
        seen.push(*key);
        match *key {
            "by" => by = Some(value.to_string()),
            "at" => at = Some(value.to_string()),
            "for" => on_behalf_of = Some(value.to_string()),
            "re" => re = Some(value.to_string()),
            "line" => line_hint = Some(value.to_string()),
            _ => {}
        }
    }
    let (Some(by), Some(at)) = (by, at) else {
        diags.push(Diagnostic::new(
            Code::MissingAttribute,
            raw.line,
            id,
            "the record has no by= or no at=",
        ));
        return None;
    };

    if verb == Verb::Revert && re.is_none() {
        diags.push(Diagnostic::new(
            Code::MissingAttribute,
            raw.line,
            id,
            "a revert record has no re= to name the change it undoes",
        ));
        return None;
    }

    let (blocks, note) = match parse_content(&raw.content) {
        Ok(parsed) => parsed,
        Err(()) => {
            diags.push(Diagnostic::new(
                Code::UnterminatedFence,
                raw.line,
                id,
                "a fence in the record is never closed",
            ));
            return None;
        }
    };
    let structure = match verb {
        Verb::Comment => quote_structure(blocks, false),
        Verb::Anchor => quote_structure(blocks, true),
        Verb::Suggest | Verb::Edit | Verb::Revert => edit_structure(blocks),
        _ => match blocks.first() {
            None => Ok((None, None, Vec::new())),
            Some((kind, _)) => Err((
                Code::UnexpectedFence,
                format!("a {} fence does not belong in a {} record", kind.name(), verb.as_str()),
            )),
        },
    };
    let (quote, near, edits) = match structure {
        Ok(parts) => parts,
        Err((code, message)) => {
            diags.push(Diagnostic::new(code, raw.line, id, message));
            return None;
        }
    };

    // The record is valid from here on. What follows only warns.
    for key in &seen {
        if !KNOWN_ATTRIBUTES.contains(key) {
            diags.push(Diagnostic::new(
                Code::UnknownAttribute,
                raw.line,
                id,
                format!("unknown attribute \"{key}\""),
            ));
        }
    }
    // A line hint is a positive number. Anything else is ignored.
    let hinted = line_hint.as_deref().map(|v| v.parse::<usize>().ok().filter(|&n| n > 0));
    if hinted == Some(None) {
        diags.push(Diagnostic::new(
            Code::UnknownAttribute,
            raw.line,
            id,
            format!(
                "line=\"{}\" is not a line number",
                line_hint.as_deref().unwrap_or_default()
            ),
        ));
    }
    let hint = Hint {
        line: hinted.flatten().filter(|_| quote.is_some()),
        near,
    };
    if !is_utc_timestamp(&at) {
        diags.push(Diagnostic::new(
            Code::BadTimestamp,
            raw.line,
            id,
            format!("at=\"{at}\" is not an RFC 3339 UTC timestamp"),
        ));
    }

    Some(Record {
        verb,
        id: header.id.to_string(),
        line: raw.line,
        by,
        at,
        on_behalf_of,
        re,
        note,
        quote,
        hint,
        edits,
    })
}

#[cfg(test)]
mod tests {
    use super::is_utc_timestamp;

    #[test]
    fn timestamps() {
        for ok in [
            "2026-11-13T09:30:00Z",
            "2026-11-13T09:30:00.123Z",
            "2026-12-31T23:59:60Z",
        ] {
            assert!(is_utc_timestamp(ok), "{ok}");
        }
        for bad in [
            "x",
            "2026-11-13",
            "2026-11-13T09:30:00",
            "2026-11-13T09:30:00+01:00",
            "2026-13-13T09:30:00Z",
            "2026-11-13T24:30:00Z",
            "2026-11-13 09:30:00Z",
            "2026-11-13T09:30:00.Z",
            "2026-11-13T09:30:0xZ",
        ] {
            assert!(!is_utc_timestamp(bad), "{bad}");
        }
    }
}
