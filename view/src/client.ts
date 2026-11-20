// Talks to the server that `endmark view` runs. No DOM here, so the tests
// can drive it with a stand-in for `fetch`.
//
// The page never sends the file. It sends one operation, and the server
// performs it on the file as it is on disk at that moment.

import type { EditInput } from "../../packages/render/src/types";

export interface Failure {
  ok: false;
  code: string;
  message: string;
  hint: string;
}

/** `GET /api/file`. */
export interface FileState {
  ok: true;
  version: string;
  source: string;
  /** The file, as it was named on the command line. */
  file: string;
  /** Who writes. Null when the server is read-only. */
  by: string | null;
  for: string | null;
  read_only: boolean;
}

/** `GET /api/version`. */
export interface VersionState {
  ok: true;
  version: string;
}

/** `POST /api/write`: one operation. The server adds the author, the time and any new id. */
export interface Operation {
  op: "comment" | "reply" | "suggest" | "accept" | "reject" | "retract" | "resolve" | "reopen" | "revert";
  id?: string;
  on?: string;
  in?: string;
  note?: string;
  edits?: EditInput[];
}

export interface Written {
  ok: true;
  version: string;
  source: string;
  id: string;
  target: string | null;
  reanchored: string[];
  detached: string[];
  stale: string[];
}

export type Outcome = (Written & { message: string }) | Failure;

type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

export class Client {
  constructor(
    private readonly token: string,
    private readonly fetcher: Fetch = (input, init) => fetch(input, init),
  ) {}

  file(): Promise<FileState | Failure> {
    return this.request<FileState>("api/file");
  }

  version(): Promise<VersionState | Failure> {
    return this.request<VersionState>("api/version");
  }

  async write(operation: Operation): Promise<Outcome> {
    const response = await this.request<Written>("api/write", operation);
    return response.ok ? { ...response, message: describe(operation, response) } : response;
  }

  /** Tries a quote alone, then inside each wider scope, until the core finds it exactly once. */
  async quoted(scopes: string[], operation: (scope: string | undefined) => Operation): Promise<Outcome> {
    let outcome = await this.write(operation(undefined));
    for (const scope of scopes) {
      if (outcome.ok || outcome.code !== "quote-ambiguous") break;
      outcome = await this.write(operation(scope));
    }
    return outcome;
  }

  private async request<T extends { ok: true }>(path: string, body?: unknown): Promise<T | Failure> {
    let response: Response;
    try {
      response = await this.fetcher(path, {
        method: body === undefined ? "GET" : "POST",
        headers: {
          "X-Endmark-Token": this.token,
          ...(body === undefined ? {} : { "Content-Type": "application/json" }),
        },
        body: body === undefined ? undefined : JSON.stringify(body),
        cache: "no-store",
        credentials: "omit",
        redirect: "error",
      });
    } catch {
      return {
        ok: false,
        code: "no-answer",
        message: "The server did not answer",
        hint: "it may have stopped; start endmark view again and reload this page",
      };
    }
    try {
      const answer = (await response.json()) as T | Failure;
      if (typeof answer === "object" && answer !== null && typeof answer.ok === "boolean") return answer;
    } catch {
      // Not JSON: fall through.
    }
    return {
      ok: false,
      code: `http-${response.status}`,
      message: `The server answered ${response.status} without saying why`,
      hint: "reload this page",
    };
  }
}

function describe(operation: Operation, written: Written): string {
  const parts: string[] = [];
  switch (operation.op) {
    case "comment":
      parts.push(`Wrote comment ${written.id} to the file.`);
      break;
    case "reply":
      parts.push(`Wrote comment ${written.id}, a reply to ${operation.id}, to the file.`);
      break;
    case "suggest":
      parts.push(`Wrote suggestion ${written.id} to the file. The document text is unchanged.`);
      break;
    case "accept":
      parts.push(`Accepted ${operation.id}: its edits are applied to the document. Find it in History to revert it.`);
      break;
    case "revert":
      parts.push(`Wrote revert ${written.id}, which undoes ${written.target ?? operation.id}.`);
      break;
    case "resolve":
      parts.push(`Resolved ${operation.id}. Find the thread in History to reopen it.`);
      break;
    case "reopen":
      parts.push(`Reopened ${operation.id}.`);
      break;
    case "reject":
      parts.push(`Rejected ${operation.id}.`);
      break;
    case "retract":
      parts.push(`Retracted ${operation.id}.`);
      break;
  }
  if (written.reanchored.length) parts.push(`Re-pointed ${written.reanchored.join(", ")}.`);
  if (written.detached.length) parts.push(`Now detached: ${written.detached.join(", ")}.`);
  if (written.stale.length) parts.push(`Now stale: ${written.stale.join(", ")}.`);
  return parts.join(" ");
}
