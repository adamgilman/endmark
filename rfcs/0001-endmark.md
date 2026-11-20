# RFC 0001: Endmark, a collaboration layer for Markdown

| | |
| --- | --- |
| Status | Draft, open for comment |
| Date | 2026-11-13 |
| Author | Adam Gilman |
| Derived from | "Margin" draft spec v0.1 (2026-11-11). Margin was the working name; see D-6. |
| Licence | MIT, for both this text and the reference implementation |

## 1. Summary

Endmark adds comments, grouped suggestions and a decision history to a Markdown file without changing how the file renders. The Markdown body stays untouched. All collaboration state lives in an append-only log of HTML-comment records, normally at the end of the file. A renderer that knows nothing about Endmark shows the current accepted document and nothing else.

This RFC does two things:

1. It turns draft spec v0.1 into normative rules that two independent implementations could follow and agree on.
2. It plans the reference implementation: a Rust core and linter that agents can run after every write, the rest of the command-line tool, a review renderer, a demo page that shows a document beside its raw source, and a language-neutral conformance suite.

Rules marked **(new)** are not in the draft spec. They fill gaps found while writing this RFC and are listed together in [section 14](#14-what-this-rfc-adds-to-the-draft-spec). Decisions are in [section 13](#13-decisions-needed), each with a recommendation. D-1, D-6 and D-8 are settled, and the rules in section 14 are accepted. The rest are open.

## 2. Goals and non-goals

**Goals**

1. **The body is always the valid, current document.** Remove the records and what remains is plain Markdown, byte for byte.
2. **No visual residue.** A plain renderer shows no markers, braces or stray text.
3. **Append-only.** Adding a comment means appending text. No existing record is rewritten.
4. **Writable by hand and by a model.** Quoted text is literal inside fences, with one escape rule.
5. **Safe under unaware edits.** Any tool may edit the body. The worst outcome is a detached anchor, never a corrupt document.
6. **Self-contained.** One file carries the document, the discussion and the decisions.

**Non-goals**

- Real-time co-editing or conflict-free merging of body text.
- Recording every keystroke. Version control remains the full history. Endmark records decisions.
- Authenticating authors. Trust comes from the transport (section 10).
- MDX. MDX rejects HTML comments as a syntax error. Endmark targets CommonMark.
- Showing suggestions in plain renderers. Suggestions are a Endmark feature, not a Markdown one (D-1).

## 3. Conventions and conformance

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are to be read as in RFC 2119.

"CommonMark" means CommonMark 0.31.2. Lines end in LF. A reader normalises CRLF to LF before any scanning or matching.

Four roles can claim conformance:

| Role | Does | Must implement |
| --- | --- | --- |
| Reader | Parses a file into body, items and state | Sections 4 to 8 |
| Writer | Appends records and, for `accept` and `revert`, edits the body | Reader, plus section 9 |
| Renderer | Draws the body with records layered on top | Reader, plus the MUSTs in section 11 |
| Agent | A model or automated tool that reads or writes Endmark files | Reader or Writer, plus the MUSTs in section 10 |

A conforming implementation passes every conformance fixture for its role (section 12.6).

## 4. File model

A Endmark file is a Markdown body plus zero or more records. The extension stays `.md`.

### 4.1 Finding records

A **record** starts on a line that begins, at column 0, with the eleven characters `<!--endmark` followed by a space, and that is not inside a fenced code block of the body.

Readers find records with a line scanner. The draft spec's example parser worked this way. Its prose instead defines a record as a top-level CommonMark HTML block of start condition 2; D-8 covers the difference.

1. Start in state *body*.
2. In *body*, a line of up to three spaces, then three or more backticks or three or more tildes, opens a code fence. Remember the fence character and its length. Move to state *fence*. A backtick line whose info string contains a backtick, such as `` ```x``` y ``, does not open a fence **(new)**.
3. In *fence*, a line of up to three spaces, then the same fence character repeated at least as many times, then only whitespace, closes the fence. Move to *body*. Every other line stays in the fence.
4. In *body*, a line starting `<!--endmark ` at column 0 starts a record. If that line contains `-->`, the record is that single line and its header is the text before the first `-->`. Otherwise move to state *record*.
5. In *record*, a line that is exactly `-->` ends the record. Move to *body*. A line that starts a record, outside any fence in the record's content, also ends it, on the line before (5.4).
6. A record still open at end of file is malformed and runs to end of file **(new)**.

Text inside a body code fence is never a record, so a document can show record examples.

### 4.2 The body

The **body** is the file with every record line removed, each with its line ending. Blank lines that separated records stay in the body.

A match (section 6) MUST NOT span a position from which a record was removed **(new)**. This only matters when an unaware editor has added body text after the log.

### 4.3 Placement and ordering

- Writers MUST add records at the end of the file, each preceded by a blank line.
- Writers MUST NOT modify, reorder or delete existing records.
- Readers MUST accept records anywhere the scanner finds them, because an unaware editor may append body text after the log.
- Log order is file order.
- Readers MUST ignore records with an unknown verb, and writers MUST preserve them.
- Readers MUST ignore unknown attributes on known verbs **(new)**.

Records are confined to column 0 at top level for a reason. Placed between two list items, a record splits the list in two and restarts its numbering.

## 5. Record syntax

A record is one HTML comment. It has a header line, and optionally content: literal text in fences, and a free-form note.

````text
<!--endmark VERB ID key=value key=value
```on
quoted body text, literal
```
A note in Markdown.
-->

<!--endmark VERB ID key=value key=value -->
````

### 5.1 Header

```abnf
header      = "<!--endmark" 1*SP verb 1*SP id *( 1*SP attr ) [ 1*SP "-->" ]
verb        = 1*( %x61-7A / "-" )                 ; lower-case letters and hyphen
id          = 1*( ALPHA / DIGIT / "-" / "_" )
attr        = key "=" value
key         = ALPHA *( ALPHA / DIGIT / "-" )
value       = 1*<any character except whitespace>
```

- For `comment`, `suggest`, `edit` and `revert`, the id names the new item. For every other verb it names the item acted on.
- Ids are unique per file. Writers SHOULD use a type letter, a hyphen, and four or more random base-32 characters, such as `c-7f3k`, so that offline writers do not collide.
- A value MUST NOT contain `-->` or `--!>` **(new)**.
- If a key appears twice, the first value wins **(new)**.

| Attribute | Required | Meaning |
| --- | --- | --- |
| `by` | Yes | Author handle. Non-human authors are prefixed `agent:`. |
| `at` | Yes | RFC 3339 timestamp in UTC, ending in `Z`. |
| `for` | No | The person an agent acted for. |
| `re` | `revert`: yes. `comment`: no. | On a comment, the item being replied to. On a revert, the change being undone (section 7). |

A record missing `by` or `at` is malformed **(new)**. So is a `revert` missing `re` **(new)**.

### 5.2 Content

- The multi-line form ends with a line that is exactly `-->`. The single-line form has no content.
- A **structural fence** is a backtick fence at column 0 whose info string is exactly `on`, `in`, `replace` or `with`. It is closed by a line of at least as many backticks and nothing else.
- Fence content is literal. It is the lines between the opening and closing fence lines, joined by LF, with no trailing newline. To end the content with a newline, leave an empty last line.
- If the content contains a run of backticks, use a longer fence, as in Markdown.
- Everything outside structural fences is the **note**, written in Markdown. The position of the note relative to the fences carries no meaning.
- Any other backtick fence at column 0 belongs to the note, with its content. A reader tracks these, so that an `on` line shown inside a note's own code block is not read as structure. Tilde fences and indented fences in a note are not tracked. As in the body, a backtick line whose info string contains a backtick does not open a fence.

### 5.3 The one escape rule

Record content must never contain `-->` or `--!>`, because either closes the HTML comment early. Writers MUST write them as `--\>` and `--!\>`.

- **Writing.** In every match of `--`, optional `!`, zero or more backslashes, `>`, insert one backslash before the `>`.
- **Reading.** In every match of `--`, optional `!`, one or more backslashes, `>`, remove one backslash.

The rule applies to the whole content, fences and note alike, and is the only transformation between the file and the literal text.

### 5.4 Malformed records

A record is **malformed** when any of these hold:

- A content line contains `-->` and is not the terminator line. The comment closes before the record does.
- A content line contains `--!>` **(new)**. Browsers close the comment there even though CommonMark does not.
- The terminator appears while a fence, structural or not, is still open.
- A single-line record has text after its `-->` **(new)**.
- It is missing a required attribute, including the `re` of a `revert`, or its header does not match the grammar **(new)**.
- A `replace` fence is not followed by a `with` fence **(new)**.
- A fence the verb requires is absent or empty **(new)**: a `suggest`, `edit` or `revert` with no edits, an `anchor` with no `on`, an `in` with no quote after it, or an empty `on` or `in`.
- A structural fence does not fit the verb **(new)**: a `replace` or `with` in a comment, an `on` in a suggestion, a second `on`, an `in` that does not come directly before the quote it scopes, or any structural fence in `accept`, `reject`, `retract`, `resolve` or `reopen`.
- It is still open at end of file, or where the next record starts **(new)**.

Readers MUST ignore a malformed record and MUST report it. A malformed header MUST NOT stop the reader from processing the rest of the file. After an early close, readers SHOULD skip ahead to the next line that is exactly `-->` and treat the skipped lines as part of the malformed record, not as body. The skip is bounded **(new)**: if another record starts, or the file ends, before such a line is found, the malformed record ends at the early close and nothing is skipped. Without the bound, one forgotten escape could swallow every record after it.

A line that starts a record, outside any fence in the content, ends an open record on the line before it **(new)**. The open record is malformed, and the new record is read normally. A record start inside a fence is content: a suggestion may quote a document's own example of a record.

A forgotten escape is the most likely authoring error, because Mermaid arrows contain `-->`. In a plain renderer the tail of such a record leaks as visible text under the document. Records are best written by a tool.

### 5.5 Why fences and not YAML or JSON

Markdown text is hostile to YAML. PyYAML reads `on: # Heading` as null, fails outright on `on: *emphasis*` and `on: [link](url)`, and parses the key `on` as boolean true. JSON is safe but turns a multi-line quote into one `\n`-escaped string. Fences are the quoting mechanism that Markdown writers and models already use.

## 6. Anchoring

A record points at the body by quoting source text that must match exactly once.

- The quote is the `on` fence of a comment or anchor, or the `replace` fence of an edit.
- An optional `in` fence before it gives a wider scope.
- Without `in`, the quote must occur exactly once in the body.
- With `in`, the scope must occur exactly once in the body, and the quote exactly once inside the scope.
- Matching is on raw Markdown source, byte for byte, after CRLF normalisation. Files are UTF-8, and no Unicode normalisation is applied **(new)**.
- Occurrences are counted at every start position, so `aa` occurs twice in `aaa` **(new)**.
- A quote may span lines and blocks, and may sit inside a code block.
- Zero matches is **detached**. More than one is **ambiguous**. Readers treat both as **unanchored** and MUST NOT guess.
- Writers MUST emit anchors that resolve at write time, widening the quote or adding `in` until it is unique.
- A comment with no `on` fence applies to the whole document, or to the item named by `re`.

To comment on one character of a repeated word, scope it:

````text
```in
retried 3 times
```
```on
3
```
````

**Insertions and deletions**

- To insert text, quote a neighbour in `replace` and repeat it in `with` alongside the addition.
- An empty `with` deletes the quoted text.
- An empty `replace` appends to the end of the body. The insertion point is immediately after the last non-blank body line and its line ending **(new: the draft spec does not say where the end is when a log follows)**.

**Why quotes and not inline markers.** Inline markers such as `<!--c1-->` break Markdown in ways that are easy to miss. At the start of a line, the marker turns that line into an HTML block and `*emphasis*` on it prints literally, which happened in four of the five engines tested for the draft spec. Markers cannot go inside code blocks, and a model rewriting a paragraph tends to drop them.

The cost of quote anchors is that they detach when the quoted text changes. An aware editor repairs this by appending an `anchor` record. An unaware editor leaves the comment detached, with its original quote preserved.

## 7. Record types

Four verbs create items and six act on them.

| Verb | Id refers to | Content | Meaning |
| --- | --- | --- | --- |
| `comment` | New comment | Optional `in` and `on`, and a note | A remark on quoted text or the whole document. With `re`, a reply to a comment or a remark on a suggestion. |
| `suggest` | New suggestion | A note, and one or more edits | A proposed change set. Its edits are accepted or rejected together. |
| `edit` | New applied change | Same as `suggest` | A change already made to the body, logged for history. Optional (D-3). |
| `revert` | New revert | `re`, an optional note, and one or more edits | Undoes the change named by `re`. Its edits are the ones the writer applied to the body in the same write **(new)**. |
| `accept` | A suggestion | Optional note | The writer applies every edit to the body in the same write. |
| `reject` | A suggestion | Optional note | Declined. The note carries the reason. |
| `retract` | Own comment or suggestion | Optional note | Withdrawn by its author. |
| `resolve` | A thread's root comment | Optional note | Thread closed. |
| `reopen` | A thread's root comment | Optional note | Thread open again. |
| `anchor` | A comment | Optional `in`, and `on` | Re-points the comment after the body changed. The original quote stays in the log. |

**Grouping.** One `suggest` record is one group. Inside it, each `replace` fence starts a new edit and the `with` fence that follows completes it. An `in` fence applies to the next `replace` only. The note is the rationale for the whole group.

**Changes and reverts (new).** A **change** is an item whose edits were applied to the body: an accepted suggestion, an `edit` or a `revert`. A revert undoes one change, named by its `re`, and undoes all of that change's edits together.

- A revert does not erase anything. The record it undoes stays in the log, and the revert is a new record beside it, with its own id, author, time and note.
- A revert is itself a change, so it can be reverted in the same way. That is how a reverted change is brought back: the second revert names the first in its `re`.
- A revert carries its edits in full, like an `edit` record, so a reader never has to compute them and the log still reads as a list of patches.
- A comment can reply to a revert with `re`, as it can to a suggestion.

**Threads.** A thread is a comment plus every comment that reaches it through `re`. The **root** is a comment whose `re` is absent or names something other than a comment **(new: the draft spec says "first comment")**.

## 8. State

State is computed by reading the log in order. Nothing is stored twice.

### 8.1 Stored state

- A suggestion is open, then accepted, rejected or retracted. Those three are final. Later state records for it are ignored.
- A thread is open or resolved, and can move between the two.
- A comment is live or retracted. Retraction is final and withdraws that one comment only. The state of its thread does not change **(new: the draft spec gives no state for a retracted comment)**.
- An `edit` item is always applied. So is a `revert` item.
- A change is either not reverted or **reverted** **(new)**. It becomes reverted when a `revert` record names it in `re`. This is final and happens at most once per change: to undo a revert, revert the revert.
- Reverting an accepted suggestion does not reopen it. It stays accepted, and is also reverted.
- If two items share an id, the first wins and the later one is ignored.

Readers ignore and report a `revert` record when **(new)**:

- its `re` names no item, or names an item that appears later in the file;
- its `re` names something that is not a change: a comment, or a suggestion that is open, rejected or retracted;
- the change it names is already reverted.

An ignored revert creates no item, and its id stays free.

A reader does not check that the edits of a revert are the exact inverse of the edits of its target. Like `by`, the `re` link is a claim made by the writer (section 10).

Readers ignore and report an action record when **(new)**:

- its id names no item, or names an item that appears later in the file;
- it is a `retract` whose `by` differs from the `by` of the item;
- it is a `resolve` or `reopen` whose id is not a thread root;
- its verb does not apply to that kind of item;
- it acts on an item whose state is final, including an `anchor` for a retracted comment.

A comment whose `re` names no known item is treated as a root and reported as dangling **(new)**.

### 8.2 Derived state

Derived state is checked against the current body on every read.

- A comment with a quote is **attached**, **detached** or **ambiguous**. The last two are unanchored. The latest `anchor` record for the comment, if any, replaces its original quote for this check.
- A comment with no quote is **unquoted**. It applies to the whole document or to its `re` target, and is not unanchored **(new: the name)**.
- An open suggestion is **applicable** when every edit resolves and no two edits overlap. Otherwise it is **stale**, and the reader reports why: an edit is detached, an edit is ambiguous, or two edits overlap.
- Two edits **overlap** when their matched byte ranges intersect. Ranges that only touch do not overlap. Two empty-`replace` insertions do overlap, because their order would be undefined **(new)**.
- A stale suggestion cannot be accepted. Its author retracts it and suggests again.
- A change is **in effect** unless it is reverted by a revert that is itself in effect **(new)**. So a reverted change is not in effect; if its revert is later reverted, it is in effect again; and so on down the chain. Readers MUST be able to report, for every change, the revert that names it and whether the change is in effect.

### 8.3 History

- Closed items stay in the log with author, time and reason.
- A change holds both the old and the new text, so it is a reversible patch. The `revert` verb applies that patch backwards and records that it did (9.3).
- A reverted change is never removed or rewritten. History shows the change, the revert that undid it, and any revert of that revert, each with its author, time and note.
- A reader MAY locate where a change that is in effect landed by resolving each `with` text, reporting **located** or **unlocatable**. A change that is not in effect is reported as **reverted** and is not located. If the edit had an `in` scope, the scope is first rewritten with the edit applied. This is best effort: the text may have changed since or may not be unique. An empty `with` cannot be located. Rejected and retracted suggestions are not located **(new: the last two sentences)**.
- Edits made by unaware tools leave no record.

## 9. Writer behaviour

### 9.1 Appending

A writer appends a blank line and then the record, at the end of the file. Before writing, it MUST check that the new record will be found by the scanner in section 4.1, which fails if the body ends inside an unclosed code fence **(new)**. It MUST apply the escape rule to all content.

### 9.2 Accepting

To accept suggestion *S*, a writer MUST:

1. Compute state. Fail if *S* is not open or is stale.
2. Resolve every edit of *S* against the current body.
3. Apply all edits together, replacing each matched range with its `with` text.
4. Append the `accept` record.
5. Write the result as one atomic file replacement.

A writer MUST NOT append `accept` without applying the edits.

### 9.3 Reverting

This whole section is **(new)**.

**Following the chain.** A change, the revert that names it, the revert that names that revert, and so on, form a **chain**. Its last link, which nothing has reverted, is the **tip**. A chain can be any length. A `revert` record always names the tip, because every earlier link is already reverted (8.1), but a person should not have to find it. So a writer accepts the id of any link, *X*, and works out the target itself:

- If *X* is not a change, fail.
- If *X* is in effect, the target *C* is the tip of *X*'s chain. Reverting the tip undoes *X*. When *X* has never been reverted, the tip is *X*.
- If *X* is not in effect, fail: it is already undone. The writer SHOULD say which id to revert to bring it back, which is again the tip.

The writer SHOULD report the id it named when that is not the id it was given.

To revert the target *C*, a writer MUST:

1. Compute state and find *C* as above.
2. Build the inverse of every edit of *C*: its `replace` is the edit's `with`, its `with` is the edit's `replace`, and its `in`, if the edit had one, is the edit's scope with the edit applied.
3. Resolve every inverse edit against the current body. Fail if one does not match exactly once, or if two overlap. The text has changed since *C* was applied, and the writer MUST NOT guess where it went.
4. Apply all inverse edits together.
5. Append a `revert` record with a new id, `re` naming *C*, and the inverse edits.
6. Write the result as one atomic file replacement.

A writer MUST NOT append `revert` without applying its edits, and MUST NOT modify or delete the record of *C*.

A revert is all or nothing, like an accept. To undo part of a change, write a suggestion.

**Deletions.** If an edit of *C* had an empty `with`, its plain inverse would have an empty `replace`, which appends to the end of the body and not where the text was removed.

- If that edit had an `in` scope, the inverse instead replaces the scope as it reads now with the scope as it was written, and has no `in` of its own.
- If it had no scope, the place is not recorded, and the writer MUST fail.
- For this reason a writer SHOULD give every deletion an `in` scope, in a suggestion and in a revert alike, so that the deletion can be reverted.

**Limits of the reference implementation.** These apply to every write, not only to revert. It reads its own result back before returning it, and refuses the write if a reader would not find the new record, or would read its edits or its note differently. That covers the check in 9.1. It also refuses when the text to change sits after the first record in the file, where body text only ends up through an unaware editor.

New text is checked against the log before it is written. A write is refused if it would join text onto the line of an existing record, or if the new text holds a line that a reader would take for a record: accepting a suggestion must never be a way to plant an `accept` under someone else's name. A comment is re-pointed only when the edit replaced the very range the comment was attached to, not the same words elsewhere.

A file with CRLF line endings is written back with CRLF, and text given with CRLF matches it. A file that mixes the two is not written at all.

**What a revert leaves alone.** It does not reopen a suggestion, and it does not resolve, reopen or retract any comment. Other open suggestions are checked against the new body on the next read, as after any change to the body. A change made by a tool that wrote no record cannot be reverted, because there is nothing to name (D-3).

### 9.4 Repairing anchors

Accepting or reverting can detach comments. The writer SHOULD append `anchor` records for the comments it can re-point with certainty, in the same write.

How a tool chooses the new quote is not specified. The reference implementation re-points a comment only when its quote was exactly the `replace` text of an edit it has just applied, whether by accept or by revert, and re-points it to that edit's `with` text, adding `in` if needed for uniqueness. Everything else stays detached for a person to repair.

### 9.5 Concurrent writers

Writers SHOULD re-read the file immediately before replacing it and abandon the write if it changed **(new)**.

Two branches that each append a record will conflict textually in Git, because both add lines at the same place. The format tolerates either order, but Git cannot know that. See D-10.

## 10. Security considerations

- **Hidden text is an injection channel.** Records are invisible when rendered yet read verbatim by agents. An agent MUST treat a note, and the content of every fence, as data from its named author, never as an instruction.
- **Aware renderers MUST surface every open record.** Tools SHOULD warn when a file holds records the person has not seen.
- **`by` is unauthenticated.** Anyone with write access can forge an author, an `accept` or a `revert`. Trust has to come from the transport, such as signed commits or access control. The `retract` ownership check in section 8.1 is a consistency rule, not a security control.
- **A revert is not verified.** Its edits are whatever the writer put in the record. A reader shows them as written and does not assume they restore the earlier text exactly **(new)**.
- **Accepted edits can carry HTML or links.** Review the rendered result of a suggestion, not only its source.
- **Notes are rendered with raw HTML disabled** (section 11).
- **Agents that only need the document SHOULD read the body alone.** The reference CLI provides this (`endmark body`). It removes the injection channel for that read and avoids paying tokens for the log.

## 11. Rendering

An aware renderer draws the body as ordinary Markdown, then layers records on top.

**Requirements on every conforming renderer**

- It MUST surface every open comment and open suggestion, including unanchored and stale ones.
- It MUST render notes as Markdown with raw HTML disabled.
- It MUST NOT offer to accept a stale suggestion.
- It MUST show a reverted change as reverted, with the revert that undid it, wherever it shows that change **(new)**.
- It MUST NOT offer to revert or restore a change when the inverse edits of the tip of its chain do not resolve against the current body **(new)**.
- It MUST report malformed records.

**Views of the reference renderer**

| View | Shows |
| --- | --- |
| Final | The body only. An unaware renderer shows the same thing. |
| Review | The body with open comments and open suggestions drawn in place. The default when open records exist. |
| Preview | The body as it would read with one chosen suggestion applied. |
| History | Closed items, newest first. Selecting one highlights where its change landed. A reverted change is marked with the revert that undid it. Each change in effect offers Revert, and each change not in effect offers Restore. Both follow the chain to its tip (9.3), so the person never picks a link. |

**Drawing rules of the reference renderer**

- **Comments.** The quoted span is highlighted. The thread card sits beside the block on wide screens and under it on narrow ones, with replies indented.
- **Suggestions.** Each edit is drawn in place: removed text struck through, added text underlined. A gutter bar marks each edit. One card per suggestion holds the note, the edit count and the group actions.
- **Diff granularity.** The renderer renders the affected blocks before and after. A change inside one block's inline text gets a word-level diff. Anything larger shows the old and new blocks stacked. Code shown as source is diffed line by line.
- **Rendered blocks.** A fenced block drawn as a picture, such as `mermaid` or math, is drawn twice: the current version and the suggested version, side by side on wide screens and stacked on narrow ones. Added elements are marked where the diagram type allows it.
- **Source diff behind a toggle.** Every rendered block also offers its line diff. A layout engine can rearrange a whole diagram for a one-line change, and some source changes do not alter the picture at all.
- **The current picture comes from the body.** No open suggestion touches the body's own fence, so the original diagram renders exactly as it does in a plain viewer.
- **Validation.** The renderer parses the suggested version of every rendered block before offering Accept. If parsing fails, the card shows the parser error in place of the picture, and accepting requires confirmation.
- **Overlap.** Two suggestions touching the same text are drawn as stacked alternatives, each in its own colour.
- **Unanchored items.** Detached comments and stale suggestions go to a tray at the end, each with its original quote shown as a blockquote.
- **Authors.** An agent's card shows its handle and the person named in `for`.

## 12. Reference implementation

### 12.1 Starting point

Milestones M1 to M5 are implemented, on 2026-11-20: `endmark-core` with the read path and one write operation per verb; the `endmark` command with every command in the table in 12.3; the reader and writer fixtures (12.6); the compatibility matrix (12.7); the renderer in `packages/render`; and the demo page in `demo`. Nothing has been released. The gaps against this RFC that are known are listed in 12.9.

The draft spec came with a 145-line Python example parser. It was an illustration, not a foundation, and it is not part of this repository. Before it was set aside it was run for this RFC, on 2026-11-13, with two results worth keeping.

First, it supports the design. On the Appendix A.1 file it reported the state in A.2. Applying `s-4q8d` produced the three body changes, and with the A.3 records appended `c-7f3k` was attached again. With the escapes removed from A.1, it dropped only `s-4q8d` and left the body's Mermaid fence intact.

Second, it exposed edge cases that the draft spec's prose does not settle, because the example handled each of them differently from the rule this RFC proposes. Each is a required conformance fixture (12.6).

| Case | Rule in this RFC |
| --- | --- |
| Record still open at end of file | Malformed (5.4) |
| `--!>` in content | Malformed (5.4) |
| Terminator inside an open fence | Skip to the next `-->` line (5.4) |
| Header token without `=` | Malformed, reading continues (5.4) |
| Missing `by` or `at` | Malformed (5.1) |
| Duplicated attribute key | First value wins (5.1) |
| Text after `-->` on a single-line record | Malformed (5.4) |
| Body line such as `` ```x``` y `` | Does not open a fence (4.1) |
| `replace` with no `with` | Malformed (5.4) |
| Quote `aa` in body `aaa` | Two occurrences, ambiguous (section 6) |
| Two empty-`replace` edits in one suggestion | Stale, overlap (8.2) |
| Empty `replace` | Inserts after the last non-blank body line (section 6) |
| `retract` by someone other than the author | Ignored (8.1) |
| `resolve` or `reopen` naming a reply | Ignored (8.1) |
| `reopen` after `retract` on a comment | Retraction is final (8.1) |
| Comment with no quote | Reported as unquoted (8.2) |
| Rejected or retracted suggestion | Not located (8.3) |
| Applied edit with an empty `with` | Cannot be located (8.3) |
| Match spanning a removed record | Not a match (4.2) |

The rendering test across eight parser configurations is also not in this repository. M3 rebuilds it.

### 12.2 Languages

**The linter is written in Rust.** Agents need a validator they can run after every write: one static binary, no runtime to install or start, and one pass over the file.

A linter has to do everything a reader does, so the Rust code is a complete reader. The recommendation (D-7) is to make it the only core, not one of two:

- `endmark-core`, a Rust crate, holds the scanner, parser, state, anchoring, diagnostics and writer operations.
- `endmark`, a Rust binary, is the linter and the rest of the command-line tool.
- The review renderer is TypeScript, because it runs in a browser and Mermaid is a JavaScript library. It calls `endmark-core` compiled to WebAssembly and has no parser of its own.

The conformance suite is plain files, so an implementation in any language can be checked against it.

### 12.3 Layout

```text
spec/endmark.md        The normative spec: sections 3 to 11 of this RFC and its worked example, generated from it
conformance/          Fixtures: an input .md and the expected .json beside it
crates/endmark-core/   Scanner, parser, state, anchoring, diagnostics, writer operations
crates/endmark-cli/    The `endmark` binary
crates/endmark-wasm/   endmark-core as a WebAssembly module, behind one JSON call
compat/               The compatibility matrix (12.7)
packages/render/      The review renderer, in TypeScript, on endmark-core as WebAssembly
demo/                 The demo page (12.5): static files built on the renderer
rfcs/                 This document and later ones
```

**Core, read path**

```rust
pub fn parse(source: &str) -> Document;

pub struct Document {
    pub body: String,
    pub items: Vec<Item>,             // comments, suggestions, edits, with stored and derived state
    pub diagnostics: Vec<Diagnostic>, // code, severity, line, item id, message, hint
}
```

**Core, write path.** One function per verb. Each takes the source and returns the new source, or fails without writing. `accept` performs sections 9.2 and 9.4, and `revert` performs 9.3 and 9.4.

**CLI**

| Command | Does |
| --- | --- |
| `endmark lint FILE...` | Validates files (12.4). The first command to ship. |
| `endmark body FILE` | Prints the body only |
| `endmark status FILE` | Lists every item with its state; `--format json` for tools |
| `endmark comment FILE` | Comment on quoted text (`--on`, with an optional `--in`), or on the whole document |
| `endmark reply FILE ID` | Reply to a comment, or remark on another item |
| `endmark suggest FILE` | Propose one or more edits, each `[--in] --replace --with`. The body is not changed. |
| `endmark edit FILE` | Make one or more edits now and log them with an `edit` record |
| `endmark accept FILE ID` | Apply every edit of a suggestion and append the record (9.2, 9.4) |
| `endmark reject`, `retract FILE ID` | Append the action |
| `endmark resolve`, `reopen FILE ID` | Append the action. `ID` may be any comment of the thread; the record names the root. |
| `endmark anchor FILE ID` | Re-point a comment at new text (`--on`, with an optional `--in`) |
| `endmark revert FILE ID` | Undo a change: apply its edits backwards and append a `revert` record. `ID` is an accepted suggestion, an `edit` or a revert, and may be any link of a chain: the command follows the chain to its tip (9.3). It refuses if `ID` is not in effect, and names the id that would bring it back. `--by` or `ENDMARK_BY` gives the author; `--note`, `--for`, `--id`, `--at` and `--dry-run` are optional. |
| `endmark log FILE` | Prints history, newest first; `--format json` for tools |

Every command that writes takes `--by` (or `ENDMARK_BY`), and optionally `--for`, `--note`, `--id`, `--at` and `--dry-run`. With `-` as the file it reads standard input and prints the result. It exits 1, with the file untouched, when the write is refused.

**What the writing commands refuse (new).** A writer in the reference implementation never writes a record that a reader would ignore or report:

- a quote, or the `replace` text of an edit, that does not match exactly once, or two edits that overlap;
- an action the reader would ignore under 8.1, such as a `retract` by someone other than the author, with the linter's code for it;
- a `resolve` of a resolved thread or a `reopen` of an open one, which would change nothing;
- a comment with neither a quote nor a note;
- a note, a quote or an id that a reader would read back differently from what was given.

After a write that changes the body, the command also names the open comments it detached and could not re-point, and the open suggestions it made stale.

### 12.4 Linter

`endmark lint` tells an agent, or a person, whether a file is a valid Endmark file and whether anything in it will be ignored or shown by mistake. It is the conforming reader with its diagnostics printed.

**Behaviour**

- It never writes. There is no fix mode in v0.1.
- It takes one or more paths, or `-` for standard input, so an agent can check content before writing it to disk.
- It needs no network, no configuration file and no other installed software.
- Releases carry prebuilt binaries for Linux, macOS and Windows. `cargo install` also works.
- First measurement, on 2026-11-13 with a release build on a 10-core arm64 Linux machine: the worked example in 4 ms, and a 1 MB file with 2,000 comments in 0.35 s. On large files the time goes to anchor resolution, which rescans the body for each quote. A benchmark with a budget in CI is still to be added.

**Exit status**

| Status | Meaning |
| --- | --- |
| 0 | No errors. Warnings may have been printed. |
| 1 | At least one error, or at least one warning under `--strict`. |
| 2 | A file could not be read, or the arguments were wrong. |

**Output.** One line per finding by default, each followed by its hint, then a one-line summary. `--format json` gives the same findings as one JSON object per file, one object per line. Both forms carry the file, line, severity, code, the id of the item when there is one, a message, and a hint that says how to fix it. Codes and the JSON shape are stable within a major version.

The example below is illustrative, not captured output.

```text
notes.md:41: error[early-close] s-4q8d: content contains "-->" before the terminator
  hint: write "--\>" inside records
notes.md:63: warning[detached] c-7f3k: quote "port 8080" not found in body
  hint: append an anchor record, or resolve the thread
```

```json
{"file": "notes.md", "errors": 1, "warnings": 1, "findings": [
  {"line": 41, "severity": "error", "code": "early-close", "id": "s-4q8d",
   "message": "content contains \"-->\" before the terminator",
   "hint": "write \"--\\>\" inside records"},
  {"line": 63, "severity": "warning", "code": "detached", "id": "c-7f3k",
   "message": "quote \"port 8080\" not found in body",
   "hint": "append an anchor record, or resolve the thread"}
]}
```

**Findings.** An **error** means a reader will ignore something the author wrote, or a plain renderer will show something it should hide. A **warning** means the file is valid but an item needs attention.

| Code | Severity | Raised when | Section |
| --- | --- | --- | --- |
| `early-close` | Error | Content contains `-->` or `--!>` before the terminator | 5.4 |
| `unterminated-record` | Error | A record is still open at end of file, or where the next record starts | 5.4 |
| `unterminated-fence` | Error | The terminator appears inside an open fence | 5.4 |
| `trailing-text` | Error | A single-line record has text after `-->` | 5.4 |
| `bad-header` | Error | The header does not match the grammar | 5.1 |
| `missing-attribute` | Error | `by` or `at` is absent, or a `revert` has no `re` | 5.1 |
| `incomplete-edit` | Error | A `replace` has no `with` | 5.4 |
| `missing-fence` | Error | A fence the verb requires is absent or empty | 5.4 |
| `unexpected-fence` | Error | A structural fence does not fit the verb | 5.4 |
| `duplicate-id` | Error | An item reuses an earlier id | 8.1 |
| `unknown-target` | Error | An action, or the `re` of a `revert`, names no item, or one that appears later | 8.1 |
| `wrong-target` | Error | The verb does not apply to that item, `resolve` names a reply, or a `revert` names something that is not a change | 8.1 |
| `not-author` | Error | A `retract` is not by the item's author | 8.1 |
| `already-final` | Error | An action follows a final state, or a `revert` names a change that is already reverted | 8.1 |
| `visible-record` | Error | The scanner and CommonMark disagree about a record (D-8) | 4.1 |
| `detached` | Warning | A live comment in an open thread has a quote that is not in the body | 6 |
| `ambiguous` | Warning | A live comment in an open thread has a quote, or a scope, that occurs more than once | 6 |
| `stale` | Warning | An open suggestion cannot be applied; the message says why | 8.2 |
| `dangling-reply` | Warning | `re` names no known item | 8.1 |
| `body-after-log` | Warning | Body text follows a record | 4.3 |
| `unknown-verb` | Warning | The verb is not one of the ten | 4.3 |
| `unknown-attribute` | Warning | An attribute is not one of the four | 4.3 |
| `bad-timestamp` | Warning | `at` is not an RFC 3339 UTC timestamp | 5.1 |

Unanchored and stale items are warnings because unaware edits cause them legitimately. An agent checking its own write SHOULD use `--strict`, since a writer must emit anchors that resolve at write time (section 6).

### 12.5 Demo page

The demo page lets someone understand the format in a minute, in a browser, without installing anything. It shows one example document twice, side by side, and lets the visitor act on it and watch the source change.

**Layout**

- **Left: the document.** The example file drawn by the reference renderer in Review view (section 11), with its comments and suggestions in place.
- **Right: the raw source.** The same file as text, with the log of records visible at the end.
- On a narrow screen the two are stacked, document first.
- The example is the Relay file from Appendix A.

**What the visitor can do**

| Action on the left | What the right shows |
| --- | --- |
| Select text and comment on it | A `comment` record appended, with the selection as its quote |
| Select text and suggest a replacement | A `suggest` record appended; the body lines are unchanged |
| Reply in a thread, resolve it, reopen it | A `comment` with `re`, a `resolve` or a `reopen` record appended |
| Accept a suggestion | The body lines change, and an `accept` record is appended, with any `anchor` records the writer adds |
| Revert an accepted suggestion, from History | The body lines return to their earlier text, and a `revert` record is appended. The suggestion's record is still there, now shown as reverted. |
| Restore the reverted suggestion, or revert the revert | Either way the body lines change forward again, and a second `revert` record is appended that names the first |
| Revert the suggestion a second time | A third `revert` record is appended. It names the second, not the suggestion: the page followed the chain. |
| Reject or retract a suggestion | A `reject` or `retract` record appended; the body lines are unchanged |
| Edit the body directly, as a tool that does not know Endmark would | Only the body changes. A comment whose quote was edited moves to the unanchored tray. |
| Switch to Final view | Nothing changes. The left shows what a plain Markdown renderer shows. |
| Reset | The original example returns |

- Lines added or changed by the last action are highlighted in the source, and the source scrolls to them.
- The linter's findings for the current source are listed under it, so a visitor sees a warning appear when an edit detaches a comment.
- The source pane is read-only in the first version. Making it editable, so a visitor can type a record and watch it render, is a later addition.
- The visitor's actions are attributed to a fixed handle, `by=you`, with the time taken from the browser's clock.

**What it does not do**

- It saves nothing. The document lives in the page's memory. Nothing is written to browser storage or sent to a server, and a reload returns the original example.
- It makes no network request after the page has loaded.
- It is not an editor for the visitor's own files.

**How it is built.** The page is static files that can be hosted anywhere. It contains no logic of its own for the format: every action calls the write path of `endmark-core`, compiled to WebAssembly, and the left side is the reference renderer. It is therefore also the first end-to-end test of both, and it cannot drift from the spec.

### 12.6 Conformance suite

Each fixture is an input file, `NAME.md`, and the expected reader output, `NAME.json`: the body, every item with its stored and derived state, and every finding with its line and code. An implementation is checked by comparing its equivalent of `endmark status --format json` with the expectation. `conformance/README.md` gives the format. A writer fixture is a directory under `conformance/write/` holding an input file, an operation, and either the expected output file, byte for byte, or the reason the write must be refused. There is at least one for every command that writes.

The suite MUST contain at least one fixture for every MUST and every **(new)** rule in sections 4 to 9, one for every row of the table in 12.1, one for every finding code in 12.4, plus:

- the worked example in Appendix A, before and after accepting `s-4q8d`, after reverting it, and after reverting that revert;
- a chain of reverts, a second revert of one change, and a revert of each kind of item that is not a change;
- the worked example with its escapes removed, expecting one malformed record and an unchanged body fence;
- records shown inside body code fences;
- body text appended after the log;
- quotes that contain backticks, `-->`, `--!>`, `<!--`, and a literal `--\>`;
- CRLF input.

### 12.7 Compatibility evidence

The promise that a plain renderer shows the document and nothing else is checked by `compat/run.py`. It renders every conformance fixture that a reader accepts without errors, once whole and once as the body alone, through each engine. Records are hidden when the two results are the same HTML once comments are removed. CI runs it on every pull request and fails if an engine that should hide records shows one. `compat/RESULTS.md` holds the last run and `compat/requirements.txt` and `compat/package.json` pin the versions.

Results on 2026-11-20, over 31 fixtures:

| Engine | Result |
| --- | --- |
| commonmark-py 0.9.2 (CommonMark reference port) | Hidden |
| commonmark.js 0.31.2 (CommonMark reference implementation) | Hidden |
| markdown-it-py 4.3.0, HTML on | Hidden |
| markdown-it 14.1.0 (JavaScript), HTML on | Hidden |
| marked 15.0.12 | Hidden |
| mistune 3.4.0 | Hidden |
| Python-Markdown 3.11.3 | Hidden in 30 of 31. See below. |
| markdown-it-py, HTML off | **Visible**: records are printed as text (D-4) |
| markdown-it (JavaScript), HTML off, which is its default | **Visible** (D-4) |
| cmark-gfm, unsafe mode | Expected hidden. Run in CI only. |
| cmark-gfm, safe mode, which is its default | Expected removed with all other raw HTML. Run in CI only. |

- **Python-Markdown and an unclosed `<!--` in the body.** If the body itself contains `<!--` with no `-->` after it, Python-Markdown reads it as the start of a comment. That swallows the start of the first record and leaves its end visible. CommonMark engines treat the same text as plain text, and so does the scanner, so `endmark lint` does not report it. Python-Markdown is not a CommonMark engine. The fixture is `escapes-in-quotes`.
- **Malformed records leak.** An unescaped `-->` inside a record shows the rest of the record as visible text in every engine. `--!>` closes the comment early in Python-Markdown, and in browsers under the HTML standard. `endmark lint` reports both as errors, and the writing commands never produce them.
- **markdown-it defaults to HTML off.** A site that uses the JavaScript markdown-it with its defaults will print records. This is the most likely place for a reader to meet D-4.

**Not tested:** Obsidian, the VS Code preview, and GitHub and GitLab web rendering. They cannot be run by a script here and have not been checked by hand. GitHub and GitLab are believed to strip HTML comments, but that has not been verified.

### 12.8 Milestones

Each milestone ends when its exit test passes.

| | Milestone | Exit test |
| --- | --- | --- |
| M0 | Spec candidate | Every open decision in section 13 is settled or explicitly deferred. `spec/endmark.md` exists. The worked example is the first fixture. |
| M1 | Rust core read path and linter | `endmark lint` passes all reader fixtures and raises the documented code for each. A property test shows the body is byte-exact for generated files. CI builds the release binaries and sets the benchmark budget. |
| M2 | Write path and the rest of the CLI | All ten verbs pass writer fixtures. On the worked example, `endmark accept` produces Appendix A.3, `endmark revert s-4q8d` then produces A.4, and `endmark revert r-6w2p` produces A.5. After that, `endmark revert s-4q8d` writes a revert that names `r-1k8v`, and `endmark revert r-6w2p` is refused. |
| M3 | Compatibility matrix | CI renders the fixtures through every engine in 12.7 plus the JavaScript ones, and fails if a record is visible where it should be hidden. Hosted renderers are checked by hand and dated. |
| M4 | Reference renderer | `endmark-core` builds to WebAssembly. All four views work on the worked example, including the two-diagram view and validation. |
| M5 | Demo page | Every action in the table in 12.5 works on the worked example and changes the source as described. Reloading restores the example. The page makes no network request after load and writes nothing to browser storage. |
| M6 | v0.1 release | Spec, packages and demo page published. Token frozen. |

### 12.9 Known gaps in the reference implementation

Recorded on 2026-11-20. Each is a place where the code does less than this RFC says, or where a claim has not been checked.

**Renderer (section 11)**

- A comment that was re-pointed and later detached is shown in the tray with its latest quote, not its original one.
- Added elements in a diagram are marked for flowcharts only. Math blocks are not drawn as pictures.
- There is one gutter bar per edited block, not one per edit.
- Raw HTML in the body is shown as text in every view. A plain renderer with HTML on would pass it through.
- Overlapping suggestions on prose, and line diffs of code blocks other than diagrams, are implemented but have not been exercised in a browser.

**Demo page (12.5)**

- Selecting text maps the rendered selection back to source by a heuristic. When it fails, the core refuses the quote and the page says so.
- There is no way to add a comment on the whole document.
- A direct body edit assumes the records are all at the end of the file, which holds for the example.
- Diagram code is split into files that load on demand from the page's own directory. Editing the body into another kind of diagram fetches one more file. No request leaves that directory.

**Not checked**

- Touch selection, Firefox, Safari and accessibility of the demo page. It was tested in Chromium only.
- Obsidian, the VS Code preview, GitHub and GitLab (12.7).
- The release workflow. No release has been made, so it has not run on a tag.

## 13. Decisions needed

D-1 to D-6 are the open questions of the draft spec. D-7 to D-10 arose while writing this RFC.

| | Question | Recommendation |
| --- | --- | --- |
| D-1 | Should suggestions be invisible in plain renderers, or shown as strikethrough and underline? | **Settled 2026-11-13: invisible.** Suggestions are a Endmark feature, not a Markdown one. |
| D-2 | Should closed records be prunable, or movable to a sidecar file? Closed threads cost tokens on every agent read. | Not in v0.1. Ship `endmark body` and `endmark status` so tools need not read the log. Revisit with usage data, since either option weakens goal 3 or goal 6. |
| D-3 | Must aware editors log an `edit` record for direct changes? | Keep it optional. The body is the truth and version control has the rest. |
| D-4 | Is a second carrier needed for renderers that escape raw HTML? | No. Document it as a known loss. |
| D-5 | Does the format need an approval verb, so an agent can wait on a human verdict? | Defer. Readers already ignore unknown verbs, so it can be added later without breaking them. |
| D-6 | What are the name and the token? | **Settled 2026-11-13: Endmark, and `<!--endmark`.** The working name "Margin" was dropped after a registry check found it in use by projects in the same niche (section 13.1). The token is part of the wire format and cannot change after the first release. |
| D-7 | The linter is Rust; that is decided. Should the Rust crate be the only core, with the renderer calling it as WebAssembly, or should the renderer have its own TypeScript reader? | One Rust core (section 12.2). A second reader in TypeScript would be a second place for the rules to drift. |
| D-8 | Is a record defined by the line scanner in 4.1, or by CommonMark block structure? | **Settled 2026-11-13: the scanner.** The spec defines its own rule. The two can disagree when the body leaves a raw HTML block such as `<pre>` or a comment unclosed, or when a code fence inside a list item is closed implicitly. `endmark lint` reports those files as `visible-record` (12.4). |
| D-9 | Does the file need a format version marker? | No marker in v0.x. Forward compatibility rests on ignoring unknown verbs and attributes. Revisit before 1.0. |
| D-10 | How should concurrent appends merge in Git? | Ship a merge driver in the CLI after v0.1: merge the body three ways as usual and take the union of the two logs. Git's built-in `union` driver is not a substitute, because it would also union conflicting body edits. |

### 13.1 Name check for D-6

**The chosen name.** On 2026-11-13, `endmark` was free on crates.io, npm and PyPI, and one web search found no product of that name. An end mark is the printer's symbol that closes an article, as the log closes the file. Trademarks, domain names and editor marketplaces were not checked.

**Why "Margin" was dropped.** Checked the same day against the public registries. Descriptions are the packages' own.

| Where | Name | Status |
| --- | --- | --- |
| crates.io | `margin` | Taken. "Structured diagnostics core and layout planning". Unrelated, active (last release August 2026). |
| crates.io | `margin-core`, `margin-cli`, `margin-lint`, `margin-md` | Free |
| npm | `margin` | Taken. "margin trader". Unrelated. |
| npm | [`margin-md`](https://www.npmjs.com/package/margin-md) | Taken. "Review AI-written markdown like a Google Doc". **Same niche.** |
| npm | `margin-core`, `margin-cli`, `margin-lint`, `@margin/core` | Free as package names. Whether the `@margin` scope can be claimed was not established. |
| PyPI | [`margin`](https://pypi.org/project/margin/) | Taken. Publishes a repository's Markdown to an Outline wiki and brings comments back to an AI agent. **Adjacent niche.** |
| PyPI | `margin-md`, `margin-lint` | Free |

The npm `margin-md` project is the serious conflict:

- It is called "margin", installs a command called `margin`, and offers a Homebrew formula of the same name.
- It does comments and suggestions on Markdown for AI agents, with accept and reject. It is MIT licensed, was first published on 2026-09-30, and had 83 downloads in the month to 2026-11-12.
- Its design differs from this one: comments live in a sidecar file beside the Markdown, not in the file.

A web search also found [margin.at](https://tangled.org/margin.at/margin), an open annotation layer for web pages on the AT Protocol, and several Obsidian plugins with "margin" in their names. These were seen in search results only and not examined.

Not checked for "Margin": trademarks, domain names, and the VS Code and Obsidian marketplaces.

## 14. What this RFC adds to the draft spec

These rules were accepted on 2026-11-13. None changes the intent of the draft spec.

Rules taken from the draft spec's example parser, which its prose did not state: the line scanner (4.1), structural fences at column 0 with content joined by LF, tracking of code fences inside notes (5.2), touching edits not overlapping (8.2), and rewriting the scope when locating an applied edit (8.3).

Rules that are new:

- A backtick line with a backtick in its info string does not open a body fence (4.1).
- A match may not span a removed record (4.2).
- Unknown attributes are ignored; a duplicated key takes its first value; values may not contain a comment terminator (4.3, 5.1).
- More ways to be malformed: open at end of file, `--!>` in content, text after a single-line terminator, a missing `by` or `at`, a bad header, an incomplete edit, a missing or empty fence, a fence that does not fit the verb (5.4).
- The skip after an early close is bounded, and a record start outside a fence ends an open record (5.4). Both were found while implementing the scanner.
- Occurrences are counted at every start position; files are UTF-8 (section 6).
- The insertion point for an empty `replace` (section 6).
- The thread root, which action records are ignored, and retraction of a comment being final (section 7, 8.1).
- The name "unquoted" for a comment with no quote; two appends in one suggestion overlap (8.2).
- Only accepted suggestions and edits are located, and never by an empty `with` (8.3).
- The accept procedure as ordered steps, the pre-append check, and the concurrent-write check (section 9).
- The observation that concurrent appends conflict in Git (9.5, D-10).
- Agents that need only the document should read the body alone (section 10).
- Conformance roles (section 3) and the conformance suite (12.6), the linter with its finding codes (12.4), and the demo page (12.5).

Added on 2026-11-20, after the rules above were accepted:

- The `revert` verb, a tenth verb and a fourth kind of item (section 7). The draft spec said an accepted suggestion is a reversible patch but gave no way to reverse one on the record.
- The reverted state of a change, final and once per change, and the rule that a revert is undone by reverting it (8.1).
- The derived state "in effect", and that only changes in effect are located (8.2, 8.3).
- The revert procedure, following a chain to its tip, the deletion rule, and anchor repair after a revert (9.3, 9.4).
- Two renderer requirements about reverts (section 11), and `endmark revert` (12.3).

## 15. Licence and contributions

The spec text, the conformance fixtures and the reference implementation are all under the MIT licence in `LICENSE`. Contributions are accepted under the same licence. Anyone may implement the format.

## 16. Prior art

Nothing existing meets all four requirements at once: comments, grouped suggestions, retained history, and a clean render in a plain viewer.

| Approach | Where review state lives | Plain renderer shows | Grouped changes | Why it falls short |
| --- | --- | --- | --- | --- |
| [CriticMarkup](https://github.com/CriticMarkup/CriticMarkup-toolkit) | Inline braces, such as `{++added++}` | The braces, literally | No | Inline only. Its own docs advise against newlines inside tags. No ids, threads or authorship in the base syntax. |
| CriticMarkup plus plugin metadata ([Commentator](https://github.com/Fevol/obsidian-criticmarkup), [Agent Comments](https://github.com/andrewroxby/agent-comments)) | Inline | Same | No | Adds authors and threads through non-standard extensions tied to one editor. |
| Sidecar tools ([comments](https://github.com/rcliao/comments), [spotlight-md](https://trilogyai.substack.com/p/introducing-spotlight-md), [margin-md](https://www.npmjs.com/package/margin-md)) | A separate file or local server | Clean | Varies | The `.md` alone no longer carries the review. |
| Forge review (pull-request suggestions) | The forge's database | Clean | Yes | Invisible to any agent or tool not wired to that forge. |

Endmark borrows three proven ideas: quote anchoring from the W3C [Web Annotation](https://www.w3.org/TR/annotation-model/) text-quote selector, the match-exactly-once rule from agent edit tools, and batches of suggestions applied together from forge code review.

## Appendix A. Worked example

### A.1 The file

It holds one direct edit, one accepted suggestion, two open threads and one open three-edit suggestion.

````text
# Relay

Relay forwards webhook events to internal consumers. It listens on port 8080 by default.

## Setup

1. Install the `relay` binary.
2. Set `RELAY_TOKEN`.
3. Run `relay serve`.

## Retries

Failed deliveries are retried 3 times with a fixed 5 second delay.

```mermaid
graph LR
  A[Webhook] --> B[Relay]
```

<!--endmark edit e-0h5n by=adam at=2026-11-09T09:12:00Z
```replace
Install the binary.
```
```with
Install the `relay` binary.
```
-->

<!--endmark suggest s-3c7j by=maria at=2026-11-09T10:30:00Z
Two retries was the v1 behaviour.

```replace
retried twice
```
```with
retried 3 times
```
-->

<!--endmark accept s-3c7j by=adam at=2026-11-09T11:02:00Z -->

<!--endmark comment c-7f3k by=maria at=2026-11-10T14:05:00Z
```on
port 8080
```
Is this still right? Staging moved to 8443 last month.
-->

<!--endmark comment c-2m9x re=c-7f3k by=agent:claude for=adam at=2026-11-11T16:38:00Z
Confirmed against `deploy/values.yaml`. Fix proposed in s-4q8d.
-->

<!--endmark suggest s-4q8d by=agent:claude for=adam at=2026-11-11T16:40:00Z
Port changed in v2, heading renamed to match the CLI docs, diagram shows the consumer.

```replace
port 8080
```
```with
port 8443
```

```replace
## Setup
```
```with
## Installation
```

```replace
  A[Webhook] --\> B[Relay]
```
```with
  A[Webhook] --\> B[Relay] --\> C[Consumer]
```
-->

<!--endmark comment c-9d1t by=adam at=2026-11-11T17:20:00Z
```in
retried 3 times
```
```on
3
```
Should this be configurable?
-->
````

### A.2 Expected state

| Item | Kind and author | Stored state | Against the current body |
| --- | --- | --- | --- |
| `e-0h5n` | Edit by adam | Applied | New text located |
| `s-3c7j` | Suggestion by maria | Accepted by adam | New text located |
| `c-7f3k` | Comment by maria, one reply | Open | Attached to "port 8080" |
| `c-2m9x` | Reply to `c-7f3k` by agent:claude for adam | Live | Unquoted |
| `s-4q8d` | Suggestion by agent:claude for adam | Open | Applicable, three edits |
| `c-9d1t` | Comment by adam | Open | Attached to "3" |

### A.3 After accepting `s-4q8d`

The three edits are applied to the body. Rewriting "port 8080" detaches `c-7f3k`, and its quote was exactly the `replace` text of an accepted edit, so the writer re-points it (section 9.4). Two records are appended in the same write:

````text
<!--endmark accept s-4q8d by=adam at=2026-11-11T17:40:00Z -->

<!--endmark anchor c-7f3k by=adam at=2026-11-11T17:40:00Z
```on
port 8443
```
-->
````

`c-7f3k` is attached again, to "port 8443".

### A.4 After reverting `s-4q8d`

A day later the rollout is postponed, and maria runs `endmark revert s-4q8d`. The three edits are applied backwards, so the body reads as it did in A.1. Nothing is erased: the `suggest` and `accept` records stay where they are, and two records are appended in the same write. The revert detaches `c-7f3k` from "port 8443", and the writer re-points it as before.

````text
<!--endmark revert r-6w2p re=s-4q8d by=maria at=2026-11-12T09:15:00Z
The v2 rollout is postponed. Staging is back on 8080.

```replace
port 8443
```
```with
port 8080
```

```replace
## Installation
```
```with
## Setup
```

```replace
  A[Webhook] --\> B[Relay] --\> C[Consumer]
```
```with
  A[Webhook] --\> B[Relay]
```
-->

<!--endmark anchor c-7f3k by=maria at=2026-11-12T09:15:00Z
```on
port 8080
```
-->
````

| Item | Kind and author | Stored state | Against the current body |
| --- | --- | --- | --- |
| `s-4q8d` | Suggestion by agent:claude for adam | Accepted by adam, reverted by `r-6w2p` | Not in effect |
| `r-6w2p` | Revert of `s-4q8d` by maria | Applied | In effect, new text located |
| `c-7f3k` | Comment by maria, one reply | Open | Attached to "port 8080" |

### A.5 After reverting the revert

The rollout goes ahead after all, and adam runs `endmark revert r-6w2p`. Bringing a change back is recorded in the same way as undoing it: a second revert that names the first. The body reads as it did in A.3.

````text
<!--endmark revert r-1k8v re=r-6w2p by=adam at=2026-11-16T10:00:00Z
The rollout went ahead today.

```replace
port 8080
```
```with
port 8443
```

```replace
## Setup
```
```with
## Installation
```

```replace
  A[Webhook] --\> B[Relay]
```
```with
  A[Webhook] --\> B[Relay] --\> C[Consumer]
```
-->

<!--endmark anchor c-7f3k by=adam at=2026-11-16T10:00:00Z
```on
port 8443
```
-->
````

| Item | Kind and author | Stored state | Against the current body |
| --- | --- | --- | --- |
| `s-4q8d` | Suggestion by agent:claude for adam | Accepted by adam, reverted by `r-6w2p` | In effect again, new text located |
| `r-6w2p` | Revert of `s-4q8d` by maria | Applied, reverted by `r-1k8v` | Not in effect |
| `r-1k8v` | Revert of `r-6w2p` by adam | Applied | In effect, new text located |
| `c-7f3k` | Comment by maria, one reply | Open | Attached to "port 8443" |

A record cannot name `s-4q8d` a second time, because it is already reverted. The tools hide that: `endmark revert s-4q8d` sees that the change is in effect, follows the chain to its tip, and appends a revert with `re=r-1k8v`. `endmark revert r-6w2p` is refused, because `r-6w2p` is not in effect, with a message that reverting `r-1k8v` is what would bring it back.
