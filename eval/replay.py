#!/usr/bin/env python3
"""Replays the real history of a Markdown file to measure repair suggestions.

For pairs of revisions (A, then a later B) of one file in a git repository:

1. Comments are placed on phrases sampled from A, with the real writer
   (`endmark comment`), so each carries real guideposts.
2. The body is swapped for B, as if people who know nothing of Endmark had
   kept editing the document.
3. `endmark lint --suggest` is asked where each unanchored comment belongs.
4. Each answer is checked against where a diff of A and B says the quoted
   text went.

It prints a table and, with --out, one JSON line per comment for review.
See README.md in this directory for how to read the result.
"""

import argparse
import bisect
import difflib
import json
import random
import re
import subprocess
import sys
from collections import Counter

AT = "2026-01-01T00:00:00Z"


def git(repo, *args):
    done = subprocess.run(["git", "-C", repo, *args], capture_output=True)
    if done.returncode != 0:
        raise SystemExit(done.stderr.decode(errors="replace"))
    return done.stdout.decode("utf-8", errors="replace")


def revisions(repo, head, path):
    return git(repo, "log", "--format=%H", "--reverse", "--diff-filter=AM", head, "--", path).split()


def text_at(repo, rev, path):
    text = git(repo, "show", f"{rev}:{path}").replace("\r\n", "\n")
    return text if text.endswith("\n") else text + "\n"


def count(hay, needle):
    """Occurrences at every start position, as the format counts them."""
    found, at = 0, hay.find(needle)
    while at != -1:
        found += 1
        at = hay.find(needle, at + 1)
    return found


def resolve(body, on, scope):
    """The range a quote matches, or "detached" or "ambiguous"."""
    hay, base = body, 0
    if scope is not None:
        n = count(body, scope)
        if n != 1:
            return "detached" if n == 0 else "ambiguous"
        base = body.find(scope)
        hay = scope
    n = count(hay, on)
    if n != 1:
        return "detached" if n == 0 else "ambiguous"
    start = base + hay.find(on)
    return (start, start + len(on))


# ---- sampling ----

WORD = re.compile(r"\S+")


def sample(body, rng, wanted):
    """Quotes a reviewer might select: a few words, a sentence or line, or a
    couple of lines. Each is (start, end, scope or None) and matches once."""
    starts = [0]
    for line in body.splitlines(keepends=True):
        starts.append(starts[-1] + len(line))
    lines = [(starts[i], starts[i + 1] - 1) for i in range(len(starts) - 1)]
    lines = [(a, b) for a, b in lines if body[a:b].strip()]
    picked, tries = [], 0
    while len(picked) < wanted and tries < wanted * 20 and lines:
        tries += 1
        index = rng.randrange(len(lines))
        a, b = lines[index]
        kind = rng.choice(["words", "words", "words", "line", "lines"])
        if kind == "words":
            words = list(WORD.finditer(body, a, b))
            if not words:
                continue
            n = min(len(words), rng.choice([1, 2, 2, 3, 3, 4, 5, 6, 8]))
            first = rng.randrange(len(words) - n + 1)
            start, end = words[first].start(), words[first + n - 1].end()
        elif kind == "line":
            start, end = a, b
            while body[start] in " \t":
                start += 1
        else:
            last = min(index + rng.choice([1, 2]), len(lines) - 1)
            start, end = a, lines[last][1]
        on = body[start:end]
        if not on.strip() or "<!--" in on or "-->" in on or "\0" in on:
            continue
        scope = None
        if count(body, on) != 1:
            # What a page does with a selection that is not unique: scope it
            # to its line. If that is not enough, pick something else.
            scope = body[a:b]
            if resolve(body, on, scope) != (start, end):
                continue
        picked.append((start, end, scope, kind))
    return picked


# ---- where the text went, by diff ----

def segments(a, b):
    """Aligned pieces of a and b: (a0, a1, b0, b1, same).

    Lines that are identical are aligned first. What lies between them is
    aligned word by word with whitespace ignored, so that a re-wrapped
    paragraph is seen as the same words and not as new text."""
    la, lb = a.splitlines(keepends=True), b.splitlines(keepends=True)
    oa, ob = [0], [0]
    for line in la:
        oa.append(oa[-1] + len(line))
    for line in lb:
        ob.append(ob[-1] + len(line))
    # Merge each run of changes, and any blank lines caught between two
    # changes, into one region.
    regions = []
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, la, lb, autojunk=False).get_opcodes():
        same = tag == "equal"
        blank = same and not "".join(la[i1:i2]).strip()
        if regions and not regions[-1][4] and (not same or blank):
            regions[-1] = (regions[-1][0], i2, regions[-1][2], j2, False, regions[-1][5] or not same)
        else:
            regions.append((i1, i2, j1, j2, same and not blank, not same))
    out = []
    for i1, i2, j1, j2, same, touched in regions:
        a0, a1, b0, b1 = oa[i1], oa[i2], ob[j1], ob[j2]
        if same or not touched:
            out.append((a0, a1, b0, b1, True))
            continue
        wa = [(m.group(), m.start()) for m in WORD.finditer(a, a0, a1)]
        wb = [(m.group(), m.start()) for m in WORD.finditer(b, b0, b1)]
        if len(wa) * len(wb) > 30_000_000:
            out.append((a0, a1, b0, b1, False))
            continue
        inner = difflib.SequenceMatcher(None, [w[0] for w in wa], [w[0] for w in wb], autojunk=False)
        for tag, x1, x2, y1, y2 in inner.get_opcodes():
            if tag == "equal":
                # One piece per word: the space between words may differ.
                for k in range(x2 - x1):
                    (word, p), (_, q) = wa[x1 + k], wb[y1 + k]
                    out.append((p, p + len(word), q, q + len(word), True))
            else:
                p0 = wa[x1][1] if x1 < len(wa) else a1
                p1 = wa[x2 - 1][1] + len(wa[x2 - 1][0]) if x2 > x1 else p0
                q0 = wb[y1][1] if y1 < len(wb) else b1
                q1 = wb[y2 - 1][1] + len(wb[y2 - 1][0]) if y2 > y1 else q0
                out.append((p0, p1, q0, q1, False))
    return out


def went_to(segs, keys, start, end):
    """Where start..end of the old text is in the new: (lo, hi, changed)."""
    first = max(bisect.bisect_right(keys, start) - 2, 0)
    lo = hi = None
    changed = False
    for a0, a1, b0, b1, same in segs[first:]:
        if a0 >= end:
            break
        if a1 <= start:
            continue
        if not same:
            changed = True
            if b0 == b1:
                continue  # these words were deleted; the rest decides
        if lo is None:
            lo = b0 + (start - a0) if same and start > a0 else b0
        hi = b0 + (end - a0) if same and end <= a1 else b1
    return lo, hi, changed


def line_span(text, start, end):
    a = text.rfind("\n", 0, start) + 1
    b = text.find("\n", end)
    return a, (len(text) if b == -1 else b)


def paragraph_span(text, start, end):
    a = text.rfind("\n\n", 0, start)
    b = text.find("\n\n", end)
    return (0 if a == -1 else a + 2), (len(text) if b == -1 else b)


# ---- one pair of revisions ----


def endmark(binary, args, stdin):
    return subprocess.run([binary, *args], input=stdin.encode(), capture_output=True)


def follow(texts, steps, start, end):
    """Follows a range through every revision in turn, which is far more
    reliable than one diff across them all. Returns (lo, hi, changed, moved),
    with lo None once nothing is left of the text."""
    lo, hi, changed, moved = start, end, False, False
    for index, (segs, keys) in enumerate(steps):
        old, new = texts[index], texts[index + 1]
        a, b, touched = went_to(segs, keys, lo, hi)
        gone = a is None or new[a:b].strip() == ""
        if gone or touched:
            # A block that moved shows up in a diff as deleted here and
            # added there. If the range's own lines are in the next
            # revision exactly once, that is where it went.
            la, lb = line_span(old, lo, hi)
            lines = old[la:lb]
            if len(lines.split()) >= 4 and count(new, lines) == 1:
                at = new.find(lines) + (lo - la)
                a, b, gone, touched, moved = at, at + (hi - lo), False, False, True
        if gone:
            return None, None, True, moved
        lo, hi, changed = a, b, changed or touched
    return lo, hi, changed, moved


def replay(binary, texts, steps, rng, wanted):
    old, new = texts[0], texts[-1]
    quotes = sample(old, rng, wanted)
    source, placed = old, []
    for n, (start, end, scope, kind) in enumerate(quotes):
        args = ["comment", "--id", f"c-{n}", "--by", "eval", "--at", AT, "--note", "n", "--on", old[start:end]]
        if scope is not None:
            args += ["--in", scope]
        done = endmark(binary, args + ["-"], source)
        if done.returncode != 0:
            continue  # the writer refused it; nothing to follow
        source = done.stdout.decode()
        placed.append((f"c-{n}", start, end, scope, kind))
    if not placed:
        return []
    log = source[len(old):]
    edited = new + log

    report = json.loads(endmark(binary, ["lint", "--suggest", "--format", "json", "-"], edited).stdout)
    findings = {f.get("id"): f for f in report["findings"] if f["code"] in ("detached", "ambiguous")}

    rows = []
    for cid, start, end, scope, kind in placed:
        on = old[start:end]
        lo, hi, changed, moved = follow(texts, steps, start, end)
        gone = lo is None
        row = {"id": cid, "kind": kind, "on": on, "in": scope, "changed": changed, "gone": gone, "moved": moved}
        if not gone:
            row["truth"] = new[lo:hi]
        now = resolve(new, on, scope)
        finding = findings.get(cid)
        if finding is None:
            if not isinstance(now, tuple):
                raise SystemExit(f"{cid}: the reader calls it attached, the replay does not")
            if gone:
                row["outcome"] = "attached-elsewhere"  # the text was removed, and the same words are somewhere else
            else:
                row["outcome"] = "attached" if now[0] < max(hi, lo + 1) and lo < now[1] else "attached-elsewhere"
        else:
            repair = finding.get("repair")
            row["was"] = finding["code"]
            if repair is None:
                row["outcome"] = "silent-right" if gone else "missed"
            else:
                row.update(score=repair["score"], reason=repair["reason"], proposed=repair["on"], line=repair["line"])
                where = resolve(new, repair["on"], repair["in"])
                # For a person checking a disputed row: the paragraph the
                # quote was in, and the one the proposal is in.
                pa, pb = paragraph_span(old, start, end)
                row["old_paragraph"] = old[pa:pb][:600]
                if isinstance(where, tuple):
                    pa, pb = paragraph_span(new, *where)
                    row["new_paragraph"] = new[pa:pb][:600]
                if not isinstance(where, tuple):
                    row["outcome"] = "invalid"  # a proposal must match exactly once
                elif gone:
                    # Nothing is left of the text. Quoting the lines it sat
                    # in is a fair answer; anywhere else is wrong.
                    row["outcome"] = "wrong-gone"
                else:
                    # A proposal that quotes the surrounding lines is right
                    # anywhere in the paragraph; any other, on the lines.
                    span = paragraph_span if repair["reason"] == "surrounding-lines" else line_span
                    ta, tb = span(new, lo, hi)
                    row["outcome"] = "right" if where[0] < tb and ta < where[1] else "wrong"
        rows.append(row)
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("repo", nargs="?")
    parser.add_argument("paths", nargs="*")
    parser.add_argument("--summarise", metavar="ROWS", help="print the table for rows saved earlier with --out")
    parser.add_argument("--endmark", default="target/release/endmark")
    parser.add_argument("--gaps", default="1,5,20", help="how many revisions apart A and B are")
    parser.add_argument("--samples", type=int, default=20, help="comments per pair of revisions")
    parser.add_argument("--pairs", type=int, default=40, help="pairs of revisions per file and gap")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--head", default="HEAD", help="the commit whose history is replayed")
    parser.add_argument("--out", help="write one JSON line per comment here")
    args = parser.parse_args()

    if args.summarise:
        with open(args.summarise) as saved:
            return summarise([json.loads(line) for line in saved])
    if not args.repo or not args.paths:
        parser.error("give a repository and at least one file")
    rng = random.Random(args.seed)
    rows = []
    for path in args.paths:
        revs = revisions(args.repo, args.head, path)
        cache, diffs = {}, {}

        def step(i):
            if i not in diffs:
                segs = segments(text(i), text(i + 1))
                diffs[i] = (segs, [s[0] for s in segs])
            return diffs[i]

        def text(i):
            if i not in cache:
                cache[i] = text_at(args.repo, revs[i], path)
            return cache[i]

        for gap in [int(g) for g in args.gaps.split(",")]:
            starts = list(range(0, len(revs) - gap))
            rng.shuffle(starts)
            for i in sorted(starts[: args.pairs]):
                old, new = text(i), text(i + gap)
                if old == new or "\n<!--endmark" in old or "\n<!--endmark" in new:
                    continue
                texts = [text(j) for j in range(i, i + gap + 1)]
                steps = [step(j) for j in range(i, i + gap)]
                for row in replay(args.endmark, texts, steps, rng, args.samples):
                    row.update(file=path, gap=gap, old=revs[i][:10], new=revs[i + gap][:10])
                    rows.append(row)
        print(f"{path}: {len(revs)} revisions", file=sys.stderr)

    if args.out:
        with open(args.out, "w") as out:
            for row in rows:
                out.write(json.dumps(row, ensure_ascii=False) + "\n")
    summarise(rows)


def summarise(rows):
    total = len(rows)
    outcome = Counter(r["outcome"] for r in rows)
    print(f"comments placed: {total}")
    print(f"  still attached, in the right place: {outcome['attached']}")
    print(f"  still attached, but to other text:  {outcome['attached-elsewhere']}")
    lost = [r for r in rows if "was" in r]
    print(f"  lost their place:                   {len(lost)}")
    print()
    print("of those that lost their place:")
    print(f"  {'':24}{'right':>8}{'wrong':>8}{'text gone':>10}")
    for name, low, high in [("proposed, score 80-100", 80, 101), ("proposed, score 60-79", 60, 80), ("proposed, score 0-59", 0, 60)]:
        band = [r for r in lost if "score" in r and low <= r["score"] < high]
        right = sum(r["outcome"] == "right" for r in band)
        wrong = sum(r["outcome"] in ("wrong", "invalid") for r in band)
        gone = sum(r["outcome"] == "wrong-gone" for r in band)
        print(f"  {name:24}{right:>8}{wrong:>8}{gone:>10}")
    print(f"  nothing proposed, and the text was gone:   {outcome['silent-right']}")
    print(f"  nothing proposed, but the text was there:  {outcome['missed']}")
    print()
    print("proposals by how they were found (right / wrong or text gone):")
    for reason in ("exact-text", "similar-text", "surrounding-lines"):
        band = [r for r in lost if r.get("reason") == reason]
        right = sum(r["outcome"] == "right" for r in band)
        print(f"  {reason:20}{right:>6} /{len(band) - right:>5}")
    if outcome["invalid"]:
        print(f"\nINVALID proposals (do not match exactly once): {outcome['invalid']}")


if __name__ == "__main__":
    main()
