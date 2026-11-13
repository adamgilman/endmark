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
| `kind` | `comment`, `suggestion` or `edit` |
| `line` | 1-based line of the record that created the item |
| `by`, `for`, `note` | The author, the `for` attribute and the note |
| `state` | Comment: `live` or `retracted`. Suggestion: `open`, `accepted`, `rejected` or `retracted`. Edit: `applied`. |
| `thread` | `open` or `resolved`. On thread roots only. |
| `root` | The id of the thread's root. On replies only. |
| `anchor` | Comments: `attached`, `detached`, `ambiguous` or `unquoted` |
| `status` | Open suggestion: `applicable` or `stale`. Accepted suggestion or edit: `located` or `unlocatable`. Absent for rejected and retracted suggestions. |
| `reason` | Why a suggestion is stale: `detached`, `ambiguous` or `overlap` |

## Finding keys

| Key | Values |
| --- | --- |
| `line` | 1-based line the finding is reported on |
| `code` | A code from the table in RFC 0001, section 12.4 |
| `id` | The item or record concerned, or `null` when there is none |

Every finding code is raised by at least one fixture. A test in the reference implementation enforces this.

`crlf.md` has CRLF line endings on purpose. `.gitattributes` stops Git from converting any fixture.
