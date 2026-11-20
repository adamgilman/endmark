// The client of the page that `endmark view` serves, against a stand-in
// for the server. The real server is tested from Rust
// (crates/endmark-cli/tests/view.rs) and in a browser (test/view.ts).

import { expect, test } from "bun:test";
import { Client } from "../../../view/src/client";

interface Call {
  path: string;
  method: string;
  headers: Record<string, string>;
  body: unknown;
}

function standIn(answer: (call: Call) => Response | Promise<Response>) {
  const calls: Call[] = [];
  const client = new Client("the-token", async (path, init) => {
    const call = {
      path,
      method: init?.method ?? "GET",
      headers: init?.headers as Record<string, string>,
      body: init?.body === undefined || init.body === null ? undefined : JSON.parse(String(init.body)),
    };
    calls.push(call);
    return answer(call);
  });
  return { client, calls };
}

const written = { ok: true, version: "v2", source: "x\n", id: "c-1abc", target: null, reanchored: [], detached: [], stale: [] };
const refusal = (code: string) => Response.json({ ok: false, code, message: `refused: ${code}`, hint: "a hint" });

test("every request carries the token, and a write is JSON", async () => {
  const { client, calls } = standIn((call) => Response.json(call.method === "GET" ? { ok: true, version: "v1" } : written));
  await client.version();
  await client.write({ op: "accept", id: "s-4q8d" });
  expect(calls.map((call) => [call.method, call.path])).toEqual([
    ["GET", "api/version"],
    ["POST", "api/write"],
  ]);
  expect(calls[0]!.headers).toEqual({ "X-Endmark-Token": "the-token" });
  expect(calls[1]!.headers).toEqual({ "X-Endmark-Token": "the-token", "Content-Type": "application/json" });
  // The operation, and nothing about who writes, when, or what the file holds.
  expect(calls[1]!.body).toEqual({ op: "accept", id: "s-4q8d" });
});

test("a quote that is not unique is tried again inside each wider scope", async () => {
  let n = 0;
  const { client, calls } = standIn(() => (n++ < 2 ? refusal("quote-ambiguous") : Response.json(written)));
  const outcome = await client.quoted(["the line", "the block", "never tried"], (scope) => ({
    op: "comment",
    on: "relay",
    in: scope,
    note: "Which?",
  }));
  expect(outcome.ok).toBe(true);
  expect(calls.map((call) => (call.body as { in?: string }).in)).toEqual([undefined, "the line", "the block"]);
  if (outcome.ok) expect(outcome.message).toBe("Wrote comment c-1abc to the file.");
});

test("any other refusal is returned as it is, at once", async () => {
  const { client, calls } = standIn(() => refusal("quote-not-found"));
  const outcome = await client.quoted(["the line"], (scope) => ({ op: "comment", on: "nope", in: scope }));
  expect(outcome).toEqual({ ok: false, code: "quote-not-found", message: "refused: quote-not-found", hint: "a hint" });
  expect(calls.length).toBe(1);
});

test("a server that is gone, or that answers something else, is a failure with a code", async () => {
  const gone = new Client("t", () => Promise.reject(new TypeError("Failed to fetch")));
  expect(await gone.file()).toMatchObject({ ok: false, code: "no-answer" });
  const other = new Client("t", async () => new Response("<html>not this server</html>", { status: 502 }));
  expect(await other.write({ op: "resolve", id: "c-1" })).toMatchObject({ ok: false, code: "http-502" });
  const refused = new Client("t", async () =>
    Response.json({ ok: false, code: "bad-token", message: "no", hint: "reload" }, { status: 403 }),
  );
  expect(await refused.version()).toEqual({ ok: false, code: "bad-token", message: "no", hint: "reload" });
});
