//! The line scanner (RFC 4.1): splits a file into its body and its raw records.

use crate::diag::Code;

/// A record starts on a line that begins with this, at column 0.
pub const START: &str = "<!--endmark ";
const TERMINATOR: &str = "-->";
const BANG_TERMINATOR: &str = "--!>";

/// Why the scanner considers a record malformed.
#[derive(Debug, Clone)]
pub(crate) struct Fault {
    pub code: Code,
    pub line: usize,
    pub message: String,
}

/// A record as found in the file, before its header and content are parsed.
#[derive(Debug, Clone)]
pub(crate) struct RawRecord {
    /// 1-based line of the header.
    pub line: usize,
    /// 1-based last line that belongs to the record.
    pub end_line: usize,
    /// Byte range in the LF-normalised text, without the final line ending.
    pub start: usize,
    pub end: usize,
    /// The header line after the start token, and before `-->` if single-line.
    pub header: String,
    /// Content lines, still escaped.
    pub content: Vec<String>,
    pub fault: Option<Fault>,
}

pub(crate) struct Scan {
    /// The file with every record line removed.
    pub body: String,
    /// Body offsets from which a record was removed. A match may not span one.
    pub cuts: Vec<usize>,
    pub records: Vec<RawRecord>,
    /// 1-based line of the first non-blank body line after a record, if any.
    pub body_after_log: Option<usize>,
}

/// Does this body line open a code fence? Returns the fence character and run length.
pub(crate) fn body_fence_open(line: &str) -> Option<(u8, usize)> {
    let b = line.as_bytes();
    let indent = b.iter().take_while(|&&c| c == b' ').count();
    if indent > 3 {
        return None;
    }
    let ch = *b.get(indent)?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let run = b[indent..].iter().take_while(|&&c| c == ch).count();
    if run < 3 {
        return None;
    }
    // CommonMark: the info string of a backtick fence cannot contain a backtick.
    if ch == b'`' && b[indent + run..].contains(&b'`') {
        return None;
    }
    Some((ch, run))
}

fn body_fence_close(line: &str, ch: u8, len: usize) -> bool {
    let b = line.as_bytes();
    let indent = b.iter().take_while(|&&c| c == b' ').count();
    if indent > 3 {
        return false;
    }
    let run = b[indent..].iter().take_while(|&&c| c == ch).count();
    run >= len && b[indent + run..].iter().all(|&c| c == b' ' || c == b'\t')
}

/// Does this record-content line open a fence? Only backtick fences at
/// column 0 count. Returns the run length and the info string.
pub(crate) fn content_fence_open(line: &str) -> Option<(usize, &str)> {
    let run = line.bytes().take_while(|&c| c == b'`').count();
    if run < 3 {
        return None;
    }
    let info = &line[run..];
    if info.contains('`') {
        return None;
    }
    Some((run, info))
}

/// A content fence is closed by a line of at least as many backticks and nothing else.
pub(crate) fn content_fence_close(line: &str, len: usize) -> bool {
    line.len() >= len && line.bytes().all(|c| c == b'`')
}

/// Scans LF-normalised text.
pub(crate) fn scan(text: &str) -> Scan {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut offsets = Vec::with_capacity(lines.len());
    let mut at = 0;
    for l in &lines {
        offsets.push(at);
        at += l.len() + 1;
    }

    let mut body: Vec<&str> = Vec::new();
    let mut next_body_start = 0usize;
    let mut cuts = Vec::new();
    let mut records: Vec<RawRecord> = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut body_after_log = None;

    let mut i = 0;
    while i < lines.len() {
        let ln = lines[i];
        if fence.is_none() && ln.starts_with(START) {
            let rec = scan_record(&lines, &offsets, i);
            i = rec.end_line;
            if cuts.last() != Some(&next_body_start) {
                cuts.push(next_body_start);
            }
            records.push(rec);
            continue;
        }
        fence = match fence {
            Some((ch, len)) if body_fence_close(ln, ch, len) => None,
            Some(open) => Some(open),
            None => body_fence_open(ln),
        };
        if !records.is_empty() && body_after_log.is_none() && !ln.trim().is_empty() {
            body_after_log = Some(i + 1);
        }
        body.push(ln);
        next_body_start += ln.len() + 1;
        i += 1;
    }

    Scan {
        body: body.join("\n"),
        cuts,
        records,
        body_after_log,
    }
}

fn scan_record(lines: &[&str], offsets: &[usize], i: usize) -> RawRecord {
    let rest = &lines[i][START.len()..];
    let make = |header: &str, content: Vec<String>, last: usize, fault: Option<Fault>| RawRecord {
        line: i + 1,
        end_line: last + 1,
        start: offsets[i],
        end: offsets[last] + lines[last].len(),
        header: header.trim().to_string(),
        content,
        fault,
    };
    let fault = |code: Code, line: usize, message: String| Some(Fault { code, line, message });

    // Single-line form: the header line itself contains the terminator.
    if let Some(p) = rest.find(TERMINATOR) {
        let header = &rest[..p];
        let trailing = &rest[p + TERMINATOR.len()..];
        let f = if header.contains(BANG_TERMINATOR) {
            fault(
                Code::EarlyClose,
                i + 1,
                "header contains \"--!>\", which closes the comment in browsers".into(),
            )
        } else if !trailing.trim().is_empty() {
            fault(
                Code::TrailingText,
                i + 1,
                "text follows \"-->\" on a single-line record".into(),
            )
        } else {
            None
        };
        return make(header, Vec::new(), i, f);
    }

    let mut pending = if rest.contains(BANG_TERMINATOR) {
        fault(
            Code::EarlyClose,
            i + 1,
            "header contains \"--!>\", which closes the comment in browsers".into(),
        )
    } else {
        None
    };
    let mut content: Vec<String> = Vec::new();
    let mut open_fence: Option<usize> = None;
    let mut j = i + 1;
    loop {
        if j >= lines.len() {
            let f = pending.take().or_else(|| {
                fault(
                    Code::UnterminatedRecord,
                    i + 1,
                    "record is still open at end of file".into(),
                )
            });
            return make(rest, content, lines.len() - 1, f);
        }
        let l = lines[j];
        if l == TERMINATOR {
            if open_fence.is_none() {
                return make(rest, content, j, pending);
            }
            let f = pending.take().or_else(|| {
                fault(
                    Code::UnterminatedFence,
                    j + 1,
                    "the terminator appears while a fence is still open".into(),
                )
            });
            return make(rest, content, skip_to_terminator(lines, j), f);
        }
        // A record start outside a fence means this record was never terminated.
        // This is checked first, because a single-line record contains "-->".
        if open_fence.is_none() && l.starts_with(START) {
            let f = pending.take().or_else(|| {
                fault(
                    Code::UnterminatedRecord,
                    i + 1,
                    format!("record is still open where the next record starts, on line {}", j + 1),
                )
            });
            return make(rest, content, j - 1, f);
        }
        if l.contains(TERMINATOR) || l.contains(BANG_TERMINATOR) {
            let which = if l.contains(TERMINATOR) {
                TERMINATOR
            } else {
                BANG_TERMINATOR
            };
            let f = pending.take().or_else(|| {
                fault(
                    Code::EarlyClose,
                    j + 1,
                    format!("content contains \"{which}\" before the terminator"),
                )
            });
            return make(rest, content, skip_to_terminator(lines, j), f);
        }
        open_fence = match open_fence {
            Some(n) if content_fence_close(l, n) => None,
            Some(n) => Some(n),
            None => content_fence_open(l).map(|(n, _)| n),
        };
        content.push(l.to_string());
        j += 1;
    }
}

/// After an early close on line `j`, the rest of the record up to its real
/// terminator belongs to the malformed record, not to the body. The skip is
/// bounded: it stops if another record starts, or the file ends, before a
/// terminator line is found.
fn skip_to_terminator(lines: &[&str], j: usize) -> usize {
    for (k, l) in lines.iter().enumerate().skip(j + 1) {
        if *l == TERMINATOR {
            return k;
        }
        if l.starts_with(START) {
            return j;
        }
    }
    j
}
