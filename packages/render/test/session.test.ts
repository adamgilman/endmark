// Drives the demo's actions: the same Session methods the page's buttons call.

import { beforeAll, expect, test } from "bun:test";
import { Session } from "../../../demo/src/session";
import type { Core } from "../src/core";
import { core as load, fixture } from "./helpers";

let core: Core;
beforeAll(async () => {
  core = await load();
});

function session(ids: string[] = []) {
  let n = 0;
  const times = ["2026-11-20T10:00:00Z", "2026-11-20T10:01:00Z", "2026-11-20T10:02:00Z", "2026-11-20T10:03:00Z"];
  let t = 0;
  return new Session(core, fixture("worked-example.md"), {
    now: () => times[Math.min(t++, times.length - 1)]!,
    newId: (letter) => ids[n++] ?? `${letter}-t${String(n).padStart(3, "0")}`,
  });
}

/** The accepted fixture, as the demo writes it: by=you, at the session's first time. */
const accepted = () =>
  fixture("worked-example-accepted.md").replaceAll("by=adam at=2026-11-11T17:40:00Z", "by=you at=2026-11-20T10:00:00Z");

/** What accepting appends. The body lines above it change too. */
const ACCEPT = accepted().slice(accepted().indexOf("\n<!--endmark accept s-4q8d"));

const revert = (id: string, re: string, at: string, back: boolean) => `
<!--endmark revert ${id} re=${re} by=you at=${at}
\`\`\`replace
port ${back ? 8443 : 8080}
\`\`\`
\`\`\`with
port ${back ? 8080 : 8443}
\`\`\`

\`\`\`replace
## ${back ? "Installation" : "Setup"}
\`\`\`
\`\`\`with
## ${back ? "Setup" : "Installation"}
\`\`\`

\`\`\`replace
  A[Webhook] --\\> B[Relay]${back ? " --\\> C[Consumer]" : ""}
\`\`\`
\`\`\`with
  A[Webhook] --\\> B[Relay]${back ? "" : " --\\> C[Consumer]"}
\`\`\`
-->

<!--endmark anchor c-7f3k by=you at=${at} line=3
\`\`\`on
port ${back ? 8080 : 8443}
\`\`\`
\`\`\`near
Relay forwards webhook events to internal consumers. It listens on port ${back ? 8080 : 8443} by default.
\`\`\`
-->
`;

test("accept, revert, restore, then revert again: the page follows the chain", () => {
  const original = fixture("worked-example.md");
  const s = session(["r-aaaa", "r-bbbb", "r-cccc"]);

  expect(s.accept("s-4q8d").ok).toBe(true);
  expect(s.source).toBe(accepted());
  expect(s.source.endsWith(ACCEPT)).toBe(true);
  expect(s.document().body).toContain("port 8443");
  expect(s.changed.length).toBeGreaterThan(8);

  // Revert, from History.
  expect(s.revert("s-4q8d").ok).toBe(true);
  expect(s.source).toBe(original + ACCEPT + revert("r-aaaa", "s-4q8d", "2026-11-20T10:01:00Z", true));
  let doc = s.document();
  expect(doc.body).toContain("port 8080");
  expect(doc.items.find((i) => i.id === "s-4q8d")).toMatchObject({ state: "accepted", status: "reverted", reverted_by: "r-aaaa" });

  // Restore: the renderer passes the tip of the chain.
  expect(s.restore("s-4q8d", "r-aaaa").ok).toBe(true);
  expect(s.source.endsWith(revert("r-bbbb", "r-aaaa", "2026-11-20T10:02:00Z", false))).toBe(true);
  doc = s.document();
  expect(doc.body).toContain("## Installation");
  expect(doc.items.find((i) => i.id === "s-4q8d")!.status).toBe("located");
  expect(doc.items.find((i) => i.id === "r-aaaa")).toMatchObject({ status: "reverted", reverted_by: "r-bbbb" });

  // Revert the suggestion a second time. The record names the second revert, not the suggestion.
  const third = s.revert("s-4q8d");
  expect(third.ok).toBe(true);
  expect(s.source.endsWith(revert("r-cccc", "r-bbbb", "2026-11-20T10:03:00Z", true))).toBe(true);
  expect(s.document().diagnostics).toEqual([]);

  expect(s.reset().ok).toBe(true);
  expect(s.source).toBe(original);
});

test("reverting a change that is not in effect is refused with a message and a hint", () => {
  const s = session(["r-aaaa"]);
  s.accept("s-4q8d");
  s.revert("s-4q8d");
  const before = s.source;
  const outcome = s.restore("s-4q8d", "s-4q8d");
  expect(outcome.ok).toBe(false);
  if (!outcome.ok) {
    expect(outcome.code).toBe("not-in-effect");
    expect(outcome.message.length).toBeGreaterThan(0);
    expect(outcome.hint).toContain("r-aaaa");
  }
  expect(s.source).toBe(before);
});

test("comment, reply, resolve, reopen, suggest, reject and retract append records only", () => {
  const s = session();
  const body = s.document().body;

  expect(s.comment({ quote: "webhook events", scopes: [], note: "Which events?" }).ok).toBe(true);
  expect(s.source).toContain("<!--endmark comment c-t001 by=you at=2026-11-20T10:00:00Z line=3\n```on\nwebhook events\n```\n```near\nRelay forwards webhook events to internal consumers. It listens on port 8080 by default.\n```\nWhich events?\n-->");

  expect(s.reply("c-7f3k", "Agreed.").ok).toBe(true);
  expect(s.source).toContain("<!--endmark comment c-t002 re=c-7f3k by=you");
  expect(s.resolve("c-7f3k").ok).toBe(true);
  expect(s.source).toContain("<!--endmark resolve c-7f3k by=you");
  expect(s.document().items.find((i) => i.id === "c-7f3k")!.thread).toBe("resolved");
  expect(s.reopen("c-7f3k").ok).toBe(true);
  expect(s.document().items.find((i) => i.id === "c-7f3k")!.thread).toBe("open");

  expect(s.suggest({ quote: "fixed 5 second", scopes: [], replacement: "fixed 10 second", note: "" }).ok).toBe(true);
  expect(s.source).toContain("```replace\nfixed 5 second\n```\n```with\nfixed 10 second\n```");
  expect(s.retract("s-t003").ok).toBe(true);
  expect(s.reject("s-4q8d").ok).toBe(true);
  expect(s.source.trimEnd().endsWith("<!--endmark reject s-4q8d by=you at=2026-11-20T10:03:00Z -->")).toBe(true);

  expect(s.document().body.trimEnd()).toBe(body.trimEnd());
  expect(s.retract("c-9d1t")).toMatchObject({ ok: false, code: "not-author" });
});

test("a quote that is not unique is retried inside the scopes the renderer gave", () => {
  const s = session();
  const line = "Failed deliveries are retried 3 times with a fixed 5 second delay.";
  expect(s.comment({ quote: "e", scopes: [], note: "x" })).toMatchObject({ ok: false, code: "quote-ambiguous" });
  expect(s.comment({ quote: "fixed", scopes: [line], note: "x" }).ok).toBe(true);
  const outcome = s.comment({ quote: "re", scopes: ["are retried", line], note: "Which?" });
  expect(outcome).toMatchObject({ ok: false, code: "quote-ambiguous" });
  expect(s.comment({ quote: "3", scopes: [line], note: "Seconds?" }).ok).toBe(true);
  expect(s.source).toContain("```in\n" + line + "\n```\n```on\n3\n```\n```near\n" + line + "\n```\nSeconds?");
});

test("an id that is in use is drawn again", () => {
  const s = session(["c-7f3k", "c-9d1t", "c-free"]);
  expect(s.comment({ quote: "webhook events", scopes: [], note: "n" }).ok).toBe(true);
  expect(s.source).toContain("<!--endmark comment c-free ");
});

test("a direct body edit writes no record and detaches the comment on the edited text", () => {
  const s = session();
  const records = s.source.slice(s.bodyText()!.length);
  const outcome = s.editBody(s.bodyText()!.replace("port 8080", "port 9000"));
  expect(outcome.ok).toBe(true);
  expect(s.source.slice(s.bodyText()!.length)).toBe(records);
  expect(s.changed).toEqual([2]);
  const doc = s.document();
  expect(doc.items.find((i) => i.id === "c-7f3k")!.anchor).toBe("detached");
  expect(doc.items.find((i) => i.id === "s-4q8d")!.status).toBe("stale");
  expect(doc.diagnostics.map((d) => [d.code, d.id])).toContainEqual(["detached", "c-7f3k"]);
});

test("default ids have a type letter, a hyphen and four characters of the alphabet", () => {
  const s = new Session(core, fixture("worked-example.md"));
  expect(s.comment({ quote: "webhook events", scopes: [], note: "n" }).ok).toBe(true);
  const match = /<!--endmark comment (\S+) by=you at=(\S+) line=3\n/.exec(s.source.slice(fixture("worked-example.md").length))!;
  expect(match[1]).toMatch(/^c-[0-9abcdefghjkmnpqrstvwxyz]{4}$/);
  expect(match[2]).toMatch(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/);
});
