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

`crlf.md` has CRLF line endings on purpose. `.gitattributes` stops Git from converting any fixture.
