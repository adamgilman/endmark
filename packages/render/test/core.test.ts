import { beforeAll, expect, test } from "bun:test";
import type { Core } from "../src/core";
import type { PreviewOk, StatusOk, WriteOk } from "../src/types";
import { core as load, fixture } from "./helpers";

let core: Core;
beforeAll(async () => {
  core = await load();
});

test("status reports the six items of the worked example", () => {
  const response = core.call<StatusOk>({ op: "status", source: fixture("worked-example.md") });
  if (!response.ok) throw new Error(response.message);
  const items = response.document.items;
  expect(items.map((item) => [item.id, item.kind, item.state])).toEqual([
    ["e-0h5n", "edit", "applied"],
    ["s-3c7j", "suggestion", "accepted"],
    ["c-7f3k", "comment", "live"],
    ["c-2m9x", "comment", "live"],
    ["s-4q8d", "suggestion", "open"],
    ["c-9d1t", "comment", "live"],
  ]);
  expect(items[2]!.anchor).toBe("attached");
  expect(items[3]!.root).toBe("c-7f3k");
  expect(items[4]!.status).toBe("applicable");
  expect(items[4]!.edits).toHaveLength(3);
  expect(response.document.diagnostics).toEqual([]);
});

test("accepting s-4q8d gives the accepted fixture byte for byte", () => {
  const response = core.call<WriteOk>({
    op: "accept",
    source: fixture("worked-example.md"),
    id: "s-4q8d",
    by: "adam",
    at: "2026-11-11T17:40:00Z",
  });
  if (!response.ok) throw new Error(response.message);
  expect(response.source).toBe(fixture("worked-example-accepted.md"));
  expect(response.reanchored).toEqual(["c-7f3k"]);
});

test("preview returns the body with the suggestion applied and writes nothing", () => {
  const response = core.call<PreviewOk>({ op: "preview", source: fixture("worked-example.md"), id: "s-4q8d" });
  if (!response.ok) throw new Error(response.message);
  expect(response.body).toContain("port 8443");
  expect(response.body).toContain("## Installation");
  expect(response.body).toContain("B[Relay] --> C[Consumer]");
});

test("a refused write comes back with a code, a message and a hint", () => {
  const response = core.call<WriteOk>({
    op: "comment",
    source: fixture("worked-example.md"),
    new_id: "c-0000",
    by: "you",
    at: "2026-11-20T10:00:00Z",
    on: "no such text",
  });
  expect(response.ok).toBe(false);
  if (response.ok) return;
  expect(response.code).toBe("quote-not-found");
  expect(response.message.length).toBeGreaterThan(0);
});
