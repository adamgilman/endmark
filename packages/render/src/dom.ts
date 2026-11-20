// Small DOM helpers. Only this file and render.ts touch the DOM.

import { alignText, decodeMarker, isMarker, type Mark } from "./blocks";

type Child = Node | string | null | undefined | false;

/** Creates an element. `props` are attributes, except `on*` listeners and `dataset`. */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Record<string, unknown> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === null || value === false) continue;
    if (key.startsWith("on") && typeof value === "function") {
      element.addEventListener(key.slice(2), value as EventListener);
    } else if (key === "dataset") {
      Object.assign(element.dataset, value);
    } else {
      element.setAttribute(key, value === true ? "" : String(value));
    }
  }
  for (const child of children) if (child) element.append(child);
  return element;
}

const TAGS = { quote: "mark", landed: "mark", del: "del", ins: "ins" } as const;

/**
 * Turns the marker characters left by `renderMarked` into elements. Each
 * stretch of text inside a mark is wrapped on its own, so a mark may cross
 * element boundaries.
 */
export function wrapMarks(root: HTMLElement, marks: readonly (Mark | undefined)[]): void {
  if (!marks.some(Boolean)) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes: Text[] = [];
  while (walker.nextNode()) nodes.push(walker.currentNode as Text);
  const active = new Set<number>();
  for (const node of nodes) {
    const data = node.data;
    let hasMarker = false;
    for (let i = 0; i < data.length && !hasMarker; i++) hasMarker = isMarker(data.charCodeAt(i));
    if (!hasMarker && active.size === 0) continue;
    const inPre = !!node.parentElement?.closest("pre");
    const pieces: Node[] = [];
    let from = 0;
    const flush = (to: number) => {
      const text = data.slice(from, to);
      if (!text) return;
      let piece: Node = document.createTextNode(text);
      // White space between block elements is not content. Leave it bare.
      if (active.size > 0 && (inPre || !/^\s*\n\s*$/.test(text))) {
        for (const index of [...active].sort((a, b) => b - a)) {
          const mark = marks[index];
          if (!mark) continue;
          piece = h(
            TAGS[mark.kind],
            { class: `em-${mark.kind}${mark.cls ? " " + mark.cls : ""}`, dataset: { id: mark.id } },
            piece,
          );
        }
      }
      pieces.push(piece);
    };
    for (let i = 0; i < data.length; i++) {
      const code = data.charCodeAt(i);
      if (!isMarker(code)) continue;
      flush(i);
      from = i + 1;
      const { index, open } = decodeMarker(code);
      if (open) active.add(index);
      else active.delete(index);
    }
    flush(data.length);
    node.replaceWith(...pieces);
  }
}

/** The text of a rendered block that came from the body: not suggested text, not layout white space. */
function sourceTextNodes(root: HTMLElement): Text[] {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const parent = (node as Text).parentElement;
      if (parent?.closest("ins.em-ins")) return NodeFilter.FILTER_REJECT;
      if (/^\s*\n\s*$/.test((node as Text).data) && !parent?.closest("pre")) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  const nodes: Text[] = [];
  while (walker.nextNode()) nodes.push(walker.currentNode as Text);
  return nodes;
}

/**
 * The place in a block's source of a point in its rendered form.
 * `side` says which end of a selection the point is. Returns null when the
 * rendered text cannot be lined up with the source.
 */
export function sourceOffset(
  block: HTMLElement,
  source: string,
  node: Node,
  offset: number,
  side: "start" | "end",
): number | null {
  const nodes = sourceTextNodes(block);
  const text = nodes.map((n) => n.data).join("");
  const point = document.createRange();
  point.setStart(block, 0);
  point.setEnd(node, offset);
  let at = 0;
  for (const n of nodes) {
    if (n === node) {
      at += offset;
      break;
    }
    if (point.comparePoint(n, n.data.length) > 0) break;
    at += n.data.length;
  }
  const aligned = alignText(text, source);
  if (!aligned) return null;
  if (side === "start") {
    // Skip white space the selection starts with.
    while (at < text.length && /\s/.test(text[at]!)) at++;
    return at < text.length ? aligned.last[at]! : null;
  }
  while (at > 0 && /\s/.test(text[at - 1]!)) at--;
  return at > 0 ? aligned.first[at - 1]! + 1 : null;
}
