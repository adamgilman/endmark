//! Cross-check against CommonMark (the `visible-record` finding).
//!
//! The scanner defines what a record is. A plain renderer follows CommonMark
//! instead: it hides a record only if it reads it as an HTML block of its
//! own. The two can disagree, for example when the body leaves a `<pre>`
//! block or a comment unclosed, or when a code fence inside a list item is
//! closed implicitly. A file where they disagree either shows a record to
//! readers of the rendered page or hides text that no Endmark reader sees.

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag};

use crate::diag::{Code, Diagnostic};
use crate::scan::{RawRecord, START};

pub(crate) fn check(text: &str, records: &[RawRecord], diags: &mut Vec<Diagnostic>) {
    if !text.contains(START) {
        return;
    }

    // HTML blocks that are direct children of the document.
    let mut blocks: Vec<Range<usize>> = Vec::new();
    let mut depth = 0usize;
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 && matches!(tag, Tag::HtmlBlock) {
                    blocks.push(range);
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }

    let id_of = |raw: &RawRecord| raw.header.split_whitespace().nth(1).map(str::to_string);

    for raw in records {
        if raw.fault.is_some() {
            continue; // already an error
        }
        // A record that follows an unterminated one sits inside that record's
        // comment as far as CommonMark is concerned. The unterminated record
        // is the error; do not report this one as well.
        let inside_faulted = blocks.iter().any(|b| {
            b.start < raw.start && raw.start < b.end && records.iter().any(|r| r.fault.is_some() && r.start == b.start)
        });
        if inside_faulted {
            continue;
        }
        let own_block = blocks
            .iter()
            .any(|b| b.start == raw.start && text[b.clone()].trim_end_matches('\n').len() == raw.end - raw.start);
        if !own_block {
            diags.push(Diagnostic::new(
                Code::VisibleRecord,
                raw.line,
                id_of(raw).as_deref(),
                "CommonMark does not read this record as an HTML comment block of its own, so a renderer may show it",
            ));
        }
    }

    for block in &blocks {
        let at_line_start = block.start == 0 || text.as_bytes()[block.start - 1] == b'\n';
        let looks_like_record = at_line_start && text[block.start..].starts_with(START);
        if looks_like_record && !records.iter().any(|r| r.start == block.start) {
            let line = text[..block.start].matches('\n').count() + 1;
            diags.push(Diagnostic::new(
                Code::VisibleRecord,
                line,
                None,
                "a renderer hides this as an HTML comment, but the scanner places it inside a code fence, so readers ignore it",
            ));
        }
    }
}
