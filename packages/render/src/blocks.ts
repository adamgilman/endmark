// Markdown handling that needs no DOM: splitting a body into blocks, and
// carrying marks (highlights, deletions, insertions) through markdown-it.
//
// A mark is carried by putting one private-use character at each end of it in
// the source. markdown-it treats them as ordinary letters, so they come out in
// the text of the rendered block, where `wrapMarks` in dom.ts finds them.
// A mark that would break the Markdown (for example one that starts inside a
// link target) is detected here and dropped, and the caller draws it another way.

import MarkdownItClass, { type Env, type MarkdownIt, type Token } from "markdown-it";
import { wordDiff } from "./diff";

export type References = Env["references"];

export interface MarkdownOptions {
  /** Draw images. Off, an image is left as its source text, so nothing is fetched. */
  images?: boolean;
}

/** A markdown-it instance with raw HTML disabled. Used for the body and for notes. */
export function createMarkdown(options: MarkdownOptions = {}): MarkdownIt {
  const md = new MarkdownItClass({ html: false, linkify: false, typographer: false });
  if (options.images === false) md.disable("image");
  const base =
    md.renderer.rules.link_open ?? ((tokens, idx, opts, _env, self) => self.renderToken(tokens, idx, opts));
  md.renderer.rules.link_open = (tokens, idx, opts, env, self) => {
    tokens[idx]!.attrSet("rel", "noopener noreferrer");
    return base(tokens, idx, opts, env, self);
  };
  return md;
}

export interface Block {
  /** String indices into the body. `end` is after the last character that is not white space. */
  start: number;
  end: number;
  /** The markdown-it token type that opens the block, such as `paragraph_open` or `fence`. */
  type: string;
  /** The first word of a fence's info string, in lower case. Empty otherwise. */
  lang: string;
  /** The content of a fence or an indented code block. */
  content: string;
}

export interface Parsed {
  blocks: Block[];
  /** Link reference definitions of the whole body, for rendering one block alone. */
  references: References;
}

/** The top-level blocks of a body, in order. */
export function splitBlocks(md: MarkdownIt, body: string): Parsed {
  const env: { references?: References } = {};
  const tokens = md.parse(body, env);
  const lineStarts = [0];
  for (let i = 0; i < body.length; i++) if (body.charCodeAt(i) === 10) lineStarts.push(i + 1);
  lineStarts.push(body.length);
  const at = (line: number) => lineStarts[Math.min(line, lineStarts.length - 1)]!;

  const blocks: Block[] = [];
  for (const token of tokens) {
    if (token.level !== 0 || token.nesting < 0 || !token.map) continue;
    let start = at(token.map[0]);
    let end = Math.max(start, at(token.map[1]));
    while (end > start && /\s/.test(body[end - 1]!)) end--;
    // A block never starts with a blank line, but be safe about leading newlines.
    while (start < end && body[start] === "\n") start++;
    if (end === start) continue;
    const previous = blocks[blocks.length - 1];
    if (previous && start < previous.end) continue;
    blocks.push({
      start,
      end,
      type: token.type,
      lang: token.type === "fence" ? (token.info.trim().split(/\s+/)[0] ?? "").toLowerCase() : "",
      content: token.type === "fence" || token.type === "code_block" ? token.content : "",
    });
  }
  return { blocks, references: env.references };
}

/** The block structure of a piece of Markdown, ignoring its text. Equal shapes draw alike. */
export function shape(md: MarkdownIt, source: string, references?: References): string {
  return shapeOf(md.parse(source, { references }));
}

function shapeOf(tokens: Token[]): string {
  return tokens
    .map((t) => (t.type === "inline" ? "i" : `${t.type}/${t.tag}/${t.level}${t.type === "fence" ? "/" + clean(t.info) : ""}`))
    .join(" ");
}

export type MarkKind = "quote" | "del" | "ins" | "landed";

export interface Mark {
  kind: MarkKind;
  /** The item the mark belongs to. */
  id: string;
  /** An extra class, such as the colour slot of a suggestion. */
  cls?: string;
  /** String indices into the source being marked. An `ins` mark has `start === end`. */
  start: number;
  end: number;
  /** The text an `ins` mark adds. */
  text?: string;
  /** A mark that must survive. If it cannot, `renderMarked` gives up and returns null. */
  required?: boolean;
}

const BASE = 0xe000;
const LIMIT = 0xf8ff;
const MARKERS = /[-]/g;

/** Removes marker characters. */
export function clean(text: string): string {
  return text.replace(MARKERS, "");
}

export function isMarker(code: number): boolean {
  return code >= BASE && code <= LIMIT;
}

/** Which mark a marker character belongs to, and whether it opens or closes it. */
export function decodeMarker(code: number): { index: number; open: boolean } {
  return { index: (code - BASE) >> 1, open: ((code - BASE) & 1) === 0 };
}

const BLOCK_PREFIX = /^[ \t]*(?:(?:#{1,6}[ \t]+|>[ \t]?|[-*+][ \t]+|\d{1,9}[.)][ \t]+))*/;

/**
 * Moves the ends of a highlight off the places where a stray character would
 * change the block: before a heading's `#`, a list marker or a `>`, and at
 * the start of the line after the highlighted text.
 */
export function settle(source: string, start: number, end: number, code: boolean): [number, number] {
  while (end > start && source[end - 1] === "\n") end--;
  while (start < end && source[start] === "\n") start++;
  if (!code) {
    const lineStart = source.lastIndexOf("\n", start - 1) + 1;
    if (/^[ \t]*$/.test(source.slice(lineStart, start))) {
      const lineEnd = source.indexOf("\n", start);
      const line = source.slice(lineStart, lineEnd < 0 ? source.length : lineEnd);
      const skip = lineStart + BLOCK_PREFIX.exec(line)![0].length;
      if (skip > start && skip < end) start = skip;
    }
  }
  return [start, end];
}

/** The source with a marker pair around each mark, and `ins` text added. */
export function insertMarkers(source: string, marks: readonly Mark[], code: boolean): string {
  interface Insertion {
    at: number;
    order: number;
    text: string;
  }
  const insertions: Insertion[] = [];
  marks.forEach((mark, index) => {
    const open = String.fromCharCode(BASE + 2 * index);
    const close = String.fromCharCode(BASE + 2 * index + 1);
    if (mark.kind === "ins") {
      insertions.push({ at: mark.start, order: 1, text: open + (mark.text ?? "") + close });
      return;
    }
    const [start, end] = mark.kind === "del" ? [mark.start, mark.end] : settle(source, mark.start, mark.end, code);
    if (end <= start) return;
    insertions.push({ at: start, order: 2, text: open });
    insertions.push({ at: end, order: 0, text: close });
  });
  insertions.sort((a, b) => a.at - b.at || a.order - b.order);
  let out = "";
  let from = 0;
  for (const insertion of insertions) {
    out += source.slice(from, insertion.at) + insertion.text;
    from = insertion.at;
  }
  return out + source.slice(from);
}

export interface Marked {
  /** Rendered HTML. Marker characters are still in its text. */
  html: string;
  /** The marks that survived, by marker index. A dropped mark leaves a hole. */
  marks: (Mark | undefined)[];
  dropped: Mark[];
}

/**
 * Renders one piece of Markdown with marks in it.
 *
 * Returns null when a required mark cannot be carried, or when the marks
 * change the block structure. Marks that are not required are dropped instead.
 */
export function renderMarked(
  md: MarkdownIt,
  source: string,
  marks: readonly Mark[],
  options: { references?: References; code?: boolean; expectShape?: string } = {},
): Marked | null {
  if (marks.length * 2 > LIMIT - BASE) marks = marks.slice(0, (LIMIT - BASE) >> 1);
  const plain = clean(source);
  const expected = options.expectShape ?? shape(md, plain, options.references);
  let live: (Mark | undefined)[] = [...marks];
  for (let attempt = 0; attempt < 3; attempt++) {
    // Keep each mark at its own index, so that marker characters stay stable.
    const active = live.map((mark) => mark ?? ({ kind: "quote", id: "", start: 0, end: 0 } as Mark));
    const tokens = md.parse(insertMarkers(plain, active, options.code ?? false), { references: options.references });
    const seen = markersInText(tokens);
    const missing = live.filter((mark, i) => mark && !(seen.has(BASE + 2 * i) && seen.has(BASE + 2 * i + 1))) as Mark[];
    const intact = shapeOf(tokens) === expected;
    if (intact && missing.length === 0) {
      const html = md.renderer.render(tokens, md.options, { references: options.references });
      return { html, marks: live, dropped: marks.filter((mark) => !live.includes(mark)) };
    }
    if (missing.some((mark) => mark.required)) return null;
    if (missing.length > 0) {
      live = live.map((mark) => (mark && missing.includes(mark) ? undefined : mark));
    } else {
      // The structure broke and every marker is still in the text. Give up on
      // the marks that may be dropped.
      if (live.every((mark) => !mark || mark.required)) return null;
      live = live.map((mark) => (mark && mark.required ? mark : undefined));
    }
  }
  return null;
}

/** Marker characters that will end up in text nodes, where they can be found again. */
function markersInText(tokens: Token[]): Set<number> {
  const seen = new Set<number>();
  const scan = (text: string) => {
    for (let i = 0; i < text.length; i++) {
      const code = text.charCodeAt(i);
      if (isMarker(code)) seen.add(code);
    }
  };
  for (const token of tokens) {
    if (token.type === "fence" || token.type === "code_block") scan(token.content);
    for (const child of token.children ?? []) {
      if (child.type === "text" || child.type === "code_inline") scan(child.content);
    }
  }
  return seen;
}

export interface EditSpan {
  id: string;
  cls?: string;
  /** String indices into the source the marks are for. */
  start: number;
  end: number;
  with: string;
}

/** The marks that draw an edit in place: deleted words struck, added words inserted. */
export function editMarks(source: string, edit: EditSpan): Mark[] {
  const marks: Mark[] = [];
  let at = edit.start;
  for (const segment of wordDiff(source.slice(edit.start, edit.end), edit.with)) {
    if (segment.op === "equal") {
      at += segment.text.length;
    } else if (segment.op === "delete") {
      marks.push({ kind: "del", id: edit.id, cls: edit.cls, start: at, end: at + segment.text.length, required: true });
      at += segment.text.length;
    } else {
      marks.push({ kind: "ins", id: edit.id, cls: edit.cls, start: at, end: at, text: segment.text, required: true });
    }
  }
  return marks;
}

/** `source` with edits applied. Edits are given in string indices and must not overlap. */
export function applyEdits(source: string, edits: readonly { start: number; end: number; with: string }[]): string {
  let out = "";
  let from = 0;
  for (const edit of [...edits].sort((a, b) => a.start - b.start)) {
    out += source.slice(from, edit.start) + edit.with;
    from = edit.end;
  }
  return out + source.slice(from);
}

/**
 * Where each character of rendered text sits in the source it was rendered
 * from. Rendered text is the source with its syntax taken out, so its
 * characters are found in order. Returns null when they are not, for example
 * when the source has an entity.
 *
 * `first[i]` is the earliest possible place of character `i`, and `last[i]`
 * the latest. A selection's start uses `last`, so that it does not take in
 * syntax before it, and its end uses `first`.
 */
export function alignText(text: string, source: string): { first: Int32Array; last: Int32Array } | null {
  const first = new Int32Array(text.length);
  const last = new Int32Array(text.length);
  let at = 0;
  for (let i = 0; i < text.length; i++) {
    at = source.indexOf(text[i]!, at);
    if (at < 0) return null;
    first[i] = at++;
  }
  at = source.length - 1;
  for (let i = text.length - 1; i >= 0; i--) {
    at = source.lastIndexOf(text[i]!, at);
    if (at < 0) return null;
    last[i] = at--;
  }
  return { first, last };
}
