# Endmark

Endmark adds comments, grouped suggestions and a decision history to a Markdown file without changing how the file renders. The Markdown body stays untouched, and all review state lives in an append-only log of HTML-comment records at the end of the file.

**Status: design.** There is no implementation yet. The format and the plan for the reference implementation are in [RFC 0001](rfcs/0001-endmark.md), which is open for comment.

Planned:

- a normative spec and a language-neutral conformance suite
- `endmark lint`, a Rust linter that agents can run after every write
- the rest of the `endmark` command-line tool
- a reference review renderer
- a demo page that shows a document beside its raw source and lets you watch the source change as you comment, suggest and accept

## Licence

MIT. See [LICENSE](LICENSE).
