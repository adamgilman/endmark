# Changelog

Notable changes to the format and to the reference implementation. Versions follow [Semantic Versioning](https://semver.org/). Before 1.0, a minor version may change the format.

## 0.1.0-alpha.1 (unreleased)

The first version. The format is defined in [RFC 0001](rfcs/0001-endmark.md) and may still change before 0.1.0, when the `<!--endmark` token is frozen.

**Format**

- Ten verbs: `comment`, `suggest`, `edit`, `revert`, `accept`, `reject`, `retract`, `resolve`, `reopen` and `anchor`.
- Records are HTML comments appended to the Markdown file. The body is never marked up.

**The `endmark` command**

- `lint`: checks files and reports 23 kinds of finding, each with a stable code and a hint. Text or JSON output.
- `status`, `log` and `body`: read a file.
- One command per verb to write records. A write that a reader would ignore is refused and leaves the file untouched.
- `revert` undoes an applied change on the record, and can be reverted in turn.
- `skill`: prints a SKILL.md document that tells an AI agent what Endmark is and how to use the command.

**The `endmark-core` crate**

- `parse` for reading, and the `write` module for writing.

**Conformance suite**

- 60 reader fixtures and 40 writer fixtures that an implementation in any language can be checked against.
- `spec/endmark.md`: the format alone, generated from the RFC.

**Guideposts and repair**

- Comments and anchors store where their quote was: a `line` attribute and a `near` fence with the surrounding lines. Readers never match on them.
- `endmark lint --suggest` says where a comment that lost its place probably belongs. `endmark repair` writes those proposals as `anchor` records.
- The search is tested against eighteen realistic edit scenarios, and measured by replaying the histories of 15 public documents (`eval/`): about 99.5% of the proposals `repair` writes by default were right.

**Renderer and demo page**

- `packages/render`: the reference review renderer, with Final, Review, Preview and History views.
- `demo`: a static page that shows a document beside its source and lets a visitor comment, suggest, accept, revert and edit.
- Both run `endmark-core` as WebAssembly and hold no format logic of their own.

**Compatibility**

- `compat/` checks nine Markdown engines on every pull request. Records stay hidden in every CommonMark engine tested with raw HTML on.

**Known gaps**

- Listed in section 12.9 of the RFC.
