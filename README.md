# Endmark

Endmark adds comments, grouped suggestions and a decision history to a Markdown file without changing how the file renders. The Markdown body stays untouched, and all review state lives in an append-only log of HTML-comment records at the end of the file.

**Status: early.** The reader, the linter and the commands that write records work. The renderer and the demo page do not exist yet. The format is defined in [RFC 0001](rfcs/0001-endmark.md), which is open for comment and may still change.

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

- `--by HANDLE` says who is writing. Set `ENDMARK_BY` to avoid repeating it. Agents add `--for PERSON`.
- `--dry-run` prints the resulting file and writes nothing. So does `-` as the file, which reads standard input.
- The command refuses, with exit status 1 and the file untouched, when the text has changed since the change was applied, or when the change deleted text without recording where. It never guesses.

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

Three more commands read a file:

- `endmark status FILE` lists every comment, suggestion, edit and revert with its state.
- `endmark log FILE` prints the history, newest first.
- `endmark body FILE` prints the document without its records.

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
- **Agents** add `--for PERSON` to name who they act for.
- **Trying it out**: `--dry-run` prints the resulting file and writes nothing.
- `retract` withdraws your own comment or suggestion, `reopen` opens a thread again, `edit` makes a change directly and logs it, and `anchor` points a comment at new text after its quote went missing.

A command that cannot do what was asked changes nothing and says why:

```text
notes.md: refused[quote-ambiguous]: "relay" is in the document more than once
  hint: quote more text, or give the surrounding text as the scope (--in)
```

After `accept`, `edit` or `revert`, the command also tells you which open comments lost their quoted text and which open suggestions no longer apply.

## Layout

| Path | Holds |
| --- | --- |
| `rfcs/` | The RFC: the format and the plan |
| `crates/endmark-core/` | The reader and the writer: scanner, records, state, anchoring, findings, and one write operation per verb |
| `crates/endmark-cli/` | The `endmark` command |
| `conformance/` | Fixtures any implementation can be checked against |

## Planned

- a reference review renderer
- a demo page that shows a document beside its raw source and lets you watch the source change as you comment, suggest, accept and revert

## Licence

MIT. See [LICENSE](LICENSE).
