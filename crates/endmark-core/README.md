# endmark-core

The reader and writer for [Endmark](https://github.com/adamgilman/endmark), a collaboration layer for Markdown: comments, grouped suggestions and a decision history kept in the Markdown file itself, without changing how it renders.

This crate is the library behind the `endmark` command. It has no I/O: it takes the text of a file and returns values.

```rust
let source = "Relay listens on port 8080.\n";

// Write: every operation returns the new text of the file, or refuses.
let stamp = endmark_core::write::Stamp { by: "maria", at: "2026-11-10T14:05:00Z", on_behalf_of: None, note: "Still right?" };
let quote = endmark_core::Quote { scope: None, on: "port 8080".into() };
let written = endmark_core::write::comment(source, "c-7f3k", &stamp, Some(&quote), None).unwrap();

// Read: the body, every item with its state, and every finding.
let doc = endmark_core::parse(&written.source);
assert_eq!(doc.body, "Relay listens on port 8080.\n\n");
assert_eq!(doc.items[0].id, "c-7f3k");
assert!(doc.diagnostics.is_empty());
```

- `parse` never fails. Anything wrong with a file is reported in `Document::diagnostics`, with a stable code and a hint.
- The functions in `write` never produce a record that `parse` would ignore or report. A write that cannot be done is refused with a `WriteError`.
- The `commonmark` feature, on by default, cross-checks each record against CommonMark block structure. Turn it off to drop the `pulldown-cmark` dependency.

The format is defined in [RFC 0001](https://github.com/adamgilman/endmark/blob/main/rfcs/0001-endmark.md). The API may change before 0.1.0.

Licence: MIT.
