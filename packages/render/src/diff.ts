// Word and line diffs. Nothing here knows about Endmark or the DOM.

export type Op = "equal" | "delete" | "insert";

export interface Segment {
  op: Op;
  text: string;
}

/** Above this many cells the table is skipped and the middle is one change. */
const MAX_CELLS = 4_000_000;

/**
 * A shortest edit script between two token lists, as runs of tokens.
 * Deletions come before insertions where both happen at one place.
 */
export function diffTokens(a: readonly string[], b: readonly string[]): { op: Op; tokens: string[] }[] {
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail++;
  const x = a.slice(head, a.length - tail);
  const y = b.slice(head, b.length - tail);

  const out: { op: Op; tokens: string[] }[] = [];
  const push = (op: Op, token: string) => {
    const last = out[out.length - 1];
    if (last && last.op === op) last.tokens.push(token);
    else out.push({ op, tokens: [token] });
  };
  for (let i = 0; i < head; i++) push("equal", a[i]!);

  if (x.length * y.length > MAX_CELLS) {
    for (const token of x) push("delete", token);
    for (const token of y) push("insert", token);
  } else if (x.length || y.length) {
    // lcs[i][j] is the length of the longest common subsequence of x[i..] and y[j..].
    const width = y.length + 1;
    const lcs = new Uint32Array((x.length + 1) * width);
    for (let i = x.length - 1; i >= 0; i--) {
      for (let j = y.length - 1; j >= 0; j--) {
        lcs[i * width + j] =
          x[i] === y[j] ? lcs[(i + 1) * width + j + 1]! + 1 : Math.max(lcs[(i + 1) * width + j]!, lcs[i * width + j + 1]!);
      }
    }
    let i = 0;
    let j = 0;
    // Collect each changed stretch so that its deletions precede its insertions.
    let removed: string[] = [];
    let added: string[] = [];
    const flush = () => {
      for (const token of removed) push("delete", token);
      for (const token of added) push("insert", token);
      removed = [];
      added = [];
    };
    while (i < x.length || j < y.length) {
      if (i < x.length && j < y.length && x[i] === y[j]) {
        flush();
        push("equal", x[i]!);
        i++;
        j++;
      } else if (i < x.length && (j === y.length || lcs[(i + 1) * width + j]! >= lcs[i * width + j + 1]!)) {
        removed.push(x[i++]!);
      } else {
        added.push(y[j++]!);
      }
    }
    flush();
  }
  for (let i = a.length - tail; i < a.length; i++) push("equal", a[i]!);
  return out;
}

/** Splits text into words, runs of white space, and single other characters. */
export function words(text: string): string[] {
  return text.match(/\s+|[\p{L}\p{N}_]+|[^\s\p{L}\p{N}_]/gu) ?? [];
}

/** A word-level diff. Joining the equal and delete texts gives `before`; equal and insert give `after`. */
export function wordDiff(before: string, after: string): Segment[] {
  return diffTokens(words(before), words(after)).map((run) => ({ op: run.op, text: run.tokens.join("") }));
}

export interface LineChange {
  op: Op;
  line: string;
}

/** A line-level diff. A final newline does not make an extra empty line. */
export function lineDiff(before: string, after: string): LineChange[] {
  const out: LineChange[] = [];
  for (const run of diffTokens(lines(before), lines(after))) {
    for (const line of run.tokens) out.push({ op: run.op, line });
  }
  return out;
}

/** The 0-based lines of `after` that are not in `before`. */
export function changedLines(before: string, after: string): number[] {
  const out: number[] = [];
  let n = 0;
  for (const change of lineDiff(before, after)) {
    if (change.op === "insert") out.push(n);
    if (change.op !== "delete") n++;
  }
  return out;
}

function lines(text: string): string[] {
  if (text === "") return [];
  const all = text.split("\n");
  if (all[all.length - 1] === "") all.pop();
  return all;
}
