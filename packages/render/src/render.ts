// The reference renderer: RFC 0001, section 11.
//
// It draws a source string into a container, in one of four views. It reads
// through the core and writes nothing: every action is handed to a callback.

import type { MarkdownIt } from "markdown-it";
import {
  applyEdits,
  createMarkdown,
  editMarks,
  renderMarked,
  shape,
  splitBlocks,
  type Block,
  type Mark,
  type Parsed,
  type References,
} from "./blocks";
import { ByteMap } from "./bytes";
import type { Core } from "./core";
import { lineDiff, wordDiff } from "./diff";
import { h, sourceOffset, wrapMarks } from "./dom";
import { randomId } from "./ids";
import { configureMermaid, loadMermaid, type MermaidApi } from "./mermaid";
import {
  buildThreads,
  buildUnits,
  closedItems,
  inEffect,
  isChange,
  isOpenThread,
  overlaps,
  revertChain,
  unitAt,
  type Closed,
  type DrawEdit,
  type Thread,
  type Unit,
} from "./model";
import type { DocumentJson, Item, PreviewOk, StatusOk, WriteOk } from "./types";

export type View = "final" | "review" | "preview" | "history";

/** Text the reader selected in Review view, as source. */
export interface SelectionInput {
  /** The selected source text. */
  quote: string;
  /** Wider pieces of source around it, narrowest first, for use as `in` when the quote is not unique. */
  scopes: string[];
  note: string;
}

/**
 * What an action may return: nothing, its outcome, or a promise of either.
 * A host that writes somewhere else, such as a server, returns a promise.
 * A form stays open, with what was typed in it, when the outcome has
 * `ok: false`. It closes on anything else.
 */
export type ActionResult = void | { ok: boolean } | Promise<void | { ok: boolean }>;

/**
 * What the reader asked for. The renderer calls these and changes nothing
 * itself. An action that is absent is not offered.
 */
export interface Actions {
  comment?(input: SelectionInput): ActionResult;
  suggest?(input: SelectionInput & { replacement: string }): ActionResult;
  reply?(id: string, note: string): ActionResult;
  accept?(id: string): ActionResult;
  reject?(id: string): ActionResult;
  retract?(id: string): ActionResult;
  resolve?(id: string): ActionResult;
  reopen?(id: string): ActionResult;
  /** Undo a change that is in effect. The core follows the chain to its tip. */
  revert?(id: string): ActionResult;
  /** Bring back a change that is not in effect, by reverting `tip`, the last revert of its chain. */
  restore?(id: string, tip: string): ActionResult;
}

export interface RenderOptions {
  core: Core;
  /** The view to start in. Default: Review when open records exist, else Final. */
  view?: View;
  /** The reader's handle. Retract is offered only on their own items. */
  me?: string;
  actions?: Actions;
  /** Draw images in the body. Off, nothing is fetched for them. Default true. */
  images?: boolean;
  /** How to get mermaid. Default: a dynamic import. */
  loadMermaid?: () => Promise<MermaidApi>;
  onViewChange?(view: View): void;
}

export interface Renderer {
  readonly view: View;
  /**
   * Draws a new source. A form the reader has open, such as a half-typed
   * reply, is kept with its text, so a host may call this at any time.
   */
  update(source: string): void;
  /**
   * Draws the same source again, keeping open forms. A host calls it after
   * changing the page's theme, so that diagrams are drawn in the new colours.
   */
  redraw(): void;
  setView(view: View): void;
  destroy(): void;
}

/** Draws `source` into `container` and returns a handle to update it. */
export function mount(container: HTMLElement, source: string, options: RenderOptions): Renderer {
  return new EndmarkRenderer(container, source, options);
}

const SLOTS = 4;
const VIEWS: [View, string][] = [
  ["final", "Final"],
  ["review", "Review"],
  ["preview", "Preview"],
  ["history", "History"],
];

interface DiagramJob {
  host: HTMLElement;
  code: string;
  /** Called with the parse error, or null when the diagram drew. */
  done?: (error: string | null) => void;
}

interface AcceptGate {
  button: HTMLButtonElement | null;
  notice: HTMLElement | null;
  pending: number;
  errors: string[];
  confirmed: boolean;
}

interface Highlight {
  id: string;
  start: number;
  end: number;
}

/** What `update` carries over a redraw. */
interface KeptForms {
  popover: HTMLElement | null;
  replies: HTMLFormElement[];
  focused: { element: HTMLTextAreaElement; start: number; end: number } | null;
}

let diagramCounter = 0;

class EndmarkRenderer implements Renderer {
  view: View;
  private source: string;
  private readonly root: HTMLElement;
  private readonly md: MarkdownIt;
  private readonly actions: Actions;
  private previewId: string | null = null;
  private selected: string | null = null;
  private generation = 0;
  private diagrams: DiagramJob[] = [];
  private compares: { current: HTMLElement; suggested: HTMLElement[] }[] = [];
  private gates = new Map<string, AcceptGate>();
  private sources = new WeakMap<HTMLElement, { start: number; source: string }>();
  private body = "";
  private parsed: Parsed = { blocks: [], references: undefined };
  private popover: HTMLElement | null = null;
  private readonly onSelection = () => this.selectionChanged();
  private selectionTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private readonly container: HTMLElement,
    source: string,
    private readonly options: RenderOptions,
  ) {
    this.source = source;
    this.md = createMarkdown({ images: options.images });
    this.actions = options.actions ?? {};
    this.root = h("div", { class: "em-root" });
    container.replaceChildren(this.root);
    const doc = this.read();
    const open =
      !!doc &&
      (doc.items.some((item) => item.kind === "suggestion" && item.state === "open") ||
        buildThreads(doc.items).some(isOpenThread));
    this.view = options.view ?? (open ? "review" : "final");
    this.root.addEventListener("click", (event) => this.clicked(event));
    document.addEventListener("selectionchange", this.onSelection);
    this.draw();
  }

  update(source: string): void {
    const kept = this.openForms();
    this.source = source;
    this.draw();
    this.restoreForms(kept);
  }

  redraw(): void {
    this.update(this.source);
  }

  setView(view: View): void {
    if (view === this.view) return;
    this.view = view;
    this.options.onViewChange?.(view);
    this.draw();
  }

  destroy(): void {
    this.generation++;
    document.removeEventListener("selectionchange", this.onSelection);
    this.root.remove();
  }

  // Forms the reader has open. A redraw for a new source must not lose them.

  private openForms(): KeptForms {
    const popover = this.popover?.querySelector("form") ? this.popover : null;
    const replies = [...this.root.querySelectorAll<HTMLFormElement>("form.em-form[data-for]")];
    const active = document.activeElement;
    const focused =
      active instanceof HTMLTextAreaElement && (popover?.contains(active) || replies.some((form) => form.contains(active)))
        ? { element: active, start: active.selectionStart, end: active.selectionEnd }
        : null;
    return { popover, replies, focused };
  }

  private restoreForms(kept: KeptForms): void {
    const orphans: HTMLElement[] = [];
    for (const form of kept.replies) {
      const id = form.dataset.for!;
      const card = [...this.root.querySelectorAll<HTMLElement>(`.em-card[data-id="${cssEscape(id)}"]`)].find(
        (candidate) => !candidate.querySelector(":scope > .em-form"),
      );
      if (card) card.append(form);
      else orphans.push(form);
    }
    if (orphans.length > 0) {
      // The item a reply was being written to is not drawn any more. Keep the text in sight.
      const section = h(
        "section",
        { class: "em-drafts", "aria-label": "Unsent replies" },
        h("h2", { class: "em-label" }, "Unsent replies"),
      );
      for (const form of orphans) {
        section.append(
          h("p", { class: "em-hint" }, "The item ", h("code", {}, form.dataset.for!), " is no longer shown in this view."),
          form,
        );
      }
      this.root.querySelector(".em-view")?.prepend(section);
    }
    if (kept.popover) {
      this.root.append(kept.popover);
      this.popover = kept.popover;
    }
    if (kept.focused && kept.focused.element.isConnected) {
      kept.focused.element.focus({ preventScroll: true });
      kept.focused.element.setSelectionRange(kept.focused.start, kept.focused.end);
    }
  }

  /**
   * Runs the action of a form. The form closes when the action is done, and
   * stays as it is when the action was refused.
   */
  private submit(form: HTMLFormElement, run: () => ActionResult, close: () => void): void {
    if (form.dataset.busy) return;
    const finish = (outcome: void | { ok: boolean }) => {
      delete form.dataset.busy;
      for (const button of form.querySelectorAll("button")) button.disabled = false;
      if (!outcome || outcome.ok) close();
    };
    form.dataset.busy = "true";
    let result: ActionResult;
    try {
      result = run();
    } catch (error) {
      finish({ ok: false });
      throw error;
    }
    if (result instanceof Promise) {
      for (const button of form.querySelectorAll("button")) button.disabled = true;
      result.then(finish, () => finish({ ok: false }));
    } else {
      finish(result);
    }
  }

  // Reading.

  private read(): DocumentJson | null {
    const response = this.options.core.call<StatusOk>({ op: "status", source: this.source });
    return response.ok ? response.document : null;
  }

  // Drawing.

  private draw(): void {
    this.generation++;
    this.diagrams = [];
    this.compares = [];
    this.gates = new Map();
    this.popover = null;
    const doc = this.read();
    if (!doc) {
      this.root.replaceChildren(h("p", { class: "em-error" }, "The file could not be read."));
      return;
    }
    this.body = doc.body;
    this.parsed = splitBlocks(this.md, doc.body);
    const threads = buildThreads(doc.items);
    const openCount =
      threads.filter(isOpenThread).length +
      doc.items.filter((item) => item.kind === "suggestion" && item.state === "open").length;

    this.root.dataset.view = this.view;
    const tabs = h("div", { class: "em-tabs", role: "tablist", "aria-label": "View" });
    for (const [view, label] of VIEWS) {
      tabs.append(
        h(
          "button",
          {
            type: "button",
            role: "tab",
            class: "em-tab",
            "aria-selected": String(view === this.view),
            dataset: { view },
            onclick: () => this.setView(view),
          },
          label,
          view === "review" && openCount > 0 ? h("span", { class: "em-count" }, String(openCount)) : null,
        ),
      );
    }
    const content = h("div", { class: "em-view", role: "tabpanel" });
    this.root.replaceChildren(tabs, content);

    const problems = this.view === "final" ? null : this.problems(doc);
    if (problems) content.append(problems);
    if (this.view === "final") content.append(this.plainBody(doc.body, this.parsed));
    else if (this.view === "review") this.drawReview(content, doc, threads);
    else if (this.view === "preview") this.drawPreview(content, doc);
    else this.drawHistory(content, doc, threads);

    this.runDiagrams();
  }

  /** Malformed and ignored records, as the core's linter reports them. */
  private problems(doc: DocumentJson): Node | null {
    if (doc.diagnostics.length === 0) return null;
    const list = h("ul", {});
    for (const finding of doc.diagnostics) {
      list.append(
        h(
          "li",
          { class: `em-finding em-${finding.severity}` },
          h("span", { class: "em-chip" }, finding.severity),
          ` Line ${finding.line}: ${finding.message} `,
          h("code", {}, finding.code),
          finding.hint ? h("span", { class: "em-hint" }, ` ${capital(finding.hint)}.`) : null,
        ),
      );
    }
    return h(
      "section",
      { class: "em-problems", "aria-label": "Problems in the file" },
      h("h2", { class: "em-label" }, `Problems in the file (${doc.diagnostics.length})`),
      list,
    );
  }

  /** A body with nothing layered on it, or with highlights only. */
  private plainBody(body: string, parsed: Parsed, highlights: Highlight[] = [], kind: Mark["kind"] = "quote"): HTMLElement {
    const out = h("div", { class: "em-doc em-plain" });
    for (const block of parsed.blocks) {
      const unit: Unit = { start: block.start, end: block.end, blocks: [block], edits: [] };
      out.append(h("div", { class: "em-row" }, h("div", { class: "em-main" }, this.unitPlain(body, parsed, unit, highlights, kind))));
    }
    if (parsed.blocks.length === 0) out.append(h("p", { class: "em-empty" }, "The body is empty."));
    return out;
  }

  private unitPlain(body: string, parsed: Parsed, unit: Unit, highlights: Highlight[], kind: Mark["kind"] = "quote"): HTMLElement {
    const block = unit.blocks.length === 1 ? unit.blocks[0]! : null;
    if (block && block.lang === "mermaid") {
      // Text inside a picture cannot be marked, so the whole picture is.
      const host = this.diagram(block.content);
      const inside = this.clip(unit, highlights, kind);
      if (inside.length > 0) {
        host.classList.add(`em-${kind}-block`);
        host.dataset.id = inside[0]!.id;
      }
      return host;
    }
    return this.sourceBlock(body, parsed, unit, this.clip(unit, highlights, kind), isCode(block));
  }

  private clip(unit: Unit, highlights: Highlight[], kind: Mark["kind"]): Mark[] {
    const marks: Mark[] = [];
    for (const highlight of highlights) {
      const start = Math.max(highlight.start, unit.start);
      const end = Math.min(highlight.end, unit.end);
      if (end > start) marks.push({ kind, id: highlight.id, start: start - unit.start, end: end - unit.start });
    }
    return marks;
  }

  /** Renders a unit's own source, with marks. Selections inside it map back to the body. */
  private sourceBlock(body: string, parsed: Parsed, unit: Unit, marks: Mark[], code: boolean): HTMLElement {
    const source = body.slice(unit.start, unit.end);
    const references = parsed.references;
    const marked =
      renderMarked(this.md, source, marks, { references, code }) ?? renderMarked(this.md, source, [], { references })!;
    const element = h("div", { class: "em-md em-src" });
    element.innerHTML = marked.html;
    wrapMarks(element, marked.marks);
    if (body === this.body) this.sources.set(element, { start: unit.start, source });
    return element;
  }

  private markdown(text: string, cls: string, references?: References): HTMLElement {
    const element = h("div", { class: `em-md ${cls}` });
    element.innerHTML = this.md.render(text, { references });
    return element;
  }

  // Review.

  private drawReview(content: HTMLElement, doc: DocumentJson, threads: Thread[]): void {
    const map = new ByteMap(doc.body);
    const openThreads = threads.filter(isOpenThread);
    const openSuggestions = doc.items.filter((item) => item.kind === "suggestion" && item.state === "open");
    const drawn = openSuggestions.filter((s) => s.status === "applicable" && (s.edits ?? []).every((e) => e.range));
    const stale = openSuggestions.filter((s) => !drawn.includes(s));

    const slots = new Map<string, number>();
    const edits: DrawEdit[] = [];
    drawn.forEach((suggestion, i) => {
      slots.set(suggestion.id, i % SLOTS);
      (suggestion.edits ?? []).forEach((edit, n) => {
        const [start, end] = map.range(edit.range)!;
        edits.push({ sid: suggestion.id, slot: i % SLOTS, n, start, end, replace: edit.replace, with: edit.with });
      });
    });
    const { units, loose } = buildUnits(this.parsed.blocks, edits);

    // Where each card goes: beside a unit, at the end for the whole document, or in the tray.
    const beside = new Map<number, Node[]>();
    const whole: Node[] = [];
    const tray: Node[] = [];
    const place = (position: number, card: Node) => {
      const index = unitAt(units, position);
      if (index < 0) whole.push(card);
      else beside.set(index, [...(beside.get(index) ?? []), card]);
    };

    // Remarks on a suggestion sit inside its card.
    const remarks = new Map<string, Thread[]>();
    const highlights: Highlight[] = [];
    const placed: Thread[] = [];
    for (const thread of openThreads) {
      const target = thread.root.re ? openSuggestions.find((s) => s.id === thread.root.re) : undefined;
      if (target && thread.root.anchor === "unquoted") {
        remarks.set(target.id, [...(remarks.get(target.id) ?? []), thread]);
        continue;
      }
      placed.push(thread);
      for (const comment of thread.comments) {
        const range = comment.state === "live" && comment.anchor === "attached" ? map.range(comment.range) : null;
        if (range) highlights.push({ id: thread.root.id, start: range[0], end: range[1] });
      }
    }

    for (const suggestion of drawn) {
      const first = Math.min(...edits.filter((e) => e.sid === suggestion.id).map((e) => e.start));
      place(first, this.suggestionCard(suggestion, slots.get(suggestion.id)!, remarks.get(suggestion.id) ?? [], false));
    }
    for (const thread of placed) {
      const anchor = thread.root.anchor;
      const range = anchor === "attached" ? map.range(thread.root.range) : null;
      if (range) place(range[0], this.threadCard(thread));
      else if (anchor === "detached" || anchor === "ambiguous") tray.push(this.threadCard(thread, true));
      else whole.push(this.threadCard(thread));
    }
    for (const suggestion of stale) tray.push(this.suggestionCard(suggestion, -1, remarks.get(suggestion.id) ?? [], true));

    const docEl = h("div", { class: "em-doc" });
    units.forEach((unit, i) => {
      const cards = beside.get(i) ?? [];
      const row = h("div", { class: "em-row" });
      const main = h("div", { class: "em-main" });
      if (unit.edits.length > 0) {
        row.classList.add("em-edited");
        const bars = h("div", { class: "em-bars", "aria-hidden": "true" });
        for (const sid of new Set(unit.edits.map((e) => e.sid))) {
          bars.append(h("span", { class: `em-bar em-s${slots.get(sid)}`, title: `Suggestion ${sid}`, dataset: { id: sid } }));
        }
        main.append(bars);
      }
      main.append(this.unitReview(doc.body, unit, highlights));
      for (const edit of loose) {
        if (unitAt(units, edit.start) === i) main.append(this.looseEdit(edit));
      }
      row.append(main);
      if (cards.length > 0) row.append(h("aside", { class: "em-side" }, ...cards));
      docEl.append(row);
    });
    if (units.length === 0) {
      docEl.append(h("p", { class: "em-empty" }, "The body is empty."));
      for (const edit of loose) docEl.append(this.looseEdit(edit));
    }
    content.append(docEl);

    if (whole.length > 0) {
      content.append(
        h(
          "section",
          { class: "em-whole", "aria-label": "Comments on the whole document" },
          h("h2", { class: "em-label" }, "On the whole document"),
          ...whole,
        ),
      );
    }
    if (tray.length > 0) {
      content.append(
        h(
          "section",
          { class: "em-tray", "aria-label": "Unanchored items" },
          h("h2", { class: "em-label" }, `Unanchored (${tray.length})`),
          h(
            "p",
            { class: "em-hint" },
            "The text these point at is no longer in the body exactly once. Each shows its original quote.",
          ),
          ...tray,
        ),
      );
    }
    if (openThreads.length + openSuggestions.length === 0) {
      content.append(h("p", { class: "em-empty" }, "Nothing is open. Select text to comment on it or to suggest a change."));
    }

    // A thread whose highlight could not be drawn shows its quote in the card.
    for (const card of content.querySelectorAll<HTMLElement>(".em-thread[data-id]")) {
      const id = card.dataset.id!;
      if (content.querySelector(`mark.em-quote[data-id="${cssEscape(id)}"]`)) continue;
      card.querySelector(".em-undrawn")?.removeAttribute("hidden");
    }
  }

  private unitReview(body: string, unit: Unit, highlights: Highlight[]): Node {
    const parsed = this.parsed;
    const block = unit.blocks.length === 1 ? unit.blocks[0]! : null;
    const quotes = this.clip(unit, highlights, "quote");
    if (unit.edits.length === 0) return this.unitPlain(body, parsed, unit, highlights);

    const source = body.slice(unit.start, unit.end);
    const references = parsed.references;
    const local = (edit: DrawEdit) => ({ start: edit.start - unit.start, end: edit.end - unit.start, with: edit.with });
    const bySuggestion = new Map<string, DrawEdit[]>();
    for (const edit of unit.edits) bySuggestion.set(edit.sid, [...(bySuggestion.get(edit.sid) ?? []), edit]);
    const versions = [...bySuggestion].map(([sid, list]) => ({
      sid,
      slot: list[0]!.slot,
      after: applyEdits(source, list.map(local)),
    }));

    // A block drawn as a picture: current and suggested, side by side.
    if (block && block.lang === "mermaid") {
      const after = versions.map((version) => {
        const blocks = splitBlocks(this.md, version.after).blocks;
        return blocks.length === 1 && blocks[0]!.lang === "mermaid" ? { ...version, code: blocks[0]!.content } : null;
      });
      if (after.every(Boolean)) return this.compare(block, after as { sid: string; slot: number; code: string }[]);
    }

    // Code shown as source: a line diff for each suggestion.
    if (isCode(block)) {
      const out = h("div", { class: "em-stack" });
      for (const version of versions) {
        out.append(
          h(
            "div",
            { class: `em-alt em-s${version.slot}`, dataset: { id: version.sid } },
            h("div", { class: "em-label" }, `Suggested by ${version.sid}`),
            sourceDiff(source, version.after),
          ),
        );
      }
      return out;
    }

    // A change inside a block's inline text: a word-level diff in place.
    const before = shape(this.md, source, references);
    if (!overlaps(unit.edits) && versions.every((v) => shape(this.md, v.after, references) === before)) {
      const marks: Mark[] = [...quotes];
      for (const edit of unit.edits) {
        marks.push(...editMarks(source, { id: edit.sid, cls: `em-s${edit.slot}`, ...local(edit) }));
      }
      const marked = renderMarked(this.md, source, marks, { references, expectShape: before });
      if (marked) {
        const element = h("div", { class: "em-md em-src" });
        element.innerHTML = marked.html;
        wrapMarks(element, marked.marks);
        this.sources.set(element, { start: unit.start, source });
        return element;
      }
    }

    // Anything larger: the old block, then each suggested block under it.
    const struck: Mark[] = unit.edits.map((edit) => ({
      kind: "del",
      id: edit.sid,
      cls: `em-s${edit.slot}`,
      start: edit.start - unit.start,
      end: edit.end - unit.start,
    }));
    const current = this.sourceBlock(body, parsed, unit, [...quotes, ...struck], false);
    const old = h("div", { class: "em-alt em-old" }, h("div", { class: "em-label" }, "Current"), current);
    if (!current.querySelector("del.em-del")) old.classList.add("em-struck");
    const out = h("div", { class: "em-stack" }, old);
    for (const version of versions) {
      out.append(
        h(
          "div",
          { class: `em-alt em-new em-s${version.slot}`, dataset: { id: version.sid } },
          h("div", { class: "em-label" }, `Suggested by ${version.sid}`),
          version.after.trim() === ""
            ? h("p", { class: "em-empty" }, "This text is removed.")
            : this.rendered(version.after, references),
        ),
      );
    }
    return out;
  }

  /** Markdown that is not the current body: diagrams are drawn, nothing maps back to the source. */
  private rendered(text: string, references?: References): HTMLElement {
    const out = h("div", { class: "em-md" });
    const parsed = splitBlocks(this.md, text);
    for (const block of parsed.blocks) {
      if (block.lang === "mermaid") out.append(this.diagram(block.content));
      else out.append(...this.markdown(text.slice(block.start, block.end), "", references ?? parsed.references).childNodes);
    }
    return out;
  }

  /** An edit that touches no block, such as text appended to the end of the body. */
  private looseEdit(edit: DrawEdit): HTMLElement {
    return h(
      "div",
      { class: `em-alt em-new em-s${edit.slot}`, dataset: { id: edit.sid } },
      h("div", { class: "em-label" }, edit.replace === "" ? `Added by ${edit.sid}` : `Changed by ${edit.sid}`),
      edit.replace.trim() === "" ? this.rendered(edit.with) : sourceDiff(edit.replace, edit.with),
    );
  }

  // Diagrams.

  private diagram(code: string, done?: DiagramJob["done"]): HTMLElement {
    const host = h("div", { class: "em-diagram" }, h("pre", { class: "em-diagram-source" }, h("code", {}, code)));
    this.diagrams.push({ host, code, done });
    return host;
  }

  private compare(block: Block, versions: { sid: string; slot: number; code: string }[]): HTMLElement {
    const current = this.diagram(block.content);
    const pictures = h(
      "div",
      { class: "em-pictures" },
      h("figure", { class: "em-figure" }, h("figcaption", { class: "em-label" }, "Current"), current),
    );
    const diffs = h("div", { class: "em-sourcediffs", hidden: true });
    const suggested: HTMLElement[] = [];
    for (const version of versions) {
      const gate = this.gate(version.sid);
      gate.pending++;
      const host = this.diagram(version.code, (error) => {
        gate.pending--;
        if (error) gate.errors.push(error);
        this.refreshGate(version.sid);
      });
      suggested.push(host);
      pictures.append(
        h(
          "figure",
          { class: `em-figure em-new em-s${version.slot}`, dataset: { id: version.sid } },
          h("figcaption", { class: "em-label" }, `Suggested by ${version.sid}`),
          host,
        ),
      );
      diffs.append(
        h(
          "div",
          { class: `em-alt em-s${version.slot}`, dataset: { id: version.sid } },
          h("div", { class: "em-label" }, `Source diff for ${version.sid}`),
          sourceDiff(block.content, version.code),
        ),
      );
    }
    this.compares.push({ current, suggested });
    const toggle = h(
      "button",
      {
        type: "button",
        class: "em-button em-toggle",
        "aria-pressed": "false",
        onclick: () => {
          const show = diffs.hidden;
          diffs.hidden = !show;
          pictures.hidden = show;
          toggle.setAttribute("aria-pressed", String(show));
          toggle.textContent = show ? "Show diagrams" : "Show source diff";
        },
      },
      "Show source diff",
    );
    return h("div", { class: "em-compare" }, h("div", { class: "em-compare-bar" }, toggle), pictures, diffs);
  }

  private runDiagrams(): void {
    const jobs = this.diagrams;
    const compares = this.compares;
    const generation = this.generation;
    if (jobs.length === 0) return;
    const load = this.options.loadMermaid ?? loadMermaid;
    void (async () => {
      let api: MermaidApi | null = null;
      let failure = "";
      try {
        api = await load();
        configureMermaid(api);
      } catch (error) {
        failure = `Mermaid could not be loaded: ${message(error)}`;
      }
      for (const job of jobs) {
        if (generation !== this.generation) return;
        let error: string | null = failure || null;
        if (api) {
          try {
            await api.parse(job.code);
            const { svg } = await api.render(`em-diagram-${++diagramCounter}`, job.code);
            if (generation !== this.generation) return;
            job.host.innerHTML = svg;
            job.host.classList.add("em-drawn");
          } catch (caught) {
            error = message(caught);
          }
        }
        if (generation !== this.generation) return;
        if (error) {
          job.host.classList.add("em-failed");
          job.host.prepend(
            h(
              "div",
              { class: "em-parse-error", role: "alert" },
              h("strong", {}, "This diagram does not parse."),
              h("pre", {}, error),
            ),
          );
        }
        job.done?.(error);
      }
      for (const group of compares) markAdded(group.current, group.suggested);
    })();
  }

  // Accept is held back until the suggested version of every picture has been parsed.

  private gate(sid: string): AcceptGate {
    let gate = this.gates.get(sid);
    if (!gate) {
      gate = { button: null, notice: null, pending: 0, errors: [], confirmed: false };
      this.gates.set(sid, gate);
    }
    return gate;
  }

  private refreshGate(sid: string): void {
    const gate = this.gate(sid);
    if (!gate.button) return;
    if (gate.pending > 0) {
      gate.button.disabled = true;
      gate.button.textContent = "Checking the diagram";
    } else if (gate.errors.length > 0 && !gate.confirmed) {
      gate.button.disabled = false;
      gate.button.textContent = "Accept anyway";
      gate.button.dataset.needsConfirm = "true";
      if (gate.notice) {
        gate.notice.hidden = false;
        gate.notice.textContent = "The suggested diagram does not parse. Accepting needs a second click to confirm.";
      }
    } else if (gate.errors.length > 0) {
      gate.button.textContent = "Confirm: accept a diagram that does not parse";
    } else {
      gate.button.disabled = false;
      gate.button.textContent = "Accept";
    }
  }

  // Cards.

  private byline(item: { by: string; for: string | null; at: string }): HTMLElement {
    const agent = item.by.startsWith("agent:");
    return h(
      "div",
      { class: "em-byline" },
      h("span", { class: `em-author${agent ? " em-agent" : ""}` }, item.by),
      item.for ? h("span", { class: "em-for" }, " for ", h("span", { class: "em-author" }, item.for)) : null,
      " ",
      h("time", { datetime: item.at }, shortTime(item.at)),
    );
  }

  private note(text: string): Node | null {
    return text.trim() ? this.markdown(text, "em-note") : null;
  }

  private button(label: string, run: (() => unknown) | undefined, cls = ""): HTMLButtonElement | null {
    if (!run) return null;
    const button = h(
      "button",
      {
        type: "button",
        class: `em-button ${cls}`.trim(),
        onclick: () => {
          // An action that is still on its way is not sent twice.
          const result = run();
          if (result instanceof Promise) {
            button.disabled = true;
            const done = () => void (button.disabled = false);
            result.then(done, done);
          }
        },
      },
      label,
    );
    return button;
  }

  private threadCard(thread: Thread, unanchored = false): HTMLElement {
    const root = thread.root;
    const card = h("section", { class: "em-card em-thread", dataset: { id: root.id }, tabindex: "0" });
    const resolved = root.thread === "resolved";
    card.append(
      h(
        "header",
        { class: "em-card-head" },
        h("span", { class: "em-kind" }, "Comment"),
        h("code", { class: "em-id" }, root.id),
        root.re && root.root === undefined ? h("span", { class: "em-chip" }, `on ${root.re}`) : null,
        root.anchor === "detached" || root.anchor === "ambiguous"
          ? h("span", { class: "em-chip em-warn" }, root.anchor)
          : null,
        resolved ? h("span", { class: "em-chip" }, "resolved") : null,
      ),
    );
    for (const comment of thread.comments) {
      const depth = Math.min(thread.depth.get(comment.id) ?? 0, 3);
      const retracted = comment.state === "retracted";
      const quote = comment.quote;
      const mine = comment.by === this.options.me;
      const own = comment === root;
      const body = h(
        "div",
        { class: `em-comment${depth ? " em-reply" : ""}${retracted ? " em-retracted" : ""}`, dataset: { depth: String(depth) } },
        this.byline(comment),
      );
      if (!own) body.firstElementChild!.append(" ", h("code", { class: "em-id" }, comment.id));
      if (quote && (unanchored || comment.anchor !== "attached" || !own)) {
        body.append(quoteBlock(quote.on, quote.in, comment.anchor, comment.events.some((e) => e.verb === "anchor")));
      } else if (quote) {
        // Shown only when the highlight could not be drawn in the body.
        const hidden = quoteBlock(quote.on, quote.in, comment.anchor, comment.events.some((e) => e.verb === "anchor"));
        hidden.classList.add("em-undrawn");
        hidden.hidden = true;
        body.append(hidden);
      }
      if (retracted) body.append(h("p", { class: "em-hint" }, "Retracted by its author."));
      const note = this.note(comment.note);
      if (note) body.append(note);
      if (!retracted && mine && !resolved && this.actions.retract) {
        body.append(
          h("div", { class: "em-actions" }, this.button("Retract", () => this.actions.retract!(comment.id), "em-quiet")),
        );
      }
      card.append(body);
    }
    const actions = h("div", { class: "em-actions" });
    if (!resolved) {
      if (this.actions.reply) actions.append(this.replyButton(card, root.id, "Reply"));
      const resolve = this.button("Resolve", this.actions.resolve && (() => this.actions.resolve!(root.id)));
      if (resolve) actions.append(resolve);
    } else {
      const reopen = this.button("Reopen", this.actions.reopen && (() => this.actions.reopen!(root.id)));
      if (reopen) actions.append(reopen);
    }
    if (actions.childNodes.length > 0) card.append(actions);
    return card;
  }

  private replyButton(card: HTMLElement, id: string, label: string): HTMLElement {
    return h(
      "button",
      {
        type: "button",
        class: "em-button",
        onclick: () => {
          if (card.querySelector(":scope > .em-form")) return;
          const text = h("textarea", { class: "em-input", rows: "3", "aria-label": label, placeholder: "Write in Markdown" });
          const form: HTMLFormElement = h(
            "form",
            {
              class: "em-form",
              dataset: { for: id },
              onsubmit: (event: Event) => {
                event.preventDefault();
                const note = text.value.trim();
                if (note) this.submit(form, () => this.actions.reply!(id, note), () => form.remove());
              },
            },
            text,
            h(
              "div",
              { class: "em-actions" },
              h("button", { type: "submit", class: "em-button em-primary" }, "Send"),
              h("button", { type: "button", class: "em-button em-quiet", onclick: () => form.remove() }, "Cancel"),
            ),
          );
          card.append(form);
          text.focus();
        },
      },
      label,
    );
  }

  private suggestionCard(suggestion: Item, slot: number, remarks: Thread[], stale: boolean): HTMLElement {
    const id = suggestion.id;
    const edits = suggestion.edits ?? [];
    const card = h("section", {
      class: `em-card em-suggestion${slot >= 0 ? ` em-s${slot}` : ""}`,
      dataset: { id },
      tabindex: "0",
    });
    card.append(
      h(
        "header",
        { class: "em-card-head" },
        h("span", { class: "em-kind" }, "Suggestion"),
        h("code", { class: "em-id" }, id),
        stale ? h("span", { class: "em-chip em-warn" }, `stale${suggestion.reason ? ": " + suggestion.reason : ""}`) : null,
      ),
      this.byline(suggestion),
    );
    const note = this.note(suggestion.note);
    if (note) card.append(note);
    card.append(h("p", { class: "em-edit-count" }, edits.length === 1 ? "1 edit" : `${edits.length} edits`));
    const list = h("ol", { class: "em-edits" });
    for (const edit of edits) {
      list.append(
        h(
          "li",
          {},
          stale ? quoteBlock(edit.replace, edit.in, edit.range ? undefined : "not found exactly once") : null,
          stale ? h("div", { class: "em-label" }, "Suggested text") : null,
          stale ? h("pre", { class: "em-code" }, h("ins", { class: "em-ins" }, edit.with || " ")) : inlineSourceDiff(edit.replace, edit.with),
        ),
      );
    }
    card.append(list);
    if (stale) {
      card.append(
        h(
          "p",
          { class: "em-hint" },
          "This suggestion no longer applies to the body, so it cannot be accepted. Its author can retract it and suggest again.",
        ),
      );
    }

    const notice = h("p", { class: "em-hint em-warn-text", hidden: true, role: "status" });
    card.append(notice);
    const actions = h("div", { class: "em-actions" });
    // A stale suggestion is never offered for acceptance.
    if (!stale && this.actions.accept) {
      const gate = this.gate(id);
      const accept = h(
        "button",
        {
          type: "button",
          class: "em-button em-primary em-accept",
          onclick: () => {
            if (gate.errors.length > 0 && !gate.confirmed) {
              gate.confirmed = true;
              this.refreshGate(id);
              return;
            }
            this.actions.accept!(id);
          },
        },
        "Accept",
      );
      gate.button = accept;
      gate.notice = notice;
      actions.append(accept);
      this.refreshGate(id);
    }
    const reject = this.button("Reject", this.actions.reject && (() => this.actions.reject!(id)));
    if (reject) actions.append(reject);
    if (suggestion.by === this.options.me) {
      const retract = this.button("Retract", this.actions.retract && (() => this.actions.retract!(id)));
      if (retract) actions.append(retract);
    }
    if (!stale) {
      actions.append(
        this.button("Preview", () => {
          this.previewId = id;
          this.setView("preview");
        })!,
      );
    }
    if (this.actions.reply) actions.append(this.replyButton(card, id, "Comment"));
    card.append(actions);
    for (const thread of remarks) card.append(this.threadCard(thread));
    return card;
  }

  // Preview.

  private drawPreview(content: HTMLElement, doc: DocumentJson): void {
    const open = doc.items.filter((item) => item.kind === "suggestion" && item.state === "open");
    const usable = open.filter((item) => item.status === "applicable");
    if (!usable.some((item) => item.id === this.previewId)) this.previewId = usable[0]?.id ?? null;
    if (open.length === 0) {
      content.append(h("p", { class: "em-empty" }, "There is no open suggestion to preview."));
      return;
    }
    const select = h("select", {
      class: "em-input em-select",
      id: "em-preview-choice",
      onchange: () => {
        this.previewId = select.value;
        this.draw();
      },
    });
    for (const item of open) {
      const option = h(
        "option",
        { value: item.id, disabled: item.status !== "applicable" },
        `${item.id} by ${item.by}${item.status !== "applicable" ? " (stale)" : ""}`,
      );
      option.selected = item.id === this.previewId;
      select.append(option);
    }
    content.append(
      h("div", { class: "em-toolbar" }, h("label", { for: "em-preview-choice" }, "The body with this suggestion applied: "), select),
    );
    if (!this.previewId) {
      content.append(h("p", { class: "em-empty" }, "Every open suggestion is stale, so none can be previewed."));
      return;
    }
    const response = this.options.core.call<PreviewOk>({ op: "preview", source: this.source, id: this.previewId });
    if (!response.ok) {
      content.append(h("p", { class: "em-error" }, `${response.message} ${response.hint}`));
      return;
    }
    content.append(this.plainBody(response.body, splitBlocks(this.md, response.body)));
  }

  // History.

  private drawHistory(content: HTMLElement, doc: DocumentJson, threads: Thread[]): void {
    const map = new ByteMap(doc.body);
    const closed = closedItems(doc.items, threads);
    if (!closed.some((entry) => entry.item.id === this.selected)) this.selected = null;

    const highlights: Highlight[] = [];
    const chosen = closed.find((entry) => entry.item.id === this.selected);
    if (chosen) {
      const ranges = chosen.thread ? [chosen.item.range] : (chosen.item.edits ?? []).map((edit) => edit.range);
      for (const range of ranges) {
        const converted = map.range(range ?? null);
        if (converted && converted[1] > converted[0]) highlights.push({ id: chosen.item.id, start: converted[0], end: converted[1] });
      }
    }

    const list = h("div", { class: "em-history-list" });
    if (closed.length === 0) list.append(h("p", { class: "em-empty" }, "Nothing is closed yet."));
    for (const entry of closed) list.append(this.historyCard(doc, entry, entry.item.id === this.selected, highlights.length));
    content.append(
      h(
        "div",
        { class: "em-history" },
        h(
          "section",
          { class: "em-history-side", "aria-label": "Closed items" },
          h("h2", { class: "em-label" }, "Closed items, newest first"),
          list,
        ),
        h(
          "section",
          { class: "em-history-doc", "aria-label": "Current body" },
          h("h2", { class: "em-label" }, "Current body"),
          this.plainBody(doc.body, this.parsed, highlights, "landed"),
        ),
      ),
    );
  }

  private historyCard(doc: DocumentJson, entry: Closed, selected: boolean, located: number): HTMLElement {
    const item = entry.item;
    if (entry.thread) {
      const card = this.threadCard(entry.thread, true);
      card.classList.add("em-history-item");
      if (selected) card.classList.add("em-selected");
      return card;
    }
    const card = h("section", {
      class: `em-card em-history-item em-${item.kind}${selected ? " em-selected" : ""}`,
      dataset: { id: item.id },
      tabindex: "0",
    });
    const kind = item.kind === "suggestion" ? "Suggestion" : item.kind === "edit" ? "Edit" : "Revert";
    const change = isChange(item);
    const effective = inEffect(item);
    const chain = revertChain(doc.items, item);
    const closing = item.events.find((event) => event.verb === "accept" || event.verb === "reject" || event.verb === "retract");

    const head = h(
      "header",
      { class: "em-card-head" },
      h("span", { class: "em-kind" }, kind),
      h("code", { class: "em-id" }, item.id),
      item.kind === "revert" && item.re ? h("span", { class: "em-chip" }, `reverts ${item.re}`) : null,
    );
    if (item.kind === "suggestion") head.append(h("span", { class: "em-chip" }, item.state));
    if (change) {
      head.append(
        effective
          ? h("span", { class: "em-chip em-ok" }, chain.length > 0 ? "in effect again" : "in effect")
          : h("span", { class: "em-chip em-warn" }, "reverted, not in effect"),
      );
    }
    card.append(head, this.byline(item));
    const note = this.note(item.note);
    if (note) card.append(note);

    const list = h("ol", { class: "em-edits" });
    for (const edit of item.edits ?? []) list.append(h("li", {}, inlineSourceDiff(edit.replace, edit.with)));
    card.append(list);

    if (closing) {
      const verb = closing.verb === "accept" ? "Accepted" : closing.verb === "reject" ? "Rejected" : "Retracted";
      card.append(h("div", { class: "em-event" }, h("span", {}, `${verb} by `), this.byline(closing), this.note(closing.note)));
    }
    // A reverted change always shows the revert that undid it, and what happened to that revert.
    chain.forEach((revert, i) => {
      const target = i === 0 ? "" : ` ${chain[i - 1]!.id} was`;
      card.append(
        h(
          "div",
          { class: "em-event em-reverted-by" },
          h("span", {}, i === 0 ? "Reverted by " : `Then${target} reverted by `),
          h("code", { class: "em-id" }, revert.id),
          this.byline(revert),
          this.note(revert.note),
        ),
      );
    });

    if (selected) {
      card.append(
        h(
          "p",
          { class: "em-hint", role: "status" },
          !change
            ? "This suggestion was never applied, so there is nothing to locate."
            : !effective
              ? "This change is not in effect, so its text is not in the body."
              : located > 0
                ? "Its text is highlighted in the body."
                : "Its text cannot be located in the body as it reads now.",
        ),
      );
    }

    if (change) {
      // Reverting and restoring both mean reverting the tip of the chain. Ask
      // the core whether that would work, and offer nothing when it would not.
      const tip = chain.length > 0 ? chain[chain.length - 1]! : item;
      const wanted = effective ? this.actions.revert : this.actions.restore;
      if (wanted) {
        const probe = this.options.core.call<WriteOk>({
          op: "revert",
          source: this.source,
          id: tip.id,
          new_id: this.freeId(doc),
          by: "probe",
          at: "1970-01-01T00:00:00Z",
        });
        if (probe.ok) {
          const run = effective ? () => this.actions.revert!(item.id) : () => this.actions.restore!(item.id, tip.id);
          card.append(h("div", { class: "em-actions" }, this.button(effective ? "Revert" : "Restore", run)));
        } else {
          card.append(
            h(
              "p",
              { class: "em-hint em-warn-text" },
              `${effective ? "Revert" : "Restore"} is not offered: ${probe.message}.`,
            ),
          );
        }
      }
    }
    return card;
  }

  private freeId(doc: DocumentJson): string {
    for (;;) {
      const id = randomId("r");
      if (!doc.items.some((item) => item.id === id)) return id;
    }
  }

  // Interaction.

  private clicked(event: MouseEvent): void {
    const target = event.target as HTMLElement;
    if (target.closest("button, a, textarea, select, input, label, .em-popover")) return;
    const holder = target.closest<HTMLElement>("[data-id]");
    if (this.view === "history") {
      const card = target.closest<HTMLElement>(".em-history-item");
      if (!card) return;
      this.selected = this.selected === card.dataset.id ? null : card.dataset.id!;
      this.draw();
      const mark = this.root.querySelector(".em-landed");
      mark?.scrollIntoView({ block: "nearest" });
      return;
    }
    for (const element of this.root.querySelectorAll(".em-active")) element.classList.remove("em-active");
    if (!holder) return;
    const id = holder.dataset.id!;
    const all = this.root.querySelectorAll<HTMLElement>(`[data-id="${cssEscape(id)}"]`);
    for (const element of all) element.classList.add("em-active");
    const other = [...all].find((element) => element.classList.contains("em-card") !== !!holder.closest(".em-card"));
    other?.scrollIntoView({ block: "nearest" });
  }

  private selectionChanged(): void {
    clearTimeout(this.selectionTimer);
    this.selectionTimer = setTimeout(() => this.offerSelection(), 150);
  }

  /** The selection as source text, when it lies in the body as drawn in Review view. */
  private selectedSource(): { quote: string; scopes: string[]; rect: DOMRect } | null {
    if (this.view !== "review") return null;
    const selection = document.getSelection();
    if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
    const range = selection.getRangeAt(0);
    const startBlock = closestSource(range.startContainer);
    const endBlock = closestSource(range.endContainer);
    if (!startBlock || !endBlock || !this.root.contains(startBlock) || !this.root.contains(endBlock)) return null;
    const startInfo = this.sources.get(startBlock);
    const endInfo = this.sources.get(endBlock);
    if (!startInfo || !endInfo) return null;
    const rect = range.getBoundingClientRect();
    const typed = selection.toString().trim();
    if (!typed) return null;

    const from = sourceOffset(startBlock, startInfo.source, range.startContainer, range.startOffset, "start");
    const to = sourceOffset(endBlock, endInfo.source, range.endContainer, range.endOffset, "end");
    if (from === null || to === null || startInfo.start + from >= endInfo.start + to) {
      // The rendered text could not be lined up with the source. Offer it as it reads.
      return { quote: typed, scopes: [], rect };
    }
    const start = startInfo.start + from;
    const end = endInfo.start + to;
    const quote = this.body.slice(start, end);
    const lineStart = this.body.lastIndexOf("\n", start - 1) + 1;
    const lineEnd = this.body.indexOf("\n", end);
    const line = this.body.slice(lineStart, lineEnd < 0 ? this.body.length : lineEnd);
    const block = this.body.slice(startInfo.start, endInfo.start + endInfo.source.length);
    const scopes = [line, block].filter((scope, i, all) => scope !== quote && all.indexOf(scope) === i);
    return { quote, scopes, rect };
  }

  private offerSelection(): void {
    if (this.popover?.querySelector("form")) return;
    this.popover?.remove();
    this.popover = null;
    if (!this.actions.comment && !this.actions.suggest) return;
    const picked = this.selectedSource();
    if (!picked) return;

    const popover = h("div", { class: "em-popover", role: "dialog", "aria-label": "Act on the selected text" });
    const close = () => {
      popover.remove();
      if (this.popover === popover) this.popover = null;
    };
    const form = (kind: "comment" | "suggest") => {
      const replacement = h("textarea", { class: "em-input", rows: "3", "aria-label": "Replacement text" });
      replacement.value = picked.quote;
      const note = h("textarea", {
        class: "em-input",
        rows: "3",
        "aria-label": kind === "comment" ? "Comment" : "Why (optional)",
        placeholder: kind === "comment" ? "Comment, in Markdown" : "Why (optional)",
      });
      const element: HTMLFormElement = h(
        "form",
        {
          class: "em-form",
          onsubmit: (event: Event) => {
            event.preventDefault();
            const input = { quote: picked.quote, scopes: picked.scopes, note: note.value.trim() };
            const run = () =>
              kind === "comment" ? this.actions.comment!(input) : this.actions.suggest!({ ...input, replacement: replacement.value });
            this.submit(element, run, close);
          },
        },
        h("div", { class: "em-label" }, kind === "comment" ? "Comment on" : "Replace"),
        h("pre", { class: "em-code em-picked" }, picked.quote),
        kind === "suggest" ? h("div", { class: "em-label" }, "With") : null,
        kind === "suggest" ? replacement : null,
        note,
        h(
          "div",
          { class: "em-actions" },
          h("button", { type: "submit", class: "em-button em-primary" }, kind === "comment" ? "Comment" : "Suggest"),
          h("button", { type: "button", class: "em-button em-quiet", onclick: close }, "Cancel"),
        ),
      );
      popover.replaceChildren(element);
      popover.classList.add("em-popover-form");
      (kind === "comment" ? note : replacement).focus();
    };
    // Keep the selection when a button is pressed.
    popover.addEventListener("mousedown", (event) => {
      if ((event.target as HTMLElement).closest("button")) event.preventDefault();
    });
    for (const button of [
      this.button("Comment", this.actions.comment && (() => form("comment")), "em-do-comment"),
      this.button("Suggest", this.actions.suggest && (() => form("suggest")), "em-do-suggest"),
    ]) {
      if (button) popover.append(button);
    }
    const frame = this.root.getBoundingClientRect();
    popover.style.top = `${picked.rect.bottom - frame.top + 6}px`;
    const width = Math.min(320, frame.width - 16);
    popover.style.maxWidth = `${width}px`;
    popover.style.left = `${Math.max(8, Math.min(picked.rect.left - frame.left, frame.width - width - 8))}px`;
    this.root.append(popover);
    this.popover = popover;
  }
}

// Pieces that need no renderer state.

function isCode(block: Block | null): boolean {
  return !!block && (block.type === "fence" || block.type === "code_block");
}

function closestSource(node: Node): HTMLElement | null {
  const element = node.nodeType === Node.ELEMENT_NODE ? (node as HTMLElement) : node.parentElement;
  return element?.closest<HTMLElement>(".em-src") ?? null;
}

/** The original quote of an unanchored item, as a blockquote. */
function quoteBlock(on: string, scope: string | null | undefined, state?: string, moved = false): HTMLElement {
  return h(
    "blockquote",
    { class: "em-original" },
    h("div", { class: "em-label" }, state && state !== "attached" ? `${moved ? "Quote from its last anchor record" : "Original quote"} (${state})` : "Quote"),
    h("pre", { class: "em-code" }, on),
    scope ? h("div", { class: "em-label" }, "Within") : null,
    scope ? h("pre", { class: "em-code" }, scope) : null,
  );
}

/** A line diff of source text: removed lines struck through, added lines underlined. */
function sourceDiff(before: string, after: string): HTMLElement {
  const pre = h("pre", { class: "em-code em-linediff" });
  for (const change of lineDiff(before, after)) {
    const text = change.line === "" ? " " : change.line;
    if (change.op === "delete") pre.append(h("del", { class: "em-del em-line" }, h("span", { class: "em-sign" }, "- "), text));
    else if (change.op === "insert") pre.append(h("ins", { class: "em-ins em-line" }, h("span", { class: "em-sign" }, "+ "), text));
    else pre.append(h("span", { class: "em-line" }, h("span", { class: "em-sign" }, "  "), text));
  }
  return pre;
}

/** A word diff of source text, for the list of edits in a card. */
function inlineSourceDiff(before: string, after: string): HTMLElement {
  const code = h("code", { class: "em-code em-worddiff" });
  for (const segment of wordDiff(before, after)) {
    if (segment.op === "delete") code.append(h("del", { class: "em-del" }, segment.text));
    else if (segment.op === "insert") code.append(h("ins", { class: "em-ins" }, segment.text));
    else code.append(segment.text);
  }
  if (before === "") code.prepend(h("span", { class: "em-hint" }, "(at the end) "));
  if (after === "") code.append(h("span", { class: "em-hint" }, " (deleted)"));
  return code;
}

/** Marks the nodes of a suggested flowchart whose label is not in the current one. */
function markAdded(current: HTMLElement, suggested: HTMLElement[]): void {
  if (!current.classList.contains("em-drawn")) return;
  const labels = new Set([...current.querySelectorAll("g.node")].map((node) => node.textContent?.trim()));
  if (labels.size === 0) return;
  for (const host of suggested) {
    for (const node of host.querySelectorAll("g.node")) {
      if (!labels.has(node.textContent?.trim())) node.classList.add("em-added-node");
    }
  }
}

function shortTime(at: string): string {
  const match = /^(\d{4}-\d\d-\d\d)T(\d\d:\d\d)(?::\d\d(?:\.\d+)?)?Z$/.exec(at);
  return match ? `${match[1]} ${match[2]} UTC` : at;
}

function capital(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function cssEscape(value: string): string {
  return typeof CSS !== "undefined" && CSS.escape ? CSS.escape(value) : value.replace(/["\\]/g, "\\$&");
}
