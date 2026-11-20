// A smoke test of the built demo in headless Chromium.
// Run with `bun run smoke` in packages/render, after `bun run build`.
//
// It serves demo/dist, opens the page, and uses the page as a visitor would:
// it clicks the real buttons and selects real text. Set CHROMIUM to the
// browser binary if it is not `chromium`.

import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { serve } from "../serve";
import { openPage, type Page } from "./cdp";

const dist = resolve(import.meta.dir, "../../../demo/dist");
if (!existsSync(join(dist, "index.html"))) {
  console.error("demo/dist is missing. Run `bun run build` first.");
  process.exit(2);
}

let passed = 0;
const failures: string[] = [];
function check(name: string, ok: unknown, detail = ""): void {
  if (ok) {
    passed++;
    console.log(`ok    ${name}`);
  } else {
    failures.push(name);
    console.log(`FAIL  ${name}${detail ? `\n      ${detail}` : ""}`);
  }
}

// Helpers that run in the page.
const HELPERS = `
  window.t = {
    doc: () => document.querySelector("#doc"),
    source: () => [...document.querySelectorAll("#source li")].map((li) => li.lastChild.textContent).join("\\n") + "\\n",
    changed: () => [...document.querySelectorAll("#source li.changed")].map((li) => li.lastChild.textContent),
    status: () => document.querySelector("#status").textContent,
    card: (id) => document.querySelector('#doc .em-card[data-id="' + id + '"]'),
    click: (root, label) => {
      const button = [...(root || document).querySelectorAll("button")].find((b) => b.textContent.trim() === label);
      if (!button) throw new Error("no button: " + label);
      button.click();
    },
    tab: (label) => document.querySelector('#doc .em-tab[data-view="' + label.toLowerCase() + '"]').click(),
    select: (text) => {
      for (const block of document.querySelectorAll("#doc .em-src")) {
        const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
        while (walker.nextNode()) {
          const at = walker.currentNode.data.indexOf(text);
          if (at < 0) continue;
          const range = document.createRange();
          range.setStart(walker.currentNode, at);
          range.setEnd(walker.currentNode, at + text.length);
          const selection = getSelection();
          selection.removeAllRanges();
          selection.addRange(range);
          return true;
        }
      }
      throw new Error("text not found in the document: " + text);
    },
    fill: (element, value) => { element.value = value; },
    submit: (form) => form.requestSubmit(),
  };
`;

async function selectAnd(page: Page, text: string, kind: "Comment" | "Suggest", note: string, replacement?: string) {
  await page.evaluate(`t.select(${JSON.stringify(text)})`);
  await page.waitFor(`document.querySelector("#doc .em-popover button")`);
  await page.evaluate(`t.click(document.querySelector("#doc .em-popover"), "${kind}")`);
  await page.waitFor(`document.querySelector("#doc .em-popover form")`);
  const picked = await page.evaluate<string>(`document.querySelector("#doc .em-popover .em-picked").textContent`);
  await page.evaluate(`(() => {
    const form = document.querySelector("#doc .em-popover form");
    const areas = form.querySelectorAll("textarea");
    ${replacement === undefined ? "" : `t.fill(areas[0], ${JSON.stringify(replacement)});`}
    t.fill(areas[areas.length - 1], ${JSON.stringify(note)});
    t.submit(form);
    getSelection().removeAllRanges();
  })()`);
  return picked;
}

async function wide(port: number): Promise<void> {
  const page = await openPage(`http://127.0.0.1:${port}/`, { width: 1280, height: 900 });
  try {
    await page.waitFor(`document.documentElement.dataset.ready === "true"`);
    await page.evaluate(HELPERS);
    await page.waitFor(`document.querySelectorAll("#doc .em-diagram.em-drawn").length === 2`);
    const original = await page.evaluate<string>(`t.source()`);

    console.log("\n# The worked example, in Review view");
    const dom = await page.evaluate<string>(`document.documentElement.outerHTML`);
    check("the page rendered a DOM", dom.includes('class="em-root"'));
    check("Review is the selected view", await page.evaluate(`document.querySelector('#doc .em-tab[aria-selected="true"]').dataset.view === "review"`));
    for (const [id, cls] of [["s-4q8d", "em-suggestion"], ["c-7f3k", "em-thread"], ["c-9d1t", "em-thread"]]) {
      check(`open item ${id} is a card`, await page.evaluate(`!!t.card("${id}") && t.card("${id}").classList.contains("${cls}")`));
    }
    check("exactly three top-level cards", (await page.evaluate<number>(`document.querySelectorAll("#doc .em-side > .em-card").length`)) === 3);
    const quote = await page.evaluate<string>(`[...document.querySelectorAll('#doc mark.em-quote[data-id="c-7f3k"]')].map((m) => m.textContent).join("")`);
    check('"port 8080" is highlighted', quote === "port 8080", `highlighted text: ${JSON.stringify(quote)}`);
    check('"3" is highlighted for c-9d1t', (await page.evaluate<string>(`document.querySelector('#doc mark.em-quote[data-id="c-9d1t"]').textContent`)) === "3");
    check("the reply c-2m9x is indented in its thread", await page.evaluate(`t.card("c-7f3k").querySelector(".em-reply").textContent.includes("c-2m9x")`));
    check("the agent card shows its handle and the for person", await page.evaluate(`/agent:claude\\s+for\\s+adam/.test(t.card("s-4q8d").querySelector(".em-byline").textContent)`));
    check("removed text is struck and added text underlined, in place", await page.evaluate(`(() => {
      const del = document.querySelector('#doc p del.em-del[data-id="s-4q8d"]');
      const ins = document.querySelector('#doc p ins.em-ins[data-id="s-4q8d"]');
      return del.textContent === "8080" && ins.textContent === "8443" &&
        getComputedStyle(del).textDecorationLine.includes("line-through") && getComputedStyle(ins).textDecorationLine.includes("underline");
    })()`));
    check("the heading edit is a word diff inside the heading", await page.evaluate(`document.querySelector("#doc h2 del").textContent === "Setup" && document.querySelector("#doc h2 ins").textContent === "Installation"`));
    check("the card has the note, the edit count and the group actions", await page.evaluate(`(() => {
      const card = t.card("s-4q8d");
      const labels = [...card.querySelectorAll(":scope > .em-actions button")].map((b) => b.textContent);
      return card.querySelector(".em-note").textContent.includes("Port changed in v2") && card.querySelector(".em-edit-count").textContent === "3 edits" &&
        labels.includes("Accept") && labels.includes("Reject");
    })()`));
    check("three gutter bars, one per edited block", (await page.evaluate<number>(`document.querySelectorAll('#doc .em-bar[data-id="s-4q8d"]').length`)) === 3);
    check("the diagram is drawn twice: current and suggested", await page.evaluate(`(() => {
      const figures = document.querySelectorAll("#doc .em-pictures .em-figure");
      return figures.length === 2 && !figures[0].textContent.includes("Consumer") && figures[1].textContent.includes("Consumer") && !!figures[1].querySelector("svg");
    })()`));
    check("the added node is marked in the suggested diagram", (await page.evaluate<number>(`document.querySelectorAll("#doc .em-added-node").length`)) === 1);
    await page.evaluate(`t.click(t.doc(), "Show source diff")`);
    check("the source diff toggle shows a line diff", await page.evaluate(`(() => {
      const diffs = document.querySelector("#doc .em-sourcediffs");
      return !diffs.hidden && document.querySelector("#doc .em-pictures").hidden &&
        diffs.querySelector("del").textContent.includes("A[Webhook] --> B[Relay]") && diffs.querySelector("ins").textContent.includes("C[Consumer]");
    })()`));
    await page.evaluate(`t.click(t.doc(), "Show diagrams")`);
    check("Accept is offered once the suggested diagram has parsed", await page.evaluate(`!t.card("s-4q8d").querySelector(".em-accept").disabled && t.card("s-4q8d").querySelector(".em-accept").textContent === "Accept"`));
    check("no findings are listed", (await page.evaluate<string>(`document.querySelector("#findings").textContent`)) === "No findings.");
    check("no horizontal scroll at 1280 px", await page.evaluate(`document.documentElement.scrollWidth <= window.innerWidth`));
    check("cards sit beside the block on a wide screen", await page.evaluate(`(() => {
      const row = t.card("s-4q8d").closest(".em-row");
      return row.querySelector(".em-side").getBoundingClientRect().left >= row.querySelector(".em-main").getBoundingClientRect().right;
    })()`));
    check("nothing is in browser storage", await page.evaluate(`localStorage.length === 0 && sessionStorage.length === 0 && document.cookie === ""`));
    const loaded = page.requests.length;
    const foreign = page.requests.filter((url) => !url.startsWith(`http://127.0.0.1:${port}/`) && !url.startsWith("data:"));
    check("every request went to the built directory", foreign.length === 0, foreign.join(", "));

    console.log("\n# Views");
    await page.evaluate(`t.tab("Final")`);
    check("Final shows the body only", await page.evaluate(`!document.querySelector("#doc .em-card, #doc mark, #doc del, #doc ins") && t.doc().textContent.includes("port 8080")`));
    check("switching views leaves the source unchanged", (await page.evaluate<string>(`t.source()`)) === original);
    await page.waitFor(`document.querySelectorAll("#doc .em-diagram.em-drawn").length === 1`);
    await page.evaluate(`t.tab("Preview")`);
    check("Preview shows the body with s-4q8d applied", await page.evaluate(`t.doc().textContent.includes("port 8443") && !!document.querySelector("#doc h2") && [...document.querySelectorAll("#doc h2")].some((h) => h.textContent === "Installation")`));
    await page.evaluate(`t.tab("Review")`);
    await page.waitFor(`document.querySelectorAll("#doc .em-diagram.em-drawn").length === 2`);

    console.log("\n# Accept, revert, restore, revert again");
    await page.evaluate(`t.click(t.card("s-4q8d"), "Accept")`);
    let source = await page.evaluate<string>(`t.source()`);
    check("accept changed the body lines", source.includes("It listens on port 8443 by default.") && source.includes("\n## Installation\n") && source.includes("B[Relay] --> C[Consumer]\n```"));
    check("accept appended an accept record and an anchor record", /<!--endmark accept s-4q8d by=you at=\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ -->\n\n<!--endmark anchor c-7f3k by=you at=\S+\n```on\nport 8443\n```\n-->\n$/.test(source), source.slice(-200));
    const changed = await page.evaluate<string[]>(`t.changed()`);
    check("the changed and added lines are highlighted in the source", changed.includes("## Installation") && changed.some((line) => line.startsWith("<!--endmark accept s-4q8d")) && changed.length === 3 + 8, JSON.stringify(changed));
    check("the source scrolled to the first changed line", await page.evaluate(`(() => {
      const pane = document.querySelector("#source"); const first = pane.querySelector("li.changed");
      const top = first.getBoundingClientRect().top; const box = pane.getBoundingClientRect();
      return top >= box.top && top <= box.bottom;
    })()`));
    check("c-7f3k is attached again, to port 8443", await page.evaluate(`document.querySelector('#doc mark.em-quote[data-id="c-7f3k"]').textContent === "port 8443"`));
    check("s-4q8d is no longer an open card", await page.evaluate(`!t.card("s-4q8d")`));

    await page.evaluate(`t.tab("History")`);
    check("History lists closed items newest first", (await page.evaluate<string>(`[...document.querySelectorAll("#doc .em-history-item")].map((c) => c.dataset.id).join(" ")`)) === "s-4q8d s-3c7j e-0h5n");
    await page.evaluate(`t.card("s-4q8d").querySelector(".em-card-head").click()`);
    const landed = await page.evaluate<string>(`[...document.querySelectorAll("#doc .em-landed")].map((m) => m.textContent).join("|")`);
    check("selecting a change highlights where it landed", landed === "port 8443|Installation", landed);
    check("a change that landed in a diagram outlines the diagram", (await page.evaluate<number>(`document.querySelectorAll("#doc .em-diagram.em-landed-block").length`)) === 1);
    check("a change in effect offers Revert and not Restore", await page.evaluate(`[...t.card("s-4q8d").querySelectorAll("button")].map((b) => b.textContent).join() === "Revert"`));
    await page.evaluate(`t.click(t.card("s-4q8d"), "Revert")`);
    source = await page.evaluate<string>(`t.source()`);
    const first = /<!--endmark revert (r-[0-9abcdefghjkmnpqrstvwxyz]{4}) re=s-4q8d by=you /.exec(source)?.[1];
    check("revert appended a revert record that names s-4q8d", !!first, source.slice(-400));
    check("the body lines returned to their earlier text", source.startsWith(original.slice(0, original.indexOf("<!--endmark"))));
    check("the suggestion's record is still there", source.includes("<!--endmark suggest s-4q8d ") && source.includes("<!--endmark accept s-4q8d "));
    check("the suggestion is shown as reverted, with the revert that undid it", await page.evaluate(`(() => {
      const card = t.card("s-4q8d");
      return card.querySelector(".em-chip.em-warn").textContent.includes("reverted") && card.querySelector(".em-reverted-by").textContent.includes("${first}");
    })()`));
    check("a change not in effect offers Restore and not Revert", await page.evaluate(`[...t.card("s-4q8d").querySelectorAll("button")].map((b) => b.textContent).join() === "Restore"`));
    check("the revert itself offers Revert", await page.evaluate(`[...t.card("${first}").querySelectorAll("button")].map((b) => b.textContent).join() === "Revert"`));

    await page.evaluate(`t.click(t.card("s-4q8d"), "Restore")`);
    source = await page.evaluate<string>(`t.source()`);
    const second = new RegExp(`<!--endmark revert (r-\\S{4}) re=${first} by=you `).exec(source)?.[1];
    check("restore appended a second revert that names the first", !!second, source.slice(-400));
    check("the body lines changed forward again", source.includes("It listens on port 8443 by default.") && source.includes("\n## Installation\n"));
    check("the suggestion reads as in effect again, still marked with its revert", await page.evaluate(`t.card("s-4q8d").querySelector(".em-chip.em-ok").textContent === "in effect again" && t.card("s-4q8d").querySelector(".em-reverted-by").textContent.includes("${first}")`));

    await page.evaluate(`t.click(t.card("s-4q8d"), "Revert")`);
    source = await page.evaluate<string>(`t.source()`);
    const third = new RegExp(`<!--endmark revert (r-\\S{4}) re=${second} by=you `).exec(source)?.[1];
    check("a second revert of the suggestion names the second revert, not the suggestion", !!third && (source.match(/re=s-4q8d /g) ?? []).length === 1, source.slice(-400));
    check("the status line says which record was named", (await page.evaluate<string>(`t.status()`)).includes(`names ${second}`), await page.evaluate<string>(`t.status()`));
    check("the linter has no findings after three reverts", (await page.evaluate<string>(`document.querySelector("#findings").textContent`)) === "No findings.");

    console.log("\n# Reset");
    await page.evaluate(`document.querySelector("#reset").click()`);
    check("Reset returns the original example", (await page.evaluate<string>(`t.source()`)) === original);
    check("Reset returns to Review", await page.evaluate(`!!t.card("s-4q8d") && document.querySelector('#doc .em-tab[aria-selected="true"]').dataset.view === "review"`));

    console.log("\n# Select text to comment and to suggest");
    let picked = await selectAnd(page, "webhook events", "Comment", "Which events?");
    source = await page.evaluate<string>(`t.source()`);
    check("commenting on a selection appends a comment with the selection as its quote", picked === "webhook events" && /<!--endmark comment c-\S{4} by=you at=\S+\n```on\nwebhook events\n```\nWhich events\?\n-->\n$/.test(source), source.slice(-200));
    check("the new comment is highlighted and has a card", await page.evaluate(`[...document.querySelectorAll("#doc mark.em-quote")].some((m) => m.textContent === "webhook events") && document.querySelectorAll("#doc .em-thread").length === 3`));

    await page.evaluate(`t.select("RELAY_TOKEN")`);
    picked = await selectAnd(page, "relay", "Comment", "Which binary?");
    source = await page.evaluate<string>(`t.source()`);
    check("a selection that is not unique is written with an in scope", /```in\n1\. Install the `relay` binary\.\n```\n```on\nrelay\n```\nWhich binary\?\n-->\n$/.test(source), source.slice(-200));

    const beforeSuggest = source;
    picked = await selectAnd(page, "5 second", "Suggest", "Give slow consumers time.", "10 second");
    source = await page.evaluate<string>(`t.source()`);
    const mine = /<!--endmark suggest (s-\S{4}) by=you at=\S+\nGive slow consumers time\.\n\n```replace\n5 second\n```\n```with\n10 second\n```\n-->\n$/.exec(source)?.[1];
    check("suggesting a replacement appends a suggest record", !!mine, source.slice(-200));
    check("the body lines are unchanged by a suggestion", source.startsWith(beforeSuggest));
    check("the new suggestion is drawn in place, in its own colour slot", await page.evaluate(`(() => {
      const ins = document.querySelector('#doc ins.em-ins[data-id="${mine}"]');
      return ins.textContent === "10" && ins.classList.contains("em-s1") && t.card("${mine}").classList.contains("em-s1");
    })()`));
    check("my own suggestion offers Retract; another person's does not", await page.evaluate(`[...t.card("${mine}").querySelectorAll("button")].some((b) => b.textContent === "Retract") && ![...t.card("s-4q8d").querySelectorAll(":scope > .em-actions button")].some((b) => b.textContent === "Retract")`));
    await page.evaluate(`t.click(t.card("${mine}"), "Retract")`);
    source = await page.evaluate<string>(`t.source()`);
    check("retract appends a retract record", new RegExp(`<!--endmark retract ${mine} by=you at=\\S+ -->\\n$`).test(source), source.slice(-120));

    console.log("\n# Reply, resolve, reopen, reject");
    await page.evaluate(`(() => {
      const card = t.card("c-9d1t");
      t.click(card, "Reply");
      t.fill(card.querySelector("textarea"), "Yes, as **RELAY_RETRIES**. <b>raw</b>");
      t.submit(card.querySelector("form"));
    })()`);
    source = await page.evaluate<string>(`t.source()`);
    check("reply appends a comment with re", /<!--endmark comment c-\S{4} re=c-9d1t by=you at=\S+\nYes, as \*\*RELAY_RETRIES\*\*\. <b>raw<\/b>\n-->\n$/.test(source), source.slice(-200));
    check("notes are Markdown with raw HTML disabled", await page.evaluate(`(() => {
      const note = t.card("c-9d1t").querySelector(".em-reply .em-note");
      return !!note.querySelector("strong") && !note.querySelector("b") && note.textContent.includes("<b>raw</b>");
    })()`));
    await page.evaluate(`t.click(t.card("c-9d1t"), "Resolve")`);
    source = await page.evaluate<string>(`t.source()`);
    check("resolve appends a resolve record", /<!--endmark resolve c-9d1t by=you at=\S+ -->\n$/.test(source));
    check("a resolved thread leaves Review", await page.evaluate(`!t.card("c-9d1t") && !document.querySelector('#doc mark[data-id="c-9d1t"]')`));
    await page.evaluate(`t.tab("History")`);
    await page.evaluate(`t.click(t.card("c-9d1t"), "Reopen")`);
    source = await page.evaluate<string>(`t.source()`);
    check("reopen appends a reopen record", /<!--endmark reopen c-9d1t by=you at=\S+ -->\n$/.test(source));
    await page.evaluate(`t.tab("Review")`);
    check("the reopened thread is back in Review", await page.evaluate(`!!t.card("c-9d1t")`));

    console.log("\n# A refused write");
    await page.evaluate(`endmarkDemo.actions.retract("c-7f3k")`);
    check("a refusal shows its message and hint", await page.evaluate(`document.querySelector("#status").classList.contains("refused") && t.status().includes("Refused:") && t.status().includes("not-author")`), await page.evaluate<string>(`t.status()`));
    console.log(`      status: ${await page.evaluate<string>(`t.status()`)}`);
    check("a refused write leaves the source unchanged", (await page.evaluate<string>(`t.source()`)) === source);

    console.log("\n# A suggestion whose diagram does not parse");
    await page.evaluate(`endmarkDemo.actions.suggest({ quote: "graph LR", scopes: [], replacement: "graph LR\\n  A[Webhook --> ", note: "Broken on purpose." })`);
    await page.waitFor(`document.querySelector("#doc .em-figure .em-parse-error")`);
    check("the parser error is shown in place of the picture", await page.evaluate(`(() => {
      const figure = document.querySelector("#doc .em-figure .em-parse-error").closest(".em-figure");
      return !figure.querySelector("svg") && figure.querySelector(".em-parse-error pre").textContent.length > 10;
    })()`));
    const broken = await page.evaluate<string>(`[...document.querySelectorAll("#doc .em-suggestion")].find((c) => c.textContent.includes("Broken on purpose")).dataset.id`);
    check("accepting it needs confirmation", await page.evaluate(`t.card("${broken}").querySelector(".em-accept").textContent === "Accept anyway"`));
    const beforeConfirm = await page.evaluate<string>(`t.source()`);
    await page.evaluate(`t.card("${broken}").querySelector(".em-accept").click()`);
    check("the first click asks and writes nothing", (await page.evaluate<string>(`t.source()`)) === beforeConfirm && (await page.evaluate<string>(`t.card("${broken}").querySelector(".em-accept").textContent`)).startsWith("Confirm"));
    check("two suggestions on one diagram are drawn as alternatives", (await page.evaluate<number>(`document.querySelectorAll("#doc .em-pictures .em-figure").length`)) === 3);
    await page.evaluate(`t.click(t.card("${broken}"), "Reject")`);
    source = await page.evaluate<string>(`t.source()`);
    check("reject appends a reject record", new RegExp(`<!--endmark reject ${broken} by=you at=\\S+ -->\\n$`).test(source));

    console.log("\n# A direct body edit");
    const beforeEdit = source;
    await page.evaluate(`document.querySelector("#edit-body").click()`);
    check("the body dialog opens with the body text", await page.evaluate(`document.querySelector("#body-dialog").open && document.querySelector("#body-text").value.startsWith("# Relay") && !document.querySelector("#body-text").value.includes("endmark")`));
    await page.evaluate(`(() => {
      const area = document.querySelector("#body-text");
      area.value = area.value.replace("port 8080", "port 9000");
      t.click(document.querySelector("#body-form"), "Apply");
    })()`);
    // The dialog's close event arrives on a later task.
    await page.waitFor(`t.status().startsWith("Body edited directly")`);
    source = await page.evaluate<string>(`t.source()`);
    const want = beforeEdit.replace("port 8080 by default", "port 9000 by default");
    let at = 0;
    while (at < want.length && want[at] === source[at]) at++;
    check("only the body changed", source === want, `first difference at ${at}: ${JSON.stringify(source.slice(at - 20, at + 40))} vs ${JSON.stringify(want.slice(at - 20, at + 40))}`);
    check("only the edited line is highlighted", JSON.stringify(await page.evaluate(`t.changed()`)) === JSON.stringify(["Relay forwards webhook events to internal consumers. It listens on port 9000 by default."]));
    check("the comment whose quote was edited moved to the tray, with its original quote as a blockquote", await page.evaluate(`(() => {
      const card = document.querySelector('#doc .em-tray .em-thread[data-id="c-7f3k"]');
      return !!card && card.querySelector("blockquote pre").textContent === "port 8080" && card.textContent.includes("detached");
    })()`));
    check("the stale suggestion is in the tray and offers no Accept", await page.evaluate(`(() => {
      const card = document.querySelector('#doc .em-tray .em-suggestion[data-id="s-4q8d"]');
      const labels = [...card.querySelectorAll("button")].map((b) => b.textContent);
      return card.querySelector("blockquote pre").textContent === "port 8080" && !labels.some((l) => /accept/i.test(l)) && labels.includes("Reject");
    })()`));
    const findings = await page.evaluate<string>(`document.querySelector("#findings").textContent`);
    check("the linter's findings appear under the source", findings.includes("detached") && findings.includes("c-7f3k") || findings.includes("port 8080"), findings);
    console.log(`      findings: ${findings}`);
    check("the renderer reports the same findings", await page.evaluate(`document.querySelector("#doc .em-problems").textContent.includes("detached")`));

    console.log("\n# A malformed record");
    await page.evaluate(`endmarkDemo.renderer.update(endmarkDemo.session.source + "\\n<!--endmark comment c-bad1 at=2026-11-20T10:00:00Z\\nNo author.\\n-->\\n")`);
    const problems = await page.evaluate<string>(`document.querySelector("#doc .em-problems").textContent`);
    check("a malformed record is reported", /missing|by/.test(problems) && problems.includes("error"), problems);
    console.log(`      problems: ${problems}`);
    await page.evaluate(`endmarkDemo.renderer.update(endmarkDemo.session.source)`);

    console.log("\n# After everything");
    await Bun.sleep(300);
    check("no request was made after the page loaded", page.requests.length === loaded, page.requests.slice(loaded).join(", "));
    check("still nothing in browser storage", await page.evaluate(`localStorage.length === 0 && sessionStorage.length === 0 && document.cookie === ""`));
    check("no console errors and no uncaught exceptions", page.errors.length === 0, page.errors.join("\n      "));
  } finally {
    await page.close();
  }
}

async function narrow(port: number): Promise<void> {
  const page = await openPage(`http://127.0.0.1:${port}/`, { width: 375, height: 800 });
  try {
    await page.waitFor(`document.documentElement.dataset.ready === "true"`);
    await page.evaluate(HELPERS);
    await page.waitFor(`document.querySelectorAll("#doc .em-diagram.em-drawn").length === 2`);
    console.log("\n# Phone width");
    check("no horizontal scroll at 375 px", await page.evaluate(`document.documentElement.scrollWidth <= window.innerWidth`), await page.evaluate<string>(`document.documentElement.scrollWidth + " > " + window.innerWidth`));
    check("the panes are stacked, document first", await page.evaluate(`document.querySelector("#doc").getBoundingClientRect().bottom <= document.querySelector("#source").getBoundingClientRect().top`));
    check("cards sit under the block on a narrow screen", await page.evaluate(`(() => {
      const row = t.card("s-4q8d").closest(".em-row");
      return row.querySelector(".em-side").getBoundingClientRect().top >= row.querySelector(".em-main").getBoundingClientRect().bottom;
    })()`));
    check("the two diagrams are stacked", await page.evaluate(`(() => {
      const figures = document.querySelectorAll("#doc .em-pictures .em-figure");
      return figures[1].getBoundingClientRect().top >= figures[0].getBoundingClientRect().bottom;
    })()`));
    await page.evaluate(`t.tab("History")`);
    check("no horizontal scroll in History at 375 px", await page.evaluate(`document.documentElement.scrollWidth <= window.innerWidth`));
    check("no console errors at phone width", page.errors.length === 0, page.errors.join("\n      "));
  } finally {
    await page.close();
  }
}

const server = serve(0);
try {
  await wide(server.port!);
  await narrow(server.port!);
} catch (error) {
  failures.push(`the run stopped: ${error instanceof Error ? error.message : String(error)}`);
  console.log(`FAIL  the run stopped: ${error instanceof Error ? (error.stack ?? error.message) : String(error)}`);
} finally {
  server.stop(true);
}

console.log(`\n${passed} passed, ${failures.length} failed`);
for (const failure of failures) console.log(`  failed: ${failure}`);
process.exit(failures.length === 0 ? 0 : 1);
