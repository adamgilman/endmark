# Conformance fixtures

Each fixture is a pair of files:

- `NAME.md` is an Endmark file.
- `NAME.json` is what a conforming reader reports for it.

The fixtures do not depend on any implementation. To check a reader written in any language, produce the JSON described below for each `.md` file and compare it with the `.json` file beside it. The reference implementation does this in `crates/endmark-cli/tests/conformance.rs`, using `endmark status --format json`.

The expectations were written by hand from RFC 0001. They were not captured from the reference implementation.

## The expectation file

```json
{
  "about": "Optional. What the fixture shows.",
  "body": "Optional. The exact body, with CRLF normalised to LF.",
  "items": [
    {"id": "c-1", "kind": "comment", "state": "live", "thread": "open", "anchor": "attached"}
  ],
  "diagnostics": [
    {"line": 3, "code": "detached", "id": "c-1"}
  ]
}
```

## How to compare

- `body`, when present, must equal the reader's body exactly.
- `items` and `diagnostics` must have the same length and order as the reader's. Items are in file order. Findings are in line order.
- Within an entry, every key the expectation gives must equal the reader's value. Keys the expectation leaves out are not checked.
- A `null` in the expectation means the reader's value is absent or null.
- Messages and hints are not part of the expectation, so their wording can change.

## Item keys

| Key | Values |
| --- | --- |
| `id` | The item's id |
| `kind` | `comment`, `suggestion`, `edit` or `revert` |
| `line` | 1-based line of the record that created the item |
| `by`, `for`, `note` | The author, the `for` attribute and the note |
| `re` | The `re` attribute: what a comment replies to, or the change a revert undoes |
| `state` | Comment: `live` or `retracted`. Suggestion: `open`, `accepted`, `rejected` or `retracted`. Edit or revert: `applied`. Reverting a change does not alter its `state`. |
| `thread` | `open` or `resolved`. On thread roots only. |
| `root` | The id of the thread's root. On replies only. |
| `anchor` | Comments: `attached`, `detached`, `ambiguous` or `unquoted` |
| `status` | Open suggestion: `applicable` or `stale`. Accepted suggestion, edit or revert: `located` or `unlocatable` while the change is in effect, and `reverted` while it is not. Absent for rejected and retracted suggestions. |
| `reverted_by` | The id of the revert that names this change. Absent or `null` when there is none. It stays set when that revert is itself reverted; `status` then says the change is in effect again. |
| `events` | The action records that took effect on the item, in file order, each with `verb`, `line`, `by`, `at`, `for` and `note`. A revert adds a `revert` event to the change it undoes. An array is compared as a whole, so give every key of every event. |
| `reason` | Why a suggestion is stale: `detached`, `ambiguous` or `overlap` |
| `edits` | The edits of a suggestion, an edit or a revert, each with `in`, `replace`, `with` and `range`. `range` is a byte range in the body: for an open suggestion, the text the edit would replace; for a change in effect, where its new text is now. It is `null` when that cannot be found. |

## Finding keys

| Key | Values |
| --- | --- |
| `line` | 1-based line the finding is reported on |
| `code` | A code from the table in RFC 0001, section 12.4 |
| `id` | The item or record concerned, or `null` when there is none |

## Revert fixtures

| Fixture | Shows |
| --- | --- |
| `worked-example-reverted` | RFC Appendix A.4: an accepted suggestion reverted, and a comment re-pointed |
| `worked-example-revert-reverted` | RFC Appendix A.5: that revert reverted, so the change is in effect again |
| `revert-edit` | A logged edit reverted, with the event it leaves on the edit |
| `revert-already-reverted` | A second revert of one change is ignored; a chain of three reverts |
| `revert-wrong-target` | A comment, an open suggestion and a rejected suggestion cannot be reverted |
| `revert-unknown-target` | `re` names nothing, or an item later in the file |
| `revert-malformed` | A revert with no `re`, and reverts with no edits |

Every finding code is raised by at least one fixture. A test in the reference implementation enforces this.

## Writer fixtures

Each directory under `write/` is one case for a writer:

- `input.md` is the file before the write.
- `op.json` is the operation.
- `expected.md` is the file after the write, byte for byte. It is absent when the write must be refused.

```json
{
  "about": "What the case shows.",
  "op": "revert",
  "id": "Optional. The item acted on or replied to.",
  "new_id": "Optional. The id for a new item.",
  "by": "maria",
  "at": "2026-11-12T09:15:00Z",
  "for": "Optional. The for attribute.",
  "note": "Optional.",
  "on": "Optional. The quote of a comment or an anchor.",
  "in": "Optional. The scope of that quote.",
  "edits": [{"in": "Optional.", "replace": "old text", "with": "new text"}],
  "fails": "Optional. Present when the write must be refused: the reason."
}
```

`op` is `comment`, `reply`, `suggest`, `edit`, `accept`, `reject`, `retract`, `resolve`, `reopen`, `anchor` or `revert`. A reply is a comment whose `re` is `id`. `edits` belongs to `suggest` and `edit`.

`id` is the id the person gave. For `revert` it may be any link of a chain, and for `resolve` and `reopen` any comment of the thread. `new_id` and `at` are fixed so that the output can be compared exactly; a writer normally chooses them itself.

A refused write must leave the file unchanged. The reasons are:

| Reason | The write is refused because |
| --- | --- |
| `not-found` | No item has the id |
| `not-a-change` | The item is a comment, or a suggestion that was never accepted |
| `not-in-effect` | The change is already undone |
| `text-changed` | The text the change wrote is no longer in the body exactly once |
| `unscoped-deletion` | The change deleted text with no `in` fence, so the place is not recorded |
| `id-in-use` | The id for the new record is taken |
| `quote-not-found` | A quote, or the text an edit replaces, is not in the body |
| `quote-ambiguous` | It is in the body more than once, and no scope narrows it |
| `overlap` | Two edits of one change touch the same text |
| `stale` | The suggestion to accept no longer applies to the body |
| `empty` | A comment has neither a quote nor a note |
| `no-change` | The thread is already in the state asked for |
| a finding code, such as `not-author`, `wrong-target` or `already-final` | A reader would ignore the record, for the reason that code gives in RFC 0001, section 12.4 |

Every `expected.md` passes `endmark lint --strict`. The reference implementation runs these cases in `crates/endmark-cli/tests/write.rs`.

`crlf.md` has CRLF line endings on purpose. `.gitattributes` stops Git from converting any fixture.
