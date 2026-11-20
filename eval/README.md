# Measuring repair suggestions on real documents

`endmark lint --suggest` proposes where a comment that lost its place now belongs (RFC 6.1, 12.4). This directory measures how often those proposals are right, by replaying the real revision histories of real documents.

It is a measurement, not a test: it needs the network and takes about a minute. The tests that run on every change are the scenarios in `crates/endmark-core/tests/repair_scenarios.rs`, whose kinds of edit were chosen from what this replay turned up.

## What it does

For pairs of revisions of one file, an older A and a newer B:

1. It places comments on phrases sampled from A with the real writer, `endmark comment`, so each carries real guideposts. The phrases are what a reviewer selects: a few words, a whole line, or a couple of lines. A phrase that is not unique is scoped to its line, as the demo page does.
2. It swaps the body for B. This is the situation the feature exists for: people kept editing the document with tools that know nothing of Endmark.
3. It asks `endmark lint --suggest` about every comment.
4. It works out where each quoted phrase went by diffing every intermediate revision in turn, and compares.

B is 1, 5 or 20 revisions after A.

## Running it

```sh
cargo build --release
eval/run.sh /some/scratch/directory
```

The documents are listed in `corpus.txt`, each pinned to a commit so the numbers can be reproduced: four chapters of the Rust book, four Kubernetes enhancement proposals, three Go design documents and four documents of the TC39 Temporal proposal. They were chosen for long histories of revision by several people. None of their text is copied into this repository.

`replay.py` runs one repository at a time and has options for the number of pairs, the samples per pair and the gaps. With `--out` it saves one JSON line per comment, including the paragraph the quote was in and the paragraph proposed, for checking by eye.

## Results on 2026-11-20

15 documents, 14,700 comments.

| What became of a comment | Count |
| --- | --- |
| Still attached, to the right text | 12,294 |
| Still attached, but to other text | 50 |
| Lost its place | 2,356 |

Of the 2,356 that lost their place:

| | Proposals | Agree with the diff | Disputed |
| --- | --- | --- | --- |
| Score 80 to 100: what `endmark repair` writes by default | 627 | 596 | 31 |
| Score 60 to 79: shown, written only on request | 507 | 404 | 103 |
| Nothing proposed, and the text was gone | 383 | | |
| Nothing proposed, though some of the text was still there | 839 | | |

Every proposal was a quote that matches exactly once.

**The 31 disputed proposals at 80 or more were each read by hand.** In 28 the proposal was right and the diff was wrong, mostly where a paragraph had been both reworded and re-wrapped. Three were wrong or doubtful:

- a paragraph was deleted and its near-twin, the same sentence about a different method, was proposed at 89;
- a one-word quote, `RangeError`, was matched in a different sentence with a similar ending, at 80;
- two lines of compiler output were matched to the output of a different example, at 80.

So about 624 of 627 high-confidence proposals were right, which is 99.5%. The disputed proposals in the 60 to 79 band have not been read by hand, so for that band only the lower bound is known: at least 79% right.

## What to take from it

- **Silence is the common failure, not a wrong answer.** For 839 comments nothing was proposed although some of the text survived. Nearly all of those are passages rewritten so heavily that only a person can say whether the comment still applies.
- **The known wrong answer is the twin paragraph.** When a paragraph is deleted and a near-copy of it remains, the copy looks like an edited version of the original. The scenario `sibling-paragraph-survives` pins the case where the two differ enough to stay under 80. Twins that differ by a single word score higher, and that is the first wrong proposal listed above.
- **Quotes themselves go wrong, rarely.** 50 comments, 0.4% of those still attached, were attached to other text: the quoted words were removed and the same words exist elsewhere. No guidepost is consulted for an attached comment, so nothing reports these. This is a property of quote anchoring, not of repair.

## Limits of the measurement

- The comments are sampled at random. Real reviewers comment on the passages most likely to change, so real comments probably lose their place more often than 16% of the time.
- "Right" is judged by a diff, which is itself a heuristic. That is why the disputed proposals had to be read.
- All four projects hard-wrap prose or use one line per sentence, and all are in English.
