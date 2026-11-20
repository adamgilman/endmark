// Arranges what the core reported for drawing. No DOM, and no format logic:
// every state and range used here was computed by the core.

import type { Block } from "./blocks";
import type { Item } from "./types";

/** One edit of an open, applicable suggestion, in string indices of the body. */
export interface DrawEdit {
  sid: string;
  /** The colour slot of the suggestion. */
  slot: number;
  /** 0-based position among the suggestion's edits. */
  n: number;
  start: number;
  end: number;
  replace: string;
  with: string;
}

/** One or more neighbouring blocks that are drawn together because an edit spans them. */
export interface Unit {
  start: number;
  end: number;
  blocks: Block[];
  edits: DrawEdit[];
}

export interface Units {
  units: Unit[];
  /** Edits that touch no block, such as text appended to the end of the body. */
  loose: DrawEdit[];
}

export function buildUnits(blocks: readonly Block[], edits: readonly DrawEdit[]): Units {
  const units: Unit[] = blocks.map((block) => ({ start: block.start, end: block.end, blocks: [block], edits: [] }));
  const loose: DrawEdit[] = [];
  for (const edit of edits) {
    let first = -1;
    let last = -1;
    if (edit.end > edit.start) {
      units.forEach((unit, i) => {
        if (unit.start <= edit.end && unit.end >= edit.start) {
          if (first < 0) first = i;
          last = i;
        }
      });
    }
    if (first < 0) {
      loose.push(edit);
      continue;
    }
    const group = units.slice(first, last + 1);
    const merged: Unit = {
      start: Math.min(group[0]!.start, edit.start),
      end: Math.max(group[group.length - 1]!.end, edit.end),
      blocks: group.flatMap((unit) => unit.blocks),
      edits: [...group.flatMap((unit) => unit.edits), edit],
    };
    units.splice(first, group.length, merged);
  }
  return { units, loose };
}

/** The unit a position belongs to: the last one that starts at or before it. -1 when there are no units. */
export function unitAt(units: readonly Unit[], position: number): number {
  let found = units.length ? 0 : -1;
  units.forEach((unit, i) => {
    if (unit.start <= position) found = i;
  });
  return found;
}

export function overlaps(edits: readonly DrawEdit[]): boolean {
  return edits.some((a, i) => edits.some((b, j) => j > i && a.start < b.end && b.start < a.end));
}

export interface Thread {
  root: Item;
  /** The root, then its replies in file order. */
  comments: Item[];
  /** How deep each comment sits under the root. The root is 0. */
  depth: Map<string, number>;
}

export function buildThreads(items: readonly Item[]): Thread[] {
  const threads = new Map<string, Thread>();
  for (const item of items) {
    if (item.kind !== "comment") continue;
    if (item.root === undefined || item.root === null) {
      threads.set(item.id, { root: item, comments: [item], depth: new Map([[item.id, 0]]) });
    } else {
      const thread = threads.get(item.root);
      if (!thread) continue;
      thread.comments.push(item);
      thread.depth.set(item.id, (thread.depth.get(item.re ?? "") ?? 0) + 1);
    }
  }
  return [...threads.values()];
}

/** A thread someone still has to deal with: not resolved, and not withdrawn in full. */
export function isOpenThread(thread: Thread): boolean {
  return thread.root.thread === "open" && thread.comments.some((comment) => comment.state === "live");
}

export function isChange(item: Item): boolean {
  return item.kind === "edit" || item.kind === "revert" || (item.kind === "suggestion" && item.state === "accepted");
}

/** The core reports a change that is not in effect with the status `reverted`. */
export function inEffect(item: Item): boolean {
  return isChange(item) && item.status !== "reverted";
}

/** The reverts that follow a change: the one that names it, the one that names that, and so on. */
export function revertChain(items: readonly Item[], item: Item): Item[] {
  const chain: Item[] = [];
  let at = item;
  while (at.reverted_by) {
    const next = items.find((other) => other.id === at.reverted_by);
    if (!next || chain.includes(next)) break;
    chain.push(next);
    at = next;
  }
  return chain;
}

/** When an item was last acted on, for ordering History. */
export function lastTime(item: Item): number {
  let latest = Date.parse(item.at) || 0;
  for (const event of item.events) latest = Math.max(latest, Date.parse(event.at) || 0);
  return latest;
}

export interface Closed {
  item: Item;
  thread?: Thread;
  time: number;
}

/** Closed items, newest first: changes, declined suggestions, and threads that are resolved or withdrawn. */
export function closedItems(items: readonly Item[], threads: readonly Thread[]): Closed[] {
  const closed: Closed[] = [];
  for (const item of items) {
    if (item.kind === "comment") continue;
    if (item.kind === "suggestion" && item.state === "open") continue;
    closed.push({ item, time: lastTime(item) });
  }
  for (const thread of threads) {
    if (isOpenThread(thread)) continue;
    closed.push({ item: thread.root, thread, time: Math.max(...thread.comments.map(lastTime)) });
  }
  return closed.sort((a, b) => b.time - a.time || b.item.line - a.item.line);
}
