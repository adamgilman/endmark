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

**The `endmark-core` crate**

- `parse` for reading, and the `write` module for writing.

**Conformance suite**

- 56 reader fixtures and 40 writer fixtures that an implementation in any language can be checked against.

**Not in this version**

- The review renderer and the demo page.
- The compatibility matrix across Markdown renderers.
