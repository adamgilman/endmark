# Endmark

Endmark adds comments, grouped suggestions and a decision history to a Markdown file without changing how the file renders. The Markdown body stays untouched, and all review state lives in an append-only log of HTML-comment records at the end of the file.

**Status: 0.1.0, the first release.** The format is defined in [RFC 0001](rfcs/0001-endmark.md). The `<!--endmark` token is frozen. Before 1.0, a minor version may change the rest of the format, and [CHANGELOG.md](CHANGELOG.md) will say how.

## Install

Download the archive for your platform from the [latest release](https://github.com/adamgilman/endmark/releases/latest), unpack it, and put `endmark` somewhere on your `PATH`. There are builds for Linux (x86-64), macOS (Apple silicon and Intel) and Windows (x86-64). `SHA256SUMS` in the release lists their checksums.

The macOS binaries are not signed, so macOS refuses to open a downloaded one at first. Clear the quarantine flag once:

```sh
xattr -d com.apple.quarantine ./endmark
```

Or install from [crates.io](https://crates.io/crates/endmark) with a Rust toolchain, 1.80 or later:

```sh
cargo install endmark
```

Or build from a checkout with `cargo build --release`; the binary is `target/release/endmark`. It is one static binary with no configuration. It connects to nothing. Only `endmark view` listens, and only on your own computer.

## Try it

Pick any Markdown file and open it for review in your browser:

```sh
endmark view --by yourname notes.md
```

```text
Serving notes.md at http://127.0.0.1:4173/ (Ctrl-C to stop)
The page writes to notes.md as yourname.
```

Open that address. The page shows the document with its comments and suggestions drawn in place, in four views: Final, Review, Preview and History. Select text to comment on it or to suggest a replacement. Reply, resolve, accept, reject, and revert from History. Each action is written to `notes.md` as a record, exactly as the matching command below would write it. For a file with something in it already, try a copy of [conformance/worked-example.md](conformance/worked-example.md).

The page follows the file. Change it in an editor, or let an agent run `endmark comment` in another terminal, and the page shows the change within a second or two. A reply you are half way through typing stays as it is.

The page follows your system's light or dark setting. The Auto, Light and Dark buttons in its header override that, and the browser remembers the choice.

- `--read-only` shows the file and writes nothing. It needs no `--by`.
- `--port N` uses another port. `--port 0` picks a free one.
- `--for PERSON` names the person an agent acts for, as on the commands that write.

`endmark view` is an example of a host for the [reference renderer](packages/render/README.md), included so that one binary is a complete tool. It serves one file to one person, on `127.0.0.1` only, with no accounts and no TLS. It draws diagrams but not images, and raw HTML in the document is shown as text. The page is inside the binary, so it needs no network and no other program. For a review site that several people share, host the renderer yourself. [SECURITY.md](SECURITY.md) says what the server protects against and what it does not.

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

## Undoing a change

A change that was applied to the body can be reverted: an accepted suggestion, a logged edit, or an earlier revert. Reverting does not erase anything. The body goes back to its earlier text, the original records stay in the log, and a `revert` record is appended that says who undid the change, when and why.

Here a suggestion was accepted and then reverted. The body is back to "port 8080":

````markdown
Relay listens on port 8080 by default.

<!--endmark suggest s-4q8d by=agent:claude for=adam at=2026-11-11T16:40:00Z
Port changed in v2.

```replace
port 8080
```
```with
port 8443
```
-->

<!--endmark accept s-4q8d by=adam at=2026-11-11T17:40:00Z -->

<!--endmark revert r-6w2p re=s-4q8d by=maria at=2026-11-12T09:15:00Z
The v2 rollout is postponed.

```replace
port 8443
```
```with
port 8080
```
-->
````

To bring the change back, revert the revert. That is recorded in the same way, as a second `revert` record that names the first, and the body reads "port 8443" again:

````markdown
<!--endmark revert r-1k8v re=r-6w2p by=adam at=2026-11-16T10:00:00Z
The rollout went ahead today.

```replace
port 8080
```
```with
port 8443
```
-->
````

`endmark status` then shows all three, and which of them are in effect:

```text
s-4q8d     suggestion accepted         located          agent:claude for adam    1 edit
r-6w2p     revert     applied          reverted         maria                    reverts s-4q8d, 1 edit
r-1k8v     revert     applied          located          adam                     reverts r-6w2p, 1 edit
```

A change can be reverted once. The linter rejects a second revert of the same change, and a revert of anything that was never applied:

```text
notes.md:30: error[already-final] r-9x2c: suggestion "s-4q8d" is already reverted, by "r-6w2p"; to undo that, revert "r-6w2p"
  hint: the item is closed; start a new one, or to undo a revert, revert the revert
notes.md:39: error[wrong-target] r-3h7m: suggestion "s-8k1d" is open, not accepted, so there is no change to revert
  hint: check which kind of item this verb acts on
```

You can go back and forth as often as you like. Each step is one more `revert` record that names the one before it, so the history is a single chain.

### The command

`endmark revert FILE ID` writes these records for you. It puts the text back, appends the `revert` record, and re-points any comment whose quoted text it changed.

```sh
endmark revert --by maria --note "The v2 rollout is postponed." notes.md s-4q8d
```

```text
notes.md: reverted s-4q8d: appended r-6w2p (re=s-4q8d); re-pointed c-7f3k
```

You do not have to find the end of the chain. Name any record in it. If that change is in effect, the command undoes it by reverting the latest record, and says so:

```text
notes.md: reverted s-4q8d: appended r-8t32 (re=r-1k8v); followed the chain from s-4q8d to r-1k8v
```

If it is already undone, the command refuses and tells you what would bring it back:

```text
notes.md: refused[not-in-effect] r-6w2p: "r-6w2p" is already undone; the latest revert in its chain is "r-1k8v"
  hint: to bring it back, revert "r-1k8v"
```

- `--by HANDLE` says who is writing. Set `ENDMARK_BY` to avoid repeating it. Agents add `--for PERSON`, or set `ENDMARK_FOR`.
- `--dry-run` prints the resulting file and writes nothing. So does `-` as the file, which reads standard input.
- The command refuses, with exit status 1 and the file untouched, when the text has changed since the change was applied, or when the change deleted text without recording where. It never guesses. A deletion records where when it has an `--in` with text wider than what it deletes; `suggest` and `edit` warn when they write one that does not.

## The linter

`endmark lint` checks that every record in a file will be read, and that nothing in it will show up in a rendered page by mistake. It is meant for agents to run after every write. It never changes a file.

```sh
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

### When a comment loses its place

A comment points at text by quoting it. If someone rewrites that text, the comment is reported as `detached`, and if the text now appears twice, as `ambiguous`. Endmark never guesses where such a comment belongs. It can suggest:

```sh
endmark lint --suggest notes.md
```

```text
notes.md:6: warning[detached] c-7f3k: quote "port 8080" is not in the body
  hint: append an anchor record with a quote that is in the body, or resolve the thread
  repair: line 3, 94% sure: the lines the quote was in; the quoted words themselves are gone. To accept it: endmark repair notes.md c-7f3k
checked 1 file: 0 errors, 1 warning
```

`endmark repair notes.md` accepts every suggestion it is at least 80% sure of, by appending an `anchor` record for each. `--min-score` changes the 80, `--dry-run` shows the result without writing, and naming ids limits it to those comments. A comment it is not sure enough about is left alone and named, which is not a failure: the exit status is 0.

How good are the suggestions? Replayed against the real revision histories of 15 public documents, about 99.5% of the ones `repair` writes by default were right, and when the search fails it mostly says nothing rather than something wrong. The known weak spot is a deleted paragraph that has a near-identical twin. [eval/](eval/README.md) has the method and the numbers.

The suggestions work because each comment also stores **guideposts**: the line its quote was on (`line=13`) and the lines around it (a `near` fence). They are deliberately imprecise. Lines shift and text gets edited, so guideposts are never used to decide where a comment is, only to propose where it went. A comment moves when an `anchor` record says so, and that record is in the history like everything else.

Three more commands read a file:

- `endmark status FILE` lists every comment, suggestion, edit and revert with its state.
- `endmark log FILE` prints the history, newest first.
- `endmark body FILE` prints the document without its records, and without the blank lines the records leave at its end.

## Writing records

Records are best written by a tool, because quoted text has to be escaped and has to match the document exactly once. There is one command per kind of record. Each appends one record; only `accept`, `edit` and `revert` change the document itself.

```sh
export ENDMARK_BY=maria        # or pass --by to each command

endmark comment --on "port 8080" --note "Is this still right?" notes.md
endmark reply --note "Confirmed." notes.md c-7f3k
endmark suggest --replace "port 8080" --with "port 8443" --note "Port changed in v2." notes.md
endmark accept notes.md s-4q8d
endmark reject --note "Not yet." notes.md s-4q8d
endmark resolve notes.md c-7f3k
```

```text
notes.md: added comment c-7f3k
notes.md: accepted s-4q8d; re-pointed c-7f3k
```

- **Quoted text** (`--on`, `--replace`) is Markdown source and must be in the document exactly once. If it is not, add `--in` with the surrounding text.
- **Several edits in one suggestion**: repeat `--replace` and `--with`. They are accepted or rejected together.
- **Agents** add `--for PERSON` to name who they act for, or set `ENDMARK_FOR`. An author that starts with `agent:` and names no one gets a warning on standard error.
- **Trying it out**: `--dry-run` prints the resulting file and writes nothing.
- `retract` withdraws your own comment or suggestion, `reopen` opens a thread again, `edit` makes a change directly and logs it, and `anchor` points a comment at new text after its quote went missing (`repair` does that for you where it can).

A command that cannot do what was asked changes nothing and says why:

```text
notes.md: refused[quote-ambiguous]: "relay" is in the document more than once
  hint: quote more text, or give the surrounding text as the scope (--in)
```

After `accept`, `edit` or `revert`, the command also tells you which open comments lost their quoted text and which open suggestions no longer apply.

## Using Endmark from an AI agent

`endmark skill` prints a SKILL.md document for AI agents: what Endmark is, the rules an agent must keep to, a command for each task, and what to do when a write is refused. An agent that finds the binary can run it and read the output. To install it as a skill:

```sh
mkdir -p .claude/skills/endmark
endmark skill > .claude/skills/endmark/SKILL.md
```

The text ships inside the binary, so it describes the version you have. The tests run its example session and check every command, option and code it names against the tool.

## Layout

| Path | Holds |
| --- | --- |
| `rfcs/` | The RFC: the format and the plan |
| `crates/endmark-core/` | The reader and the writer: scanner, records, state, anchoring, findings, and one write operation per verb |
| `crates/endmark-cli/` | The `endmark` command |
| `conformance/` | Fixtures any implementation can be checked against |
| `spec/` | The format alone: the normative sections of the RFC |
| `crates/endmark-wasm/` | `endmark-core` as a WebAssembly module |
| `packages/render/` | The review renderer, in TypeScript |
| `demo/` | The demo page |
| `view/` | The page that `endmark view` serves. Its built form is committed in `crates/endmark-cli/view/` and embedded in the binary. |
| `compat/` | The check that plain Markdown renderers hide records |
| `eval/` | A measurement of repair suggestions against real revision histories |

## The demo page

The demo page shows one document twice: drawn with its comments and suggestions on the left, and as raw source on the right. Comment, suggest, accept, revert, or edit the text directly, and watch what each one does to the file. It saves nothing and sends nothing.

```sh
cd packages/render
bun install
bun run build     # builds the WebAssembly module and demo/dist/
bun run serve     # http://127.0.0.1:4173/
```

The page is static files. It has no format logic of its own: every read and write goes through `endmark-core`, compiled to WebAssembly, so it cannot drift from the command-line tool. See [packages/render](packages/render/README.md) and [demo](demo/README.md).

## Does it really stay hidden?

Yes in every CommonMark engine tested, with raw HTML left on. [compat/](compat/README.md) renders the fixtures through nine Markdown engines on every pull request and fails if a record shows. Engines with raw HTML turned off, which is the default of the JavaScript markdown-it, print records as text. Section 12.7 of the RFC has the table and the one known exception.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Changes are listed in [CHANGELOG.md](CHANGELOG.md), and [SECURITY.md](SECURITY.md) says what to keep in mind when agents read files with hidden records.

## Licence

MIT. See [LICENSE](LICENSE).
