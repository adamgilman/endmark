// The demo's state and its actions. No DOM here, so the tests can drive the
// same code the buttons do. Every read and every write goes through the core.

import type { Core } from "../../packages/render/src/core";
import { changedLines } from "../../packages/render/src/diff";
import { randomId } from "../../packages/render/src/ids";
import type { DocumentJson, Request, StatusOk, WriteOk } from "../../packages/render/src/types";

export type Outcome =
  | { ok: true; message: string }
  | { ok: false; code: string; message: string; hint: string };

export interface SessionOptions {
  /** The handle every action is attributed to. */
  by?: string;
  /** The time of an action. Default: the clock, to the second, in UTC. */
  now?: () => string;
  /** A new id for a type letter. Default: the letter, a hyphen and four random characters. */
  newId?: (letter: string) => string;
}

type Write = Omit<Request, "source" | "by" | "at" | "new_id">;

const LETTERS: Record<string, string> = { comment: "c", reply: "c", suggest: "s", edit: "e", revert: "r" };

export class Session {
  source: string;
  /** 0-based lines of `source` that the last action added or changed. */
  changed: number[] = [];
  private readonly by: string;
  private readonly now: () => string;
  private readonly newId: (letter: string) => string;

  constructor(
    private readonly core: Core,
    private readonly original: string,
    options: SessionOptions = {},
  ) {
    this.source = original;
    this.by = options.by ?? "you";
    this.now = options.now ?? (() => new Date().toISOString().slice(0, 19) + "Z");
    this.newId = options.newId ?? randomId;
  }

  /** The current file as the core reads it. Its `diagnostics` are the linter's findings. */
  document(): DocumentJson {
    const response = this.core.call<StatusOk>({ op: "status", source: this.source });
    if (!response.ok) throw new Error(response.message);
    return response.document;
  }

  comment(input: { quote: string; scopes: string[]; note: string }): Outcome {
    return this.quoted(input.scopes, (scope) => ({ op: "comment", on: input.quote, in: scope, note: input.note }));
  }

  suggest(input: { quote: string; scopes: string[]; replacement: string; note: string }): Outcome {
    return this.quoted(input.scopes, (scope) => ({
      op: "suggest",
      note: input.note,
      edits: [{ ...(scope === undefined ? {} : { in: scope }), replace: input.quote, with: input.replacement }],
    }));
  }

  reply(id: string, note: string): Outcome {
    return this.write({ op: "reply", id, note });
  }

  accept(id: string): Outcome {
    return this.write({ op: "accept", id });
  }

  reject(id: string): Outcome {
    return this.write({ op: "reject", id });
  }

  retract(id: string): Outcome {
    return this.write({ op: "retract", id });
  }

  resolve(id: string): Outcome {
    return this.write({ op: "resolve", id });
  }

  reopen(id: string): Outcome {
    return this.write({ op: "reopen", id });
  }

  /** Undoes a change that is in effect. The core follows the chain and names its tip. */
  revert(id: string): Outcome {
    return this.write({ op: "revert", id });
  }

  /** Brings back a change that is not in effect, by reverting the last revert of its chain. */
  restore(_id: string, tip: string): Outcome {
    return this.write({ op: "revert", id: tip });
  }

  /** The body text an unaware tool would see and edit: the file up to its first record. */
  bodyText(): string | null {
    const body = this.document().body.replace(/\s+$/, "");
    return this.source.startsWith(body) ? body : null;
  }

  /** Replaces the body and writes no record, as a tool that does not know Endmark would. */
  editBody(text: string): Outcome {
    const body = this.bodyText();
    if (body === null) {
      return {
        ok: false,
        code: "demo-body-not-first",
        message: "The body cannot be edited here, because the file has records between its body lines.",
        hint: "Reset the example.",
      };
    }
    const next = text.replace(/\s+$/, "") + this.source.slice(body.length);
    if (next === this.source) return { ok: false, code: "demo-no-change", message: "The body is unchanged.", hint: "" };
    this.commit(next);
    const findings = this.document().diagnostics.length;
    return {
      ok: true,
      message: `Body edited directly. No record was written. The linter now reports ${count(findings, "finding")}.`,
    };
  }

  reset(): Outcome {
    this.commit(this.original);
    this.changed = [];
    return { ok: true, message: "The original example is back." };
  }

  /** Tries a quote alone, then inside each wider scope, until the core finds it exactly once. */
  private quoted(scopes: string[], request: (scope: string | undefined) => Write): Outcome {
    let outcome = this.write(request(undefined));
    for (const scope of scopes) {
      if (outcome.ok || outcome.code !== "quote-ambiguous") break;
      outcome = this.write(request(scope));
    }
    return outcome;
  }

  private write(request: Write): Outcome {
    const letter = LETTERS[request.op];
    for (let attempt = 0; ; attempt++) {
      const response = this.core.call<WriteOk>({
        ...request,
        source: this.source,
        by: this.by,
        at: this.now(),
        ...(letter ? { new_id: this.newId(letter) } : {}),
      });
      if (!response.ok) {
        if (response.code === "id-in-use" && attempt < 20) continue;
        return { ok: false, code: response.code, message: response.message, hint: response.hint };
      }
      this.commit(response.source);
      return { ok: true, message: describe(request, response) };
    }
  }

  private commit(next: string): void {
    this.changed = changedLines(this.source, next);
    this.source = next;
  }
}

function describe(request: Write, response: WriteOk): string {
  const parts: string[] = [];
  const id = response.id;
  switch (request.op) {
    case "comment":
      parts.push(`Appended comment ${id}.`);
      break;
    case "reply":
      parts.push(`Appended comment ${id}, with re=${request.id}.`);
      break;
    case "suggest":
      parts.push(`Appended suggestion ${id}. The body lines are unchanged.`);
      break;
    case "accept":
      parts.push(`Applied the edits of ${request.id} to the body and appended an accept record. Find it in History to revert it.`);
      break;
    case "revert":
      parts.push(`Applied the inverse edits to the body and appended revert ${id}, which names ${response.target ?? request.id}.`);
      break;
    case "resolve":
      parts.push(`Appended a resolve record for ${request.id}. Find the thread in History to reopen it.`);
      break;
    default:
      parts.push(`Appended a ${request.op} record for ${request.id}.`);
  }
  if (response.reanchored?.length) parts.push(`Re-pointed ${response.reanchored.join(", ")} with an anchor record.`);
  if (response.detached?.length) parts.push(`Now detached: ${response.detached.join(", ")}.`);
  if (response.stale?.length) parts.push(`Now stale: ${response.stale.join(", ")}.`);
  return parts.join(" ");
}

function count(n: number, word: string): string {
  return n === 1 ? `1 ${word}` : `${n} ${word}s`;
}
