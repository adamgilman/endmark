import { expect, test } from "bun:test";
import {
  alignText,
  applyEdits,
  clean,
  createMarkdown,
  editMarks,
  renderMarked,
  settle,
  splitBlocks,
} from "../src/blocks";
import { ByteMap, byteToIndex, indexToByte } from "../src/bytes";
import { changedLines, lineDiff, wordDiff, words } from "../src/diff";
import { buildThreads, buildUnits, closedItems, inEffect, revertChain, unitAt, type DrawEdit } from "../src/model";
import type { Item } from "../src/types";

// Byte ranges.

test("byte offsets convert to string indices across multi-byte text", () => {
  const text = "a\u00e9\u4e2d\u{1f600}z"; // 1, 2, 3 and 4 byte characters.
  const bytes = new TextEncoder().encode(text);
  expect(bytes.length).toBe(11);
  const map = new ByteMap(text);
  expect(map.byteLength).toBe(11);
  // Byte offset of each character start, and the string index it maps to.
  const starts: [number, number][] = [[0, 0], [1, 1], [3, 2], [6, 3], [10, 5], [11, 6]];
  for (const [byte, index] of starts) {
    expect(map.toIndex(byte)).toBe(index);
    expect(byteToIndex(text, byte)).toBe(index);
    expect(map.toByte(index)).toBe(byte);
    expect(indexToByte(text, index)).toBe(byte);
  }
  expect(map.range([6, 10])).toEqual([3, 5]);
  expect(text.slice(3, 5)).toBe("\u{1f600}");
  expect(map.range(null)).toBeNull();
});

test("ByteMap agrees with TextEncoder at every index", () => {
  const text = "Relay \u2192 r\u00e9sum\u00e9 \u{1f680} port 8080\n\u65e5\u672c\u8a9e";
  const map = new ByteMap(text);
  const encoder = new TextEncoder();
  for (let i = 0; i <= text.length; i++) {
    const code = text.charCodeAt(i);
    if (code >= 0xdc00 && code <= 0xdfff) continue; // Inside a surrogate pair.
    expect(map.toByte(i)).toBe(encoder.encode(text.slice(0, i)).length);
    expect(map.toIndex(map.toByte(i))).toBe(i);
  }
});

// Diffs.

test("words splits into words, white space and single other characters", () => {
  expect(words("port 8080, ok")).toEqual(["port", " ", "8080", ",", " ", "ok"]);
});

test("wordDiff keeps shared words and pairs a deletion with its insertion", () => {
  expect(wordDiff("port 8080", "port 8443")).toEqual([
    { op: "equal", text: "port " },
    { op: "delete", text: "8080" },
    { op: "insert", text: "8443" },
  ]);
  expect(wordDiff("retried twice", "retried 3 times")).toEqual([
    { op: "equal", text: "retried " },
    { op: "delete", text: "twice" },
    { op: "insert", text: "3 times" },
  ]);
  expect(wordDiff("same", "same")).toEqual([{ op: "equal", text: "same" }]);
  expect(wordDiff("", "new")).toEqual([{ op: "insert", text: "new" }]);
  expect(wordDiff("old", "")).toEqual([{ op: "delete", text: "old" }]);
});

test("wordDiff round-trips both sides", () => {
  const before = "Failed deliveries are retried 3 times with a fixed 5 second delay.";
  const after = "Failed deliveries are retried up to 5 times, with a 10 second delay.";
  const diff = wordDiff(before, after);
  expect(diff.filter((s) => s.op !== "insert").map((s) => s.text).join("")).toBe(before);
  expect(diff.filter((s) => s.op !== "delete").map((s) => s.text).join("")).toBe(after);
});

test("lineDiff and changedLines", () => {
  expect(lineDiff("a\nb\nc\n", "a\nB\nc\nd\n")).toEqual([
    { op: "equal", line: "a" },
    { op: "delete", line: "b" },
    { op: "insert", line: "B" },
    { op: "equal", line: "c" },
    { op: "insert", line: "d" },
  ]);
  expect(changedLines("a\nb\nc\n", "a\nB\nc\nd\n")).toEqual([1, 3]);
  expect(changedLines("a\n", "a\n")).toEqual([]);
});

// Blocks and marks.

const md = createMarkdown();
const body = "# Relay\n\nIt listens on port 8080 by default.\n\n## Setup\n\n1. Install `relay`.\n2. Run it.\n\n```mermaid\ngraph LR\n  A --> B\n```\n";

test("splitBlocks finds the top-level blocks and their source", () => {
  const { blocks } = splitBlocks(md, body);
  expect(blocks.map((b) => [b.type, body.slice(b.start, b.end)])).toEqual([
    ["heading_open", "# Relay"],
    ["paragraph_open", "It listens on port 8080 by default."],
    ["heading_open", "## Setup"],
    ["ordered_list_open", "1. Install `relay`.\n2. Run it."],
    ["fence", "```mermaid\ngraph LR\n  A --> B\n```"],
  ]);
  expect(blocks[4]!.lang).toBe("mermaid");
  expect(blocks[4]!.content).toBe("graph LR\n  A --> B\n");
});

test("a word-level edit is carried through markdown-it as marks", () => {
  const source = "It listens on port 8080 by default.";
  const start = source.indexOf("port 8080");
  const marks = editMarks(source, { id: "s-1", start, end: start + 9, with: "port 8443" });
  expect(marks.map((m) => [m.kind, m.start, m.end, m.text])).toEqual([
    ["del", start + 5, start + 9, undefined],
    ["ins", start + 9, start + 9, "8443"],
  ]);
  const marked = renderMarked(md, source, marks)!;
  expect(marked.dropped).toEqual([]);
  expect(clean(marked.html)).toBe("<p>It listens on port 80808443 by default.</p>\n");
});

test("an edit that keeps a heading a heading is drawn in place; one that changes its level is not", () => {
  const same = renderMarked(md, "## Setup", editMarks("## Setup", { id: "s", start: 0, end: 8, with: "## Installation" }));
  expect(clean(same!.html)).toBe("<h2>SetupInstallation</h2>\n");
  expect(renderMarked(md, "## Setup", editMarks("## Setup", { id: "s", start: 0, end: 8, with: "### Setup" }))).toBeNull();
});

test("a highlight that would break the Markdown is dropped, not drawn wrong", () => {
  const source = "See [the docs](https://example.com/a) now.";
  const start = source.indexOf("example");
  const marked = renderMarked(md, source, [{ kind: "quote", id: "c-1", start, end: start + 7 }])!;
  expect(marked.dropped).toHaveLength(1);
  expect(marked.html).toBe('<p>See <a href="https://example.com/a" rel="noopener noreferrer">the docs</a> now.</p>\n');
});

test("settle moves a highlight off a heading marker", () => {
  expect(settle("## Setup", 0, 8, false)).toEqual([3, 8]);
  expect(settle("- item\n", 0, 7, false)).toEqual([2, 6]);
  expect(settle("## Setup", 0, 8, true)).toEqual([0, 8]);
});

test("raw HTML is not passed through", () => {
  expect(md.render("<b>hi</b> <script>x</script>")).not.toContain("<script>");
  expect(md.render("<b>hi</b>")).toContain("&lt;b&gt;");
});

test("applyEdits applies several edits at once", () => {
  expect(applyEdits("one two three", [{ start: 8, end: 13, with: "3" }, { start: 0, end: 3, with: "1" }])).toBe("1 two 3");
});

test("alignText places rendered text in its source", () => {
  const source = "1. Install the `relay` binary.";
  const text = "Install the relay binary.";
  const aligned = alignText(text, source)!;
  const from = aligned.last[text.indexOf("relay")]!;
  const to = aligned.first[text.indexOf("relay") + 4]! + 1;
  expect(source.slice(from, to)).toBe("relay");
  expect(aligned.last[0]).toBe(3);
  expect(alignText("a & b", "a &amp; b")).not.toBeNull();
  expect(alignText("x", "abc")).toBeNull();
});

// Arranging items.

const edit = (sid: string, start: number, end: number): DrawEdit => ({ sid, slot: 0, n: 0, start, end, replace: "", with: "" });

test("buildUnits attaches edits to blocks and merges blocks an edit spans", () => {
  const { blocks } = splitBlocks(md, body);
  const port = body.indexOf("port 8080");
  const one = buildUnits(blocks, [edit("s-1", port, port + 9)]);
  expect(one.units).toHaveLength(5);
  expect(one.units[1]!.edits).toHaveLength(1);
  expect(unitAt(one.units, port)).toBe(1);

  const spanning = buildUnits(blocks, [edit("s-2", body.indexOf("by default"), body.indexOf("Setup") + 5)]);
  expect(spanning.units).toHaveLength(4);
  expect(spanning.units[1]!.blocks).toHaveLength(2);

  const appended = buildUnits(blocks, [edit("s-3", body.length, body.length)]);
  expect(appended.loose).toHaveLength(1);
  expect(appended.units).toHaveLength(5);
});

const item = (over: Partial<Item>): Item => ({
  id: "x", kind: "comment", line: 1, by: "a", at: "2026-01-01T00:00:00Z", for: null, note: "", re: null, state: "live", events: [], ...over,
});

test("threads, chains and history order", () => {
  const items: Item[] = [
    item({ id: "s-1", kind: "suggestion", state: "accepted", status: "reverted", reverted_by: "r-1", line: 1 }),
    item({ id: "c-1", thread: "resolved", line: 2, at: "2026-01-02T00:00:00Z" }),
    item({ id: "c-2", re: "c-1", root: "c-1", line: 3 }),
    item({ id: "c-3", re: "c-2", root: "c-1", line: 4 }),
    item({ id: "r-1", kind: "revert", state: "applied", status: "located", re: "s-1", line: 5, at: "2026-01-03T00:00:00Z" }),
  ];
  const threads = buildThreads(items);
  expect(threads).toHaveLength(1);
  expect([...threads[0]!.depth]).toEqual([["c-1", 0], ["c-2", 1], ["c-3", 2]]);
  expect(inEffect(items[0]!)).toBe(false);
  expect(inEffect(items[4]!)).toBe(true);
  expect(revertChain(items, items[0]!).map((r) => r.id)).toEqual(["r-1"]);
  expect(closedItems(items, threads).map((c) => c.item.id)).toEqual(["r-1", "c-1", "s-1"]);
});
