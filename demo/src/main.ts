// The demo page of RFC 0001, section 12.5. It holds no format logic: the
// renderer draws, the session asks the core to write, and this file wires
// the two to the page.

import example from "../../conformance/worked-example.md" with { type: "text" };
import { loadCore, mount, type Actions, type Renderer } from "../../packages/render/src/index";
import { Session, type Outcome } from "./session";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function start(): Promise<void> {
  const status = $("status");
  const core = await loadCore(new URL("endmark.wasm", document.baseURI));
  const session = new Session(core, example);
  let renderer: Renderer;

  const report = (outcome: Outcome) => {
    status.classList.toggle("refused", !outcome.ok);
    status.replaceChildren();
    if (outcome.ok) {
      status.append(outcome.message);
    } else {
      const label = document.createElement("strong");
      label.textContent = "Refused:";
      const code = document.createElement("code");
      code.textContent = outcome.code;
      status.append(label, `${outcome.message}. `, outcome.hint ? `${sentence(outcome.hint)} ` : "", code);
    }
  };

  const drawSource = (scroll: boolean) => {
    const list = $("source");
    const changed = new Set(session.changed);
    list.replaceChildren();
    const lines = session.source.split("\n");
    if (lines[lines.length - 1] === "") lines.pop();
    let first: HTMLElement | null = null;
    lines.forEach((line, i) => {
      const item = document.createElement("li");
      const sign = document.createElement("span");
      sign.className = "sign";
      const text = document.createElement("span");
      text.textContent = line;
      if (changed.has(i)) {
        item.className = "changed";
        item.title = "Added or changed by the last action";
        sign.textContent = "+";
        first ??= item;
      }
      item.append(sign, text);
      list.append(item);
    });
    if (scroll && first) {
      // Scroll the pane, not the page.
      list.scrollTop = Math.max(0, (first as HTMLElement).offsetTop - list.clientHeight / 3);
    }

    const findings = $("findings");
    findings.replaceChildren();
    const diagnostics = session.document().diagnostics;
    if (diagnostics.length === 0) {
      const none = document.createElement("li");
      none.className = "none";
      none.textContent = "No findings.";
      findings.append(none);
    }
    for (const finding of diagnostics) {
      const item = document.createElement("li");
      const code = document.createElement("code");
      code.textContent = finding.code;
      item.append(`${finding.severity}, line ${finding.line}: ${finding.message} `, code);
      if (finding.hint) item.append(` ${sentence(finding.hint)}`);
      findings.append(item);
    }
  };

  /** Runs one action, then shows what it did on both sides. */
  const act = (run: () => Outcome) => {
    const outcome = run();
    report(outcome);
    if (outcome.ok) {
      renderer.update(session.source);
      drawSource(true);
    }
    return outcome;
  };

  const actions: Actions = {
    comment: (input) => act(() => session.comment(input)),
    suggest: (input) => act(() => session.suggest(input)),
    reply: (id, note) => act(() => session.reply(id, note)),
    accept: (id) => act(() => session.accept(id)),
    reject: (id) => act(() => session.reject(id)),
    retract: (id) => act(() => session.retract(id)),
    resolve: (id) => act(() => session.resolve(id)),
    reopen: (id) => act(() => session.reopen(id)),
    revert: (id) => act(() => session.revert(id)),
    restore: (id, tip) => act(() => session.restore(id, tip)),
  };

  renderer = mount($("doc"), session.source, { core, view: "review", me: "you", actions, images: false });
  drawSource(false);
  report({ ok: true, message: "Ready. The example is the Relay file from Appendix A of the RFC." });

  $("reset").addEventListener("click", () => {
    act(() => session.reset());
    renderer.setView("review");
    $("source").scrollTop = 0;
  });

  const dialog = $<HTMLDialogElement>("body-dialog");
  const text = $<HTMLTextAreaElement>("body-text");
  $("edit-body").addEventListener("click", () => {
    const body = session.bodyText();
    if (body === null) {
      act(() => session.editBody(""));
      return;
    }
    text.value = body;
    dialog.returnValue = "";
    dialog.showModal();
  });
  dialog.addEventListener("close", () => {
    if (dialog.returnValue === "apply") act(() => session.editBody(text.value));
  });

  // For the smoke test and for anyone exploring from the console.
  Object.assign(window, { endmarkDemo: { session, actions, renderer } });
  document.documentElement.dataset.ready = "true";
}

function sentence(text: string): string {
  const trimmed = text.trim();
  const capital = trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
  return /[.!?]$/.test(capital) ? capital : capital + ".";
}

start().catch((error) => {
  const status = $("status");
  status.classList.add("refused");
  status.textContent = `The demo could not start: ${error instanceof Error ? error.message : String(error)}`;
});
