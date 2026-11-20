//! Repair suggestions for comments that have lost their place (RFC 6.1).
//!
//! A reader never guesses where a comment belongs: a quote matches exactly
//! once or the comment is unanchored. This module is the optional other
//! half. For an unanchored comment it searches the body for the place the
//! comment most likely meant, using the guideposts stored with the quote,
//! and proposes a new quote. Nothing here changes state. A proposal only
//! takes effect when someone writes it as an `anchor` record.

use std::collections::{BTreeSet, HashMap};

use crate::anchor::{self, Resolved};
use crate::scan;
use crate::{parse, Anchor, Detail, Hint, Quote};

/// How a candidate was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The quoted text is in the body more than once, or its scope is gone;
    /// this is the occurrence that fits the guideposts.
    ExactText,
    /// The quoted text is gone. Similar text was found.
    SimilarText,
    /// The quoted words are gone, but the lines they were in were found.
    /// The proposal quotes those lines.
    SurroundingLines,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::ExactText => "exact-text",
            Reason::SimilarText => "similar-text",
            Reason::SurroundingLines => "surrounding-lines",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Reason::ExactText => "the same text, at the place that fits its guideposts",
            Reason::SimilarText => "similar text",
            Reason::SurroundingLines => "the lines the quote was in; the quoted words themselves are gone",
        }
    }
}

/// A proposed new quote for one comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The comment.
    pub id: String,
    /// A quote that matches the current body exactly once.
    pub quote: Quote,
    /// The 1-based line on which the proposed quote starts.
    pub line: usize,
    /// Confidence, 0 to 100. For similar text, how much of it is the same.
    pub score: u8,
    pub reason: Reason,
}

/// Below this, similar text is not proposed at all.
const FLOOR: f64 = 0.6;
/// Two places closer than this in score are a tie.
const MARGIN: f64 = 0.08;
/// Places to score in one fuzzy search before common words stop being added.
const MANY: usize = 2_000;
/// A cap on the work of one fuzzy search.
const BUDGET: usize = 40_000_000;

/// Proposes a repair for every live comment that is detached or ambiguous
/// and for which one place stands out. A comment with no clear candidate
/// gets none.
pub fn candidates(source: &str) -> Vec<Candidate> {
    let text = source.replace("\r\n", "\n");
    let doc = parse(&text);
    let scanned = scan::scan(&text);
    let (body, cuts) = (scanned.body.as_str(), scanned.cuts.as_slice());
    // The body is indexed once, however many comments need a place.
    let index = Index::new(body);
    let mut found = Vec::new();
    for item in &doc.items {
        let Detail::Comment(c) = &item.detail else { continue };
        if c.retracted || !matches!(c.anchor, Anchor::Detached | Anchor::Ambiguous) {
            continue;
        }
        let Some(quote) = &c.quote else { continue };
        if let Some((start, end, score, reason)) = locate(&index, quote, &c.hint) {
            let Some(scope) = unique_scope(body, cuts, start, end) else {
                continue;
            };
            let quote = Quote {
                scope,
                on: body[start..end].to_string(),
            };
            found.push(Candidate {
                id: item.id.clone(),
                quote,
                line: line_of(body, start),
                score: (score * 100.0).round().clamp(0.0, 100.0) as u8,
                reason,
            });
        }
    }
    found
}

/// The range to propose, with a score between 0 and 1.
fn locate(index: &Index, quote: &Quote, hint: &Hint) -> Option<(usize, usize, f64, Reason)> {
    let body = index.text;
    // First, the quoted words themselves, wherever lines now break.
    let found = index.occurrences(&quote.on);
    if !found.is_empty() {
        let around = sides(quote, hint);
        // A quote that needed a scope was not unique when it was written,
        // so its own words say nothing about which place was meant.
        let quoted = match quote.scope {
            Some(_) => 0,
            None => quote.on.split_whitespace().count(),
        };
        let ranked: Vec<(usize, f64)> = found
            .iter()
            .map(|&(start, end)| (start, fit(body, start, end, around, quoted)))
            .collect();
        if let Some((start, score)) = pick(body, &ranked, hint.line) {
            let end = found.iter().find(|r| r.0 == start)?.1;
            return Some((start, end, score, Reason::ExactText));
        }
        // The words are in the body, but nowhere that fits. Short quotes
        // such as "3" end up here once their own sentence has changed.
    }

    // The words are gone. Look for the lines they were in, or failing
    // that, for text like the quote.
    let context = hint.near.as_deref().or(quote.scope.as_deref());
    let target = context.unwrap_or(&quote.on);
    let (start, end, score) = fuzzy_find(index, target, hint.line)?;
    if context.is_none() {
        return Some((start, end, score, Reason::SimilarText));
    }
    // The lines were found. Look for something like the quote inside them.
    let block = &body[start..end];
    if let Some((s, e, inner)) = fuzzy_find(&Index::new(block), &quote.on, None) {
        let (mut s, mut e) = (start + s, start + e);
        if whole_lines(quote, hint) {
            // The quote was whole lines, so the proposal is too: a line
            // that gained a clause at its end is still that line.
            s = body[..s].rfind('\n').map_or(0, |p| p + 1);
            s += body[s..].len() - body[s..].trim_start_matches([' ', '\t']).len();
            e = body[e..].find('\n').map_or(body.len(), |p| e + p);
        }
        return Some((s, e, score.min(inner), Reason::SimilarText));
    }
    Some((start, end, score, Reason::SurroundingLines))
}

/// Whether the quote began at the start of a line and ended at the end of
/// one when it was written, as far as its stored lines say.
fn whole_lines(quote: &Quote, hint: &Hint) -> bool {
    let Some(near) = hint.near.as_deref() else {
        // No near fence is written when it would equal the quote.
        return quote.scope.is_none();
    };
    match occurrences(near, &quote.on).as_slice() {
        [at] => {
            let end = at + quote.on.len();
            let starts = near[..*at]
                .rsplit('\n')
                .next()
                .is_some_and(|lead| lead.trim().is_empty());
            starts && (end == near.len() || near[end..].starts_with('\n'))
        }
        _ => false,
    }
}

/// How many words either side of a quote are compared.
const SIDE: usize = 10;

/// How well a place where the quoted words are fits what was around the
/// quote when it was written, from 0 to 1. The words themselves count as
/// evidence too, so a long quote needs less from its surroundings than a
/// short one. With nothing known about the surroundings it is 1.
fn fit(body: &str, start: usize, end: usize, around: Option<(&str, &str)>, quoted: usize) -> f64 {
    let Some((before, after)) = around else { return 1.0 };
    let mut before: Vec<&str> = before.split_whitespace().rev().take(SIDE).collect();
    before.reverse();
    let after: Vec<&str> = after.split_whitespace().take(SIDE).collect();
    // Look twice as far in the body as was stored, so that words added
    // right beside the quote do not push its old neighbours out of view.
    let had: Vec<&str> = body[..start].split_whitespace().rev().take(2 * before.len()).collect();
    let has: Vec<&str> = body[end..].split_whitespace().take(2 * after.len()).collect();
    let known = (before.len() + after.len()) as f64;
    if known == 0.0 {
        return 1.0;
    }
    let same = (kept(&before, &had) + kept(&after, &has)) as f64;
    let own = 2.0 * quoted as f64;
    (own + same) / (own + known)
}

/// How many of the wanted words are among the given ones, each used once.
fn kept(wanted: &[&str], given: &[&str]) -> usize {
    let mut left: Vec<&str> = given.to_vec();
    wanted
        .iter()
        .filter(|word| match left.iter().position(|w| w == *word) {
            Some(at) => {
                left.swap_remove(at);
                true
            }
            None => false,
        })
        .count()
}

/// A text prepared for searching: its words and where each is, and the
/// text with every run of whitespace as one space.
struct Index<'a> {
    text: &'a str,
    words: Vec<Word<'a>>,
    /// For each distinct word, its positions in `words`.
    at: HashMap<&'a str, Vec<usize>>,
    flat: String,
    /// For each byte of `flat`, where it is in `text`.
    origin: Vec<usize>,
}

impl<'a> Index<'a> {
    fn new(text: &'a str) -> Self {
        let words = words(text);
        let mut at: HashMap<&str, Vec<usize>> = HashMap::new();
        for (position, word) in words.iter().enumerate() {
            at.entry(word.text).or_default().push(position);
        }
        let mut flat = String::with_capacity(text.len());
        let mut origin: Vec<usize> = Vec::with_capacity(text.len());
        let mut in_space = false;
        for (offset, c) in text.char_indices() {
            if c.is_whitespace() {
                if !in_space {
                    flat.push(' ');
                    origin.push(offset);
                }
                in_space = true;
            } else {
                flat.push(c);
                origin.extend(std::iter::repeat(offset).take(c.len_utf8()));
                in_space = false;
            }
        }
        Index {
            text,
            words,
            at,
            flat,
            origin,
        }
    }

    /// Every place the needle is in the text when any run of whitespace
    /// matches any other, so that re-wrapped lines still match. At most 64.
    fn occurrences(&self, needle: &str) -> Vec<(usize, usize)> {
        let wanted = needle.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut found = Vec::new();
        if wanted.is_empty() {
            return found;
        }
        let mut from = 0;
        while let Some(rel) = self.flat[from..].find(&wanted) {
            let start = from + rel;
            let last = self.origin[start + wanted.len() - 1]; // where the last character starts
            let end = last + self.text[last..].chars().next().map_or(1, char::len_utf8);
            found.push((self.origin[start], end));
            if found.len() == 64 {
                break;
            }
            from = start + self.flat[start..].chars().next().map_or(1, char::len_utf8);
        }
        found
    }
}

/// What was before and after the quote when it was written, from the
/// stored lines and the scope. `None` if they do not say.
fn sides<'a>(quote: &'a Quote, hint: &'a Hint) -> Option<(&'a str, &'a str)> {
    let once = |hay: &str, needle: &str| match occurrences(hay, needle).as_slice() {
        [at] => Some(*at),
        _ => None,
    };
    let split = |text: &'a str, at: usize| (&text[..at], &text[at + quote.on.len()..]);
    let (near, scope) = (hint.near.as_deref(), quote.scope.as_deref());
    // The scope places the quote; the stored lines, when they hold the
    // scope, say more about what was around it.
    if let (Some(near), Some(scope)) = (near, scope) {
        if let (Some(scope_at), Some(at)) = (once(near, scope), once(scope, &quote.on)) {
            return Some(split(near, scope_at + at));
        }
    }
    let context = scope.or(near)?;
    once(context, &quote.on).map(|at| split(context, at))
}

/// A place chosen only because it is nearest the line hint is a guess:
/// lines shift. It is never scored higher than this.
const GUESS: f64 = 0.7;

/// The best-scored place, if one stands out. A tie is settled by the line
/// hint, and stays a tie without one.
fn pick(body: &str, ranked: &[(usize, f64)], hint_line: Option<usize>) -> Option<(usize, f64)> {
    let best = ranked.iter().map(|r| r.1).fold(0.0, f64::max);
    if best < FLOOR {
        return None;
    }
    let close: Vec<(usize, f64)> = ranked.iter().copied().filter(|r| best - r.1 < MARGIN).collect();
    if let [only] = close.as_slice() {
        return Some(*only);
    }
    let line = hint_line?;
    let distance = |start: usize| line_of(body, start).abs_diff(line);
    let nearest = close.iter().map(|r| distance(r.0)).min()?;
    let mut at_nearest = close.iter().filter(|r| distance(r.0) == nearest);
    let chosen = at_nearest.next()?;
    at_nearest.next().is_none().then_some((chosen.0, chosen.1.min(GUESS)))
}

/// Byte offsets of every occurrence of `needle`, counted at every start position.
fn occurrences(hay: &str, needle: &str) -> Vec<usize> {
    let mut found = Vec::new();
    if needle.is_empty() {
        return found;
    }
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let start = from + rel;
        found.push(start);
        from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

fn line_of(body: &str, offset: usize) -> usize {
    body[..offset].matches('\n').count() + 1
}

/// The lines around a range: the unbroken run of non-blank lines it sits
/// in, at most three lines to either side. This is what a writer stores in
/// a `near` fence.
pub(crate) fn context_of(body: &str, start: usize, end: usize) -> &str {
    let mut from = body[..start].rfind('\n').map_or(0, |p| p + 1);
    let mut to = body[end..].find('\n').map_or(body.len(), |p| end + p);
    for _ in 0..3 {
        if from == 0 {
            break;
        }
        let previous = body[..from - 1].rfind('\n').map_or(0, |p| p + 1);
        if body[previous..from].trim().is_empty() {
            break;
        }
        from = previous;
    }
    for _ in 0..3 {
        if to >= body.len() {
            break;
        }
        let next = body[to + 1..].find('\n').map_or(body.len(), |p| to + 1 + p);
        if body[to + 1..next].trim().is_empty() {
            break;
        }
        to = next;
    }
    &body[from..to]
}

/// The smallest scope of whole lines that makes `start..end` match exactly
/// once: `Some(None)` if no scope is needed, `None` if none is enough.
fn unique_scope(body: &str, cuts: &[usize], start: usize, end: usize) -> Option<Option<String>> {
    let on = &body[start..end];
    let here = Resolved::At(start, end);
    if on.is_empty() {
        return None;
    }
    if anchor::resolve(body, cuts, on, None) == here {
        return Some(None);
    }
    let mut from = body[..start].rfind('\n').map_or(0, |p| p + 1);
    let mut to = body[end..].find('\n').map_or(body.len(), |p| end + p);
    loop {
        let scope = &body[from..to];
        if anchor::resolve(body, cuts, on, Some(scope)) == here {
            return Some(Some(scope.to_string()));
        }
        if from == 0 && to >= body.len() {
            return None;
        }
        if from > 0 {
            from = body[..from - 1].rfind('\n').map_or(0, |p| p + 1);
        }
        if to < body.len() {
            to = body[to + 1..].find('\n').map_or(body.len(), |p| to + 1 + p);
        }
    }
}

/// A word and where it is.
#[derive(Clone, Copy)]
struct Word<'a> {
    text: &'a str,
    start: usize,
}

fn words(text: &str) -> Vec<Word<'_>> {
    let mut found = Vec::new();
    let mut start = None;
    for (at, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(at),
            (true, Some(s)) => {
                found.push(Word {
                    text: &text[s..at],
                    start: s,
                });
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        found.push(Word {
            text: &text[s..],
            start: s,
        });
    }
    found
}

/// The last row of the edit-distance table: the distance from all of `a`
/// to each prefix of `b`.
fn distances(a: &[&str], b: &[&str]) -> Vec<usize> {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, wa) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, wb) in b.iter().enumerate() {
            let substitute = diagonal + usize::from(wa != wb);
            diagonal = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(diagonal + 1);
        }
    }
    row
}

/// Finds the run of words in the text most like `target`. Needs at least
/// three words to go on.
///
/// It does not try every position. A place worth scoring holds one of the
/// target's rarest words, so those words say where to look; each place
/// found is then nudged a few words either way to line it up.
fn fuzzy_find(index: &Index, target: &str, hint_line: Option<usize>) -> Option<(usize, usize, f64)> {
    let want: Vec<&str> = target.split_whitespace().collect();
    let have: Vec<&str> = index.words.iter().map(|w| w.text).collect();
    let n = want.len();
    if n < 3 || have.is_empty() {
        return None;
    }
    let slack = n / 3 + 1;
    let cost = n * (n + slack);

    // The target's words that are in the text, rarest first.
    let mut anchors: Vec<(usize, usize)> = Vec::new(); // (how often in the text, position in the target)
    for (j, word) in want.iter().enumerate() {
        if let Some(places) = index.at.get(word) {
            if !want[..j].contains(word) {
                anchors.push((places.len(), j));
            }
        }
    }
    anchors.sort_unstable();
    let mut starts: BTreeSet<usize> = BTreeSet::new();
    for &(often, j) in anchors.iter().take(4) {
        // Rare words are enough. A common one is only used when there is
        // nothing rarer, and never past the cap on work.
        let total = starts.len() + often;
        if total.saturating_mul(cost) > BUDGET || (!starts.is_empty() && total > MANY) {
            break;
        }
        starts.extend(index.at[want[j]].iter().map(|&at| at.saturating_sub(j)));
    }

    // The best run of words starting at `i`: (length, score).
    let score = |i: usize| -> (usize, f64) {
        let window = &have[i..(i + n + slack).min(have.len())];
        let row = distances(&want, window);
        (1..=window.len())
            .map(|len| (len, 1.0 - row[len] as f64 / n.max(len) as f64))
            .fold((0, f64::MIN), |best, next| if next.1 > best.1 { next } else { best })
    };
    let mut coarse: Vec<(usize, usize, f64)> = starts
        .iter()
        .map(|&i| {
            let (len, s) = score(i);
            (i, len, s)
        })
        .filter(|place| place.2 >= FLOOR - 0.2)
        .collect();
    coarse.sort_by(|a, b| b.2.total_cmp(&a.2));
    coarse.truncate(16);

    let mut places: Vec<(usize, usize, f64)> = Vec::new(); // first word, one past the last, score
    for (i, len, s) in coarse {
        let mut best = (i, len, s);
        for d in 1..=slack {
            for j in [i.checked_sub(d), Some(i + d).filter(|&j| j < have.len())]
                .into_iter()
                .flatten()
            {
                let (len, s) = score(j);
                if s > best.2 {
                    best = (j, len, s);
                }
            }
        }
        if best.2 >= FLOOR {
            places.push((best.0, best.0 + best.1, best.2));
        }
    }
    // Overlapping places are one place: keep the best of each cluster.
    places.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut distinct: Vec<(usize, usize, f64)> = Vec::new();
    for place in places {
        if !distinct.iter().any(|kept| place.0 < kept.1 && kept.0 < place.1) {
            distinct.push(place);
        }
    }
    let ranked: Vec<(usize, f64)> = distinct.iter().map(|p| (index.words[p.0].start, p.2)).collect();
    let (start, score) = pick(index.text, &ranked, hint_line)?;
    let place = distinct.iter().find(|p| index.words[p.0].start == start)?;
    let last = index.words[place.1 - 1];
    Some((start, last.start + last.text.len(), score))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(on: &str, extra: &str) -> String {
        format!("\n<!--endmark comment c-1 by=ana at=2026-11-13T09:00:00Z{extra}\n```on\n{on}\n```\nNote.\n-->\n")
    }

    #[test]
    fn nothing_is_proposed_for_an_attached_comment() {
        let source = format!("Retries happen 3 times.\n{}", comment("3 times", ""));
        assert!(candidates(&source).is_empty());
    }

    #[test]
    fn reworded_text_is_found() {
        let source = format!(
            "Intro.\n\nFailed deliveries are retried five times with a fixed delay.\n{}",
            comment("Failed deliveries are retried 3 times with a fixed delay.", "")
        );
        let found = candidates(&source);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].quote.on,
            "Failed deliveries are retried five times with a fixed delay."
        );
        assert_eq!(found[0].line, 3);
        assert_eq!(found[0].reason, Reason::SimilarText);
        assert!(found[0].score >= 80, "{}", found[0].score);
    }

    #[test]
    fn a_repeated_quote_needs_a_guidepost() {
        let body = "- TODO\n- other\n- TODO\n";
        assert!(candidates(&format!("{body}{}", comment("- TODO", ""))).is_empty());
        let found = candidates(&format!("{body}{}", comment("- TODO", " line=3")));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 3);
        assert_eq!(found[0].reason, Reason::ExactText);
        assert!(found[0].score <= 70, "a place chosen by line alone is a guess");
        // The proposal is a quote that matches exactly once.
        assert!(found[0].quote.scope.is_some());
    }

    #[test]
    fn a_shifted_line_does_not_beat_the_scope() {
        // The comment was on the second TODO, on line 3. A line added above
        // puts the first TODO on... line 2, and the second on line 4; the
        // hint is nearer neither, but the scope says which one it was.
        let body = "new line\n- TODO\n- another\n- TODO\n";
        let record = "\n<!--endmark comment c-1 by=ana at=2026-11-13T09:00:00Z line=2\n```in\n- other\n- TODO\n```\n```on\n- TODO\n```\n```near\n- TODO\n- other\n- TODO\n```\nNote.\n-->\n";
        let found = candidates(&format!("{body}{record}"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 4, "the hint pointed at line 2, the first TODO");
    }

    #[test]
    fn a_line_hint_between_two_matches_decides_nothing() {
        let body = "- TODO\n- other\n- TODO\n";
        assert!(candidates(&format!("{body}{}", comment("- TODO", " line=2"))).is_empty());
    }

    #[test]
    fn unrelated_text_is_not_proposed() {
        let source = format!(
            "Something else entirely lives here now.\n{}",
            comment("Failed deliveries are retried 3 times with a fixed delay.", "")
        );
        assert!(candidates(&source).is_empty());
    }

    #[test]
    fn word_distance_survives_an_insertion() {
        let a: Vec<&str> = "one two three four five".split_whitespace().collect();
        let b: Vec<&str> = "one two extra three four five".split_whitespace().collect();
        assert_eq!(distances(&a, &b)[b.len()], 1);
    }
}
