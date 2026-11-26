# Changelog

Notable changes to the format and to the reference implementation. Versions follow [Semantic Versioning](https://semver.org/). Before 1.0, a minor version may change the format.

## 0.1.1 (2026-11-26)

The format is unchanged.

- `endmark --licenses` prints Endmark's license and the licenses of the crates and JavaScript packages built into the binary, from `THIRD_PARTY_NOTICES.txt`. The release archives carry the same file, and each published crate now carries `LICENSE`.
- A code of conduct, issue and pull request templates, and Dependabot.

## 0.1.0 (2026-11-26)

The first release. The format is defined in [RFC 0001](rfcs/0001-endmark.md). The `<!--endmark` token is frozen from this version on. Before 1.0, a minor version may change the rest of the format.

**Format**

- Ten verbs: `comment`, `suggest`, `edit`, `revert`, `accept`, `reject`, `retract`, `resolve`, `reopen` and `anchor`.
- Records are HTML comments appended to the Markdown file. The body is never marked up.

**The `endmark` command**

- `lint`: checks files and reports 23 kinds of finding, each with a stable code and a hint. Text or JSON output.
- `status`, `log` and `body`: read a file.
- One command per verb to write records. A write that a reader would ignore is refused and leaves the file untouched.
- `revert` undoes an applied change on the record, and can be reverted in turn.
- `skill`: prints a SKILL.md document that tells an AI agent what Endmark is and how to use the command.
- `view`: serves the reference renderer for one file at `http://127.0.0.1:4173/`. The page shows the four views, writes each action to the file on disk through the same code as the commands, and follows the file when it changes. The page is built into the binary. It is an example host for one person: loopback only, with `--read-only` and `--port`. It follows the system's light or dark setting, and a switch in its header overrides that. See SECURITY.md for what it protects against.

- Rough edges found while writing the agent instructions:
  - `ENDMARK_FOR` stands in for `--for`. An author that starts with `agent:` and names no person gets a warning on standard error.
  - `suggest` and `edit` warn when they write a deletion that `revert` could not undo: one with no `--in`, or one that deletes all of its `--in` text. The refusal of such a revert now says which of the two it was.
  - A refusal with a linter's code, such as `not-author`, shows that code's hint.
  - `repair` exits 1 and writes nothing when an anchor is refused, and prints the refusal like every other command. Leaving a comment alone is still exit 0.
  - `body` prints one line ending after the document, not a blank line for each record, and rejects options it does not use. The body the format defines is unchanged.
  - `status` leaves the place of a retracted comment empty in its text output. JSON is unchanged.
  - A `no-change` refusal reached through a reply names the reply and its thread. `anchor` on a comment that was attached says `moved attached comment`.
  - `endmark help` is listed in the usage, and the usage no longer says `reply` needs `--note`.

**Releases**

- Prebuilt binaries for Linux, macOS and Windows, with checksums. The macOS binaries are signed with a Developer ID and notarized.
- `endmark` and `endmark-core` are published on crates.io.

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
- `view`: the page that `endmark view` serves, a thin host around the renderer.
- The renderer keeps a form that is being typed in when it is given a new source, and an action may return a promise.
- Both run `endmark-core` as WebAssembly and hold no format logic of their own.

**Compatibility**

- `compat/` checks nine Markdown engines on every pull request. Records stay hidden in every CommonMark engine tested with raw HTML on.

**Known gaps**

- Listed in section 12.9 of the RFC.
