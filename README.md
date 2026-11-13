# Endmark

Endmark adds comments, grouped suggestions and a decision history to a Markdown file without changing how the file renders. The Markdown body stays untouched, and all review state lives in an append-only log of HTML-comment records at the end of the file.

**Status: early.** The reader and the linter work. The commands that write records, the renderer and the demo page do not exist yet. The format is defined in [RFC 0001](rfcs/0001-endmark.md), which is open for comment and may still change.

## What a file looks like

````markdown
Relay listens on port 8080 by default.

<!--endmark comment c-7f3k by=maria at=2026-11-10T14:05:00Z
```on
port 8080
```
Is this still right? Staging moved to 8443 last month.
-->
````

A plain Markdown renderer shows the first line and nothing else.

## The linter

`endmark lint` checks that every record in a file will be read, and that nothing in it will show up in a rendered page by mistake. It is meant for agents to run after every write. It never changes a file.

```sh
cargo build --release          # the binary is target/release/endmark

endmark lint notes.md          # exit 0: no errors; 1: errors; 2: could not read
endmark lint --strict notes.md # warnings fail too
endmark lint --format json notes.md
cat notes.md | endmark lint -  # check content before writing it
```

```text
notes.md:71: error[early-close] s-4q8d: content contains "-->" before the terminator
  hint: write "--\>" and "--!\>" inside records
checked 1 file: 1 error, 0 warnings
```

Two more commands read a file:

- `endmark status FILE` lists every comment and suggestion with its state.
- `endmark body FILE` prints the document without its records.

## Layout

| Path | Holds |
| --- | --- |
| `rfcs/` | The RFC: the format and the plan |
| `crates/endmark-core/` | The reader: scanner, records, state, anchoring, findings |
| `crates/endmark-cli/` | The `endmark` command |
| `conformance/` | Fixtures any implementation can be checked against |

## Planned

- the commands that write records: `comment`, `suggest`, `accept` and the rest
- a reference review renderer
- a demo page that shows a document beside its raw source and lets you watch the source change as you comment, suggest and accept

## Licence

MIT. See [LICENSE](LICENSE).
