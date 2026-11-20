// The page that `endmark view FILE` serves. It holds no format logic: the
// renderer draws through the core (WebAssembly), and every write is one
// operation sent to the server, which performs it on the file on disk.

import { loadCore, mount, type Actions, type Renderer } from "../../packages/render/src/index";
import { Client, type Failure, type Outcome } from "./client";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

/** How often the page asks whether the file changed on disk. */
const POLL_MS = 1000;

function element(tag: string, text: string, className = ""): HTMLElement {
  const made = document.createElement(tag);
  made.textContent = text;
  if (className) made.className = className;
  return made;
}

function sentence(text: string): string {
  const trimmed = text.trim();
  const capital = trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
  return /[.!?]$/.test(capital) ? capital : capital + ".";
}

/** Shows the outcome of the last action. The same text twice is not announced twice. */
function report(outcome: { ok: true; message: string } | Failure, label = "Refused:"): void {
  const status = $("status");
  const parts: (Node | string)[] = [];
  if (outcome.ok) {
    parts.push(outcome.message);
  } else {
    parts.push(element("strong", label), `${sentence(outcome.message)} `);
    if (outcome.hint) parts.push(`${sentence(outcome.hint)} `);
    parts.push(element("code", outcome.code));
  }
  const fresh = document.createElement("p");
  fresh.append(...parts);
  if (status.textContent === fresh.textContent && status.classList.contains("refused") === !outcome.ok) return;
  status.classList.toggle("refused", !outcome.ok);
  status.replaceChildren(...fresh.childNodes);
}

async function start(): Promise<void> {
  const token = document.querySelector<HTMLMetaElement>('meta[name="endmark-token"]')?.content ?? "";
  const client = new Client(token);
  const [core, first] = await Promise.all([loadCore(new URL("endmark.wasm", document.baseURI)), client.file()]);
  if (!first.ok) throw new Error(`${first.message} (${first.code})`);

  // The header: the file, who writes, and whether anything can be written.
  // All of it is set as text, never as HTML.
  $("file").textContent = first.file;
  document.title = `${first.file.split(/[\\/]/).pop() || first.file} - Endmark`;
  const who = $("who");
  if (first.read_only) {
    who.append(element("span", "read-only", "mode"), " Nothing can be written from this page.");
  } else {
    who.append("Writing as ", element("code", first.by ?? ""));
    if (first.for) who.append(" for ", element("code", first.for));
  }

  let version = first.version;
  // Counts the writes this page made, so that an answer about the file that
  // was asked for before a write is not drawn after it.
  let writes = 0;
  let renderer: Renderer;

  const act = async (run: () => Promise<Outcome>): Promise<Outcome> => {
    const outcome = await run();
    // A refusal is an answer. No answer at all is not.
    report(outcome, !outcome.ok && /^(no-answer$|http-)/.test(outcome.code) ? "Not written:" : "Refused:");
    if (outcome.ok) {
      writes++;
      version = outcome.version;
      renderer.update(outcome.source);
    }
    return outcome;
  };

  const actions: Actions = {
    comment: (input) =>
      act(() => client.quoted(input.scopes, (scope) => ({ op: "comment", on: input.quote, in: scope, note: input.note }))),
    suggest: (input) =>
      act(() =>
        client.quoted(input.scopes, (scope) => ({
          op: "suggest",
          note: input.note,
          edits: [{ ...(scope === undefined ? {} : { in: scope }), replace: input.quote, with: input.replacement }],
        })),
      ),
    reply: (id, note) => act(() => client.write({ op: "reply", id, note })),
    accept: (id) => act(() => client.write({ op: "accept", id })),
    reject: (id) => act(() => client.write({ op: "reject", id })),
    retract: (id) => act(() => client.write({ op: "retract", id })),
    resolve: (id) => act(() => client.write({ op: "resolve", id })),
    reopen: (id) => act(() => client.write({ op: "reopen", id })),
    revert: (id) => act(() => client.write({ op: "revert", id })),
    // Restoring a change means reverting the last revert of its chain.
    restore: (_id, tip) => act(() => client.write({ op: "revert", id: tip })),
  };

  renderer = mount($("doc"), first.source, {
    core,
    me: first.by ?? undefined,
    // With no actions the renderer offers none.
    actions: first.read_only ? {} : actions,
    // An image would be a request to somewhere else, or to a file this server does not serve.
    images: false,
  });
  report({
    ok: true,
    message: first.read_only
      ? "Ready. This view is read-only."
      : "Ready. Select text to comment on it or to suggest a change.",
  });

  // Follow the file. A change made by an editor, an agent or another
  // terminal is drawn within a poll or two. The renderer keeps a form that
  // is being typed in.
  let checking = false;
  let lost = false;
  const check = async (): Promise<void> => {
    if (checking) return;
    checking = true;
    try {
      const before = writes;
      const seen = await client.version();
      if (before !== writes) return;
      if (!seen.ok) {
        lost = true;
        report(seen, "Not following the file:");
        return;
      }
      if (seen.version !== version) {
        const file = await client.file();
        if (before !== writes) return;
        if (!file.ok) {
          lost = true;
          report(file, "Not following the file:");
          return;
        }
        if (file.version !== version) {
          version = file.version;
          renderer.update(file.source);
          report({ ok: true, message: "The file changed on disk. This is the file as it reads now." });
          lost = false;
          return;
        }
      }
      if (lost) {
        lost = false;
        report({ ok: true, message: "Following the file again." });
      }
    } finally {
      checking = false;
    }
  };
  setInterval(() => {
    if (!document.hidden) void check();
  }, POLL_MS);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) void check();
  });

  // For the browser test and for anyone exploring from the console.
  Object.assign(window, { endmarkView: { actions, renderer, check } });
  document.documentElement.dataset.ready = "true";
}

start().catch((error) => {
  report(
    { ok: false, code: "not-started", message: error instanceof Error ? error.message : String(error), hint: "" },
    "The view could not start:",
  );
});
