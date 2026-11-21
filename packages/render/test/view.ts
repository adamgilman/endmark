// A test of `endmark view` in headless Chromium.
// Run with `bun run smoke:view` in packages/render, after `bun run build`.
//
// It builds the `endmark` binary, starts `endmark view` on a copy of the
// worked example, and uses the page as a person would: it clicks the real
// buttons and selects real text, and then reads the file on disk. Set
// CHROMIUM to the browser binary if it is not `chromium`, and ENDMARK_BIN
// to use a binary that is already built.

import { $ } from "bun";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { openPage, type Page } from "./cdp";

const root = resolve(import.meta.dir, "../../..");
let binary = process.env.ENDMARK_BIN;
if (!binary) {
  await $`cargo build -p endmark`.cwd(root);
  binary = join(root, "target/debug/endmark");
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

/** Runs the command-line tool, as another terminal would while the page is open. */
function endmark(...args: string[]): void {
  const result = Bun.spawnSync([binary!, ...args], { env: { ...process.env, ENDMARK_BY: "" } });
  if (result.exitCode !== 0) throw new Error(`endmark ${args.join(" ")}: ${result.stderr.toString()}`);
}

interface Server {
  port: number;
  origin: string;
  stop(): Promise<void>;
}

async function view(file: string, ...options: string[]): Promise<Server> {
  const child = Bun.spawn([binary!, "view", "--port", "0", ...options, file], {
    env: { ...process.env, ENDMARK_BY: "" },
    stdout: "pipe",
    stderr: "inherit",
  });
  const reader = child.stdout.getReader();
  let seen = "";
  while (!seen.includes("\n")) {
    const { value, done } = await reader.read();
    if (done) throw new Error(`endmark view stopped before it said where it serves: ${seen}`);
    seen += new TextDecoder().decode(value);
  }
  reader.releaseLock();
  const match = /^Serving .* at (http:\/\/127\.0\.0\.1:(\d+))\/ \(Ctrl-C to stop\)$/m.exec(seen);
  if (!match) throw new Error(`unexpected first line: ${seen}`);
  return {
    port: Number(match[2]),
    origin: match[1]!,
    async stop() {
      child.kill();
      await child.exited;
    },
  };
}

// Helpers that run in the page.
const HELPERS = `
  window.t = {
    doc: () => document.querySelector("#doc"),
    status: () => document.querySelector("#status").textContent,
    card: (id) => document.querySelector('#doc .em-card[data-id="' + id + '"]'),
    buttons: () => [...document.querySelectorAll("#doc button:not(.em-tab)")].map((b) => b.textContent.trim()),
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
    violations: [],
  };
  document.addEventListener("securitypolicyviolation", (event) => t.violations.push(event.violatedDirective + " " + event.blockedURI));
`;

async function open(server: Server, size = { width: 1280, height: 900 }): Promise<Page> {
  const page = await openPage(`${server.origin}/`, size);
  await page.waitFor(`document.documentElement.dataset.ready === "true"`);
  await page.evaluate(HELPERS);
  return page;
}

/** Waits until the file on disk matches. */
async function onDisk(file: string, pattern: RegExp, timeoutMs = 10_000): Promise<string> {
  const end = Date.now() + timeoutMs;
  for (;;) {
    const text = await readFile(file, "utf8");
    if (pattern.test(text) || Date.now() > end) return text;
    await Bun.sleep(50);
  }
}

/** Selects text and opens the comment or the suggestion form on it. */
async function openForm(page: Page, text: string, kind: "Comment" | "Suggest"): Promise<void> {
  await page.evaluate(`t.select(${JSON.stringify(text)})`);
  await page.waitFor(`document.querySelector("#doc .em-popover button")`);
  await page.evaluate(`t.click(document.querySelector("#doc .em-popover"), "${kind}")`);
  await page.waitFor(`document.querySelector("#doc .em-popover form")`);
}

const foreign = (page: Page, server: Server) =>
  page.requests.filter((url) => !url.startsWith(`${server.origin}/`) && !url.startsWith("data:"));

async function writable(dir: string): Promise<void> {
  const file = join(dir, "notes.md");
  await writeFile(file, await readFile(join(root, "conformance/worked-example.md")));
  const original = await readFile(file, "utf8");
  const server = await view(file, "--by", "adam");
  const page = await open(server);
  try {
    console.log("\n# The worked example, served from disk");
    await page.waitFor(`document.querySelectorAll("#doc .em-diagram.em-drawn").length === 2`);
    check("the header names the file", (await page.evaluate<string>(`document.querySelector("#file").textContent`)) === file);
    check("the title names the file", (await page.evaluate<string>(`document.title`)) === "notes.md - Endmark");
    check("the header says who is writing", (await page.evaluate<string>(`document.querySelector("#who").textContent`)) === "Writing as adam");
    check("Review is the selected view", await page.evaluate(`document.querySelector('#doc .em-tab[aria-selected="true"]').dataset.view === "review"`));
    for (const [id, cls] of [["s-4q8d", "em-suggestion"], ["c-7f3k", "em-thread"], ["c-9d1t", "em-thread"]]) {
      check(`open item ${id} is a card`, await page.evaluate(`!!t.card("${id}") && t.card("${id}").classList.contains("${cls}")`));
    }
    check('"port 8080" is highlighted', (await page.evaluate<string>(`[...document.querySelectorAll('#doc mark.em-quote[data-id="c-7f3k"]')].map((m) => m.textContent).join("")`)) === "port 8080");
    check("the diagram is drawn twice, under the page's content security policy", await page.evaluate(`document.querySelectorAll("#doc .em-pictures .em-figure svg").length === 2`));
    check("the four views are offered", (await page.evaluate<string>(`[...document.querySelectorAll("#doc .em-tab")].map((b) => b.dataset.view).join()`)) === "final,review,preview,history");
    check("there is no source pane", await page.evaluate(`!document.querySelector("#source") && !document.body.textContent.includes("<!--endmark")`));
    check("no horizontal scroll at 1280 px", await page.evaluate(`document.documentElement.scrollWidth <= window.innerWidth`));
    check("nothing is in browser storage", await page.evaluate(`localStorage.length === 0 && sessionStorage.length === 0 && document.cookie === ""`));
    check("the token is not in the address", !(await page.evaluate<string>(`location.href`)).includes(await page.evaluate<string>(`document.querySelector('meta[name="endmark-token"]').content`)));

    console.log("\n# Views");
    await page.evaluate(`t.tab("Final")`);
    check("Final shows the body only", await page.evaluate(`!document.querySelector("#doc .em-card, #doc mark, #doc del, #doc ins") && t.doc().textContent.includes("port 8080")`));
    await page.evaluate(`t.tab("Preview")`);
    check("Preview shows the body with s-4q8d applied", await page.evaluate(`t.doc().textContent.includes("port 8443") && [...document.querySelectorAll("#doc h2")].some((h) => h.textContent === "Installation")`));
    await page.evaluate(`t.tab("History")`);
    check("History lists the closed items", (await page.evaluate<string>(`[...document.querySelectorAll("#doc .em-history-item")].map((c) => c.dataset.id).join(" ")`)) === "s-3c7j e-0h5n");
    await page.evaluate(`t.tab("Review")`);
    await page.waitFor(`document.querySelectorAll("#doc .em-diagram.em-drawn").length === 2`);
    check("looking at the views wrote nothing", (await readFile(file, "utf8")) === original);

    console.log("\n# Accept writes to the file on disk");
    await page.waitFor(`!t.card("s-4q8d").querySelector(".em-accept").disabled`);
    await page.evaluate(`t.click(t.card("s-4q8d"), "Accept")`);
    let source = await onDisk(file, /<!--endmark accept s-4q8d /);
    check("the body lines on disk changed", source.includes("It listens on port 8443 by default.") && source.includes("\n## Installation\n"));
    check("an accept record and an anchor record were appended, by adam", /<!--endmark accept s-4q8d by=adam at=\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ -->\n\n<!--endmark anchor c-7f3k by=adam at=\S+ line=3\n```on\nport 8443\n```\n```near\n[^\n]+\n```\n-->\n$/.test(source), source.slice(-200));
    await page.waitFor(`!t.card("s-4q8d")`);
    check("the page shows the file as written", await page.evaluate(`document.querySelector('#doc mark.em-quote[data-id="c-7f3k"]').textContent === "port 8443"`));
    check("the status line says what was done", (await page.evaluate<string>(`t.status()`)).startsWith("Accepted s-4q8d"), await page.evaluate<string>(`t.status()`));

    console.log("\n# A comment on a selection");
    await openForm(page, "webhook events", "Comment");
    await page.evaluate(`(() => {
      const form = document.querySelector("#doc .em-popover form");
      form.querySelector("textarea").value = "Which events?";
      form.requestSubmit();
    })()`);
    source = await onDisk(file, /Which events\?/);
    check("a comment record with the selection as its quote is on disk", /<!--endmark comment c-\S{4} by=adam at=\S+ line=3\n```on\nwebhook events\n```\n```near\n[^\n]+\n```\nWhich events\?\n-->\n$/.test(source), source.slice(-200));
    await page.waitFor(`!document.querySelector("#doc .em-popover")`);
    check("the new comment is highlighted and has a card", await page.evaluate(`[...document.querySelectorAll("#doc mark.em-quote")].some((m) => m.textContent === "webhook events") && document.querySelectorAll("#doc .em-thread").length === 3`));

    await page.evaluate(`getSelection().removeAllRanges()`);
    await openForm(page, "relay", "Comment");
    await page.evaluate(`(() => {
      const form = document.querySelector("#doc .em-popover form");
      form.querySelector("textarea").value = "Which binary?";
      form.requestSubmit();
    })()`);
    source = await onDisk(file, /Which binary\?/);
    check("a selection that is not unique is written with an in scope", /```in\n1\. Install the `relay` binary\.\n```\n```on\nrelay\n```/.test(source), source.slice(-300));
    await page.waitFor(`!document.querySelector("#doc .em-popover")`);

    console.log("\n# The page follows the file");
    let started = Date.now();
    endmark("reply", "--by", "maria", "--note", "Written from **another terminal**.", file, "c-9d1t");
    await page.waitFor(`t.card("c-9d1t").textContent.includes("another terminal")`, 5000);
    const took = Date.now() - started;
    check("a record written by another program appears without a reload", took < 3000, `it took ${took} ms`);
    console.log(`      it appeared after ${took} ms`);
    check("the status line says the file changed", (await page.evaluate<string>(`t.status()`)).startsWith("The file changed on disk"));

    console.log("\n# A half-typed reply survives a change on disk");
    await page.evaluate(`(() => {
      const card = t.card("c-9d1t");
      t.click(card, "Reply");
      const area = card.querySelector(":scope > form textarea");
      area.value = "Half typed, and then";
      area.focus();
      area.setSelectionRange(4, 9);
    })()`);
    endmark("comment", "--by", "maria", "--on", "RELAY_TOKEN", "--note", "While you were typing.", file);
    await page.waitFor(`document.querySelector("#doc").textContent.includes("While you were typing.")`, 5000);
    check("the reply form is still open, with its text", (await page.evaluate<string>(`t.card("c-9d1t").querySelector(":scope > form textarea")?.value`)) === "Half typed, and then");
    check("the caret is where it was", await page.evaluate(`(() => {
      const area = t.card("c-9d1t").querySelector(":scope > form textarea");
      return document.activeElement === area && area.selectionStart === 4 && area.selectionEnd === 9;
    })()`));
    await page.evaluate(`t.card("c-9d1t").querySelector(":scope > form").requestSubmit()`);
    source = await onDisk(file, /Half typed, and then/);
    check("sending it writes the reply", /<!--endmark comment c-\S{4} re=c-9d1t by=adam at=\S+\nHalf typed, and then\n-->\n$/.test(source), source.slice(-200));
    await page.waitFor(`!t.card("c-9d1t").querySelector(":scope > form")`);
    check("the form closes once the reply is written", await page.evaluate(`t.card("c-9d1t").textContent.includes("Half typed, and then")`));

    console.log("\n# A half-typed comment on a selection survives too, and a refusal keeps it");
    await page.evaluate(`getSelection().removeAllRanges()`);
    await openForm(page, "5 second", "Suggest");
    await page.evaluate(`(() => {
      const areas = document.querySelectorAll("#doc .em-popover form textarea");
      areas[0].value = "10 second";
      areas[1].value = "Give slow consumers time.";
    })()`);
    endmark("edit", "--by", "maria", "--replace", "5 second", "--with", "6 second", file);
    await page.waitFor(`document.querySelector("#doc .em-src") && t.doc().textContent.includes("6 second")`, 5000);
    check("the suggestion form is still open, with its text", await page.evaluate(`(() => {
      const areas = document.querySelectorAll("#doc .em-popover form textarea");
      return areas.length === 2 && areas[0].value === "10 second" && areas[1].value === "Give slow consumers time.";
    })()`));
    const beforeRefusal = await readFile(file, "utf8");
    await page.evaluate(`document.querySelector("#doc .em-popover form").requestSubmit()`);
    await page.waitFor(`document.querySelector("#status").classList.contains("refused")`);
    const refusal = await page.evaluate<string>(`t.status()`);
    check("the refusal shows its message, hint and code", refusal.startsWith("Refused:") && refusal.includes("5 second") && refusal.includes("quote-not-found") && /\. [A-Z]/.test(refusal), refusal);
    console.log(`      status: ${refusal}`);
    check("a refused write leaves the file alone", (await readFile(file, "utf8")) === beforeRefusal);
    check("the refused form keeps what was typed", await page.evaluate(`document.querySelectorAll("#doc .em-popover form textarea")[1]?.value === "Give slow consumers time."`));
    await page.evaluate(`t.click(document.querySelector("#doc .em-popover"), "Cancel"); getSelection().removeAllRanges()`);

    await page.evaluate(`endmarkView.actions.retract("c-7f3k")`);
    await page.waitFor(`t.status().includes("not-author")`);
    check("retracting another person's comment is refused by the server", (await readFile(file, "utf8")) === beforeRefusal);

    console.log("\n# Suggest, retract, resolve, reopen, revert");
    await openForm(page, "6 second", "Suggest");
    await page.evaluate(`(() => {
      const form = document.querySelector("#doc .em-popover form");
      const areas = form.querySelectorAll("textarea");
      areas[0].value = "10 second";
      areas[1].value = "Give slow consumers time.";
      form.requestSubmit();
    })()`);
    source = await onDisk(file, /```with\n10 second\n```\n-->\n$/);
    const mine = /<!--endmark suggest (s-\S{4}) by=adam at=\S+\nGive slow consumers time\.\n\n```replace\n6 second\n```\n```with\n10 second\n```\n-->\n$/.exec(source)?.[1];
    check("a suggest record is on disk", !!mine, source.slice(-200));
    await page.waitFor(`t.card("${mine}")`);
    check("my own suggestion offers Retract", await page.evaluate(`[...t.card("${mine}").querySelectorAll("button")].some((b) => b.textContent === "Retract")`));
    await page.evaluate(`t.click(t.card("${mine}"), "Retract")`);
    source = await onDisk(file, new RegExp(`<!--endmark retract ${mine} `));
    check("retract appends a retract record", new RegExp(`<!--endmark retract ${mine} by=adam at=\\S+ -->\\n$`).test(source), source.slice(-120));

    await page.waitFor(`!t.card("${mine}")`);
    await page.evaluate(`t.click(t.card("c-9d1t"), "Resolve")`);
    source = await onDisk(file, /<!--endmark resolve c-9d1t /);
    check("resolve appends a resolve record", /<!--endmark resolve c-9d1t by=adam at=\S+ -->\n$/.test(source));
    await page.waitFor(`!t.card("c-9d1t")`);
    await page.evaluate(`t.tab("History")`);
    await page.evaluate(`t.click(t.card("c-9d1t"), "Reopen")`);
    source = await onDisk(file, /<!--endmark reopen c-9d1t /);
    check("reopen appends a reopen record", /<!--endmark reopen c-9d1t by=adam at=\S+ -->\n$/.test(source));

    await page.waitFor(`t.card("s-4q8d") && [...t.card("s-4q8d").querySelectorAll("button")].some((b) => b.textContent === "Revert")`);
    await page.evaluate(`t.click(t.card("s-4q8d"), "Revert")`);
    source = await onDisk(file, /<!--endmark revert r-\S{4} re=s-4q8d /);
    const revert = /<!--endmark revert (r-[0-9abcdefghjkmnpqrstvwxyz]{4}) re=s-4q8d by=adam /.exec(source)?.[1];
    check("revert appends a revert record that names s-4q8d", !!revert, source.slice(-300));
    check("the body lines on disk returned to their earlier text", source.includes("It listens on port 8080 by default.") && source.includes("\n## Setup\n"));
    await page.waitFor(`t.card("s-4q8d").querySelector(".em-reverted-by")`);
    check("the suggestion is shown as reverted, and offers Restore", await page.evaluate(`t.card("s-4q8d").querySelector(".em-reverted-by").textContent.includes("${revert}") && [...t.card("s-4q8d").querySelectorAll("button")].map((b) => b.textContent).join() === "Restore"`));
    await page.evaluate(`t.click(t.card("s-4q8d"), "Restore")`);
    source = await onDisk(file, new RegExp(`<!--endmark revert r-\\S{4} re=${revert} `));
    check("restore appends a revert of the revert", new RegExp(`<!--endmark revert r-\\S{4} re=${revert} by=adam `).test(source), source.slice(-300));

    console.log("\n# A reject");
    endmark("suggest", "--by", "maria", "--id", "s-m4r1", "--replace", "internal consumers", "--with", "consumers", file);
    await page.evaluate(`t.tab("Review")`);
    await page.waitFor(`t.card("s-m4r1")`, 5000);
    await page.evaluate(`t.click(t.card("s-m4r1"), "Reject")`);
    source = await onDisk(file, /<!--endmark reject s-m4r1 /);
    check("reject appends a reject record", /<!--endmark reject s-m4r1 by=adam at=\S+ -->\n$/.test(source));

    console.log("\n# After everything");
    const lint = Bun.spawnSync([binary!, "lint", "--strict", file]);
    check("the file the page wrote passes the linter", lint.exitCode === 0, lint.stdout.toString());
    await Bun.sleep(1500);
    check("every request went to the server itself", foreign(page, server).length === 0, foreign(page, server).join(", "));
    const paths = new Set(page.requests.map((url) => new URL(url).pathname));
    check("the page asked for the file and its version, and wrote", paths.has("/api/file") && paths.has("/api/version") && paths.has("/api/write"));
    check("still nothing in browser storage", await page.evaluate(`localStorage.length === 0 && sessionStorage.length === 0 && document.cookie === ""`));
    check("no console errors and no uncaught exceptions", page.errors.length === 0, page.errors.join("\n      "));
    check("nothing the page did broke its content security policy", (await page.evaluate<string[]>(`t.violations`)).length === 0, (await page.evaluate<string[]>(`t.violations`)).join(", "));

    console.log("\n# The content security policy holds (these are meant to be blocked)");
    const errorsBefore = page.errors.length;
    await page.evaluate(`(() => {
      const script = document.createElement("script");
      script.textContent = "window.inlineRan = true";
      document.head.append(script);
      const remote = document.createElement("script");
      remote.src = "http://example.invalid/x.js";
      document.head.append(remote);
      const image = document.createElement("img");
      image.src = "http://example.invalid/x.png";
      document.body.append(image);
      fetch("http://example.invalid/data").catch(() => {});
      const frame = document.createElement("iframe");
      frame.src = "http://example.invalid/";
      document.body.append(frame);
    })()`);
    await page.waitFor(`t.violations.length >= 5`, 5000).catch(() => {});
    const violations = await page.evaluate<string[]>(`t.violations`);
    check("an inline script does not run", await page.evaluate(`window.inlineRan === undefined`));
    check("a script, an image, a fetch and a frame from elsewhere are blocked", ["script-src", "img-src", "connect-src", "frame-src"].every((d) => violations.some((v) => v.startsWith(d) || (d !== "script-src" && v.startsWith("default-src")))) && violations.length >= 5, violations.join(" | "));
    const sent = foreign(page, server).filter((url) => page.blocked.get(url) !== "csp");
    check("the browser sent none of them", sent.length === 0, sent.join(", "));
    check("the browser reported them", page.errors.length > errorsBefore);

    console.log("\n# The server stops");
    await server.stop();
    await page.evaluate(`endmarkView.actions.resolve("c-7f3k")`);
    await page.waitFor(`t.status().includes("did not answer")`, 5000);
    check("the page says the server is gone", (await page.evaluate<string>(`t.status()`)).startsWith("Not written:The server did not answer. It may have stopped; start endmark view again"), await page.evaluate<string>(`t.status()`));
  } finally {
    await page.close();
    await server.stop();
  }
}

async function readOnly(dir: string): Promise<void> {
  const file = join(dir, "read-only.md");
  await writeFile(file, await readFile(join(root, "conformance/worked-example-accepted.md")));
  // One open suggestion, so that Review has both kinds of card.
  endmark("suggest", "--by", "maria", "--id", "s-m4r1", "--replace", "internal consumers", "--with", "consumers", file);
  const original = await readFile(file, "utf8");
  const server = await view(file, "--read-only");
  const page = await open(server);
  try {
    console.log("\n# Read-only");
    check("the header says read-only", (await page.evaluate<string>(`document.querySelector("#who").textContent`)).startsWith("read-only"));
    check("the cards are drawn", await page.evaluate(`!!t.card("s-m4r1") && !!t.card("c-7f3k")`));
    const writing = /^(Accept|Accept anyway|Reject|Retract|Reply|Comment|Suggest|Resolve|Reopen|Revert|Restore|Send)$/;
    let labels = await page.evaluate<string[]>(`t.buttons()`);
    check("Review has no button that writes", labels.length > 0 && !labels.some((l) => writing.test(l)), labels.join(", "));
    check("the only button on a card changes the view", labels.every((l) => l === "Preview" || l === "Show source diff"), labels.join(", "));
    await page.evaluate(`t.select("webhook events")`);
    await Bun.sleep(500);
    check("selecting text offers nothing", await page.evaluate(`!document.querySelector("#doc .em-popover")`));
    await page.evaluate(`getSelection().removeAllRanges(); t.tab("History")`);
    labels = await page.evaluate<string[]>(`t.buttons()`);
    check("History has no button at all", labels.length === 0 && (await page.evaluate(`document.querySelectorAll("#doc .em-history-item").length >= 3`)), labels.join(", "));
    await page.evaluate(`t.tab("Review")`);

    // Even a page that tried is refused by the server. The refusal is a 403, which the browser reports.
    const errorsBefore = page.errors.length;
    check("no console errors before the forced write", errorsBefore === 0, page.errors.join("\n      "));
    await page.evaluate(`endmarkView.actions.accept("s-m4r1")`);
    await page.waitFor(`t.status().includes("read-only")`);
    check("a write forced from the console is refused by the server", (await page.evaluate<string>(`t.status()`)).includes("This view is read-only"), await page.evaluate<string>(`t.status()`));
    check("the file is untouched", (await readFile(file, "utf8")) === original);

    endmark("reply", "--by", "maria", "--note", "Still followed.", file, "c-7f3k");
    await page.waitFor(`t.card("c-7f3k").textContent.includes("Still followed.")`, 5000);
    check("a read-only page follows the file too", true);
    check("every request went to the server itself", foreign(page, server).length === 0, foreign(page, server).join(", "));
  } finally {
    await page.close();
    await server.stop();
  }
}

async function hostile(dir: string): Promise<void> {
  const file = join(dir, "hostile.md");
  const body = [
    "# Hostile",
    "",
    'A paragraph with <script>window.pwned = "script"</script> and <img src=x onerror="window.pwned = \'img\'"> in it.',
    "",
    '<script>window.pwned = "block script"</script>',
    "",
    '<img src="http://example.invalid/track.png" onerror="window.pwned = \'block img\'">',
    "",
    '<iframe src="http://example.invalid/"></iframe> <svg onload="window.pwned = \'svg\'"></svg>',
    "",
    "A [link](javascript:window.pwned='link') and an image ![alt](http://example.invalid/image.png).",
    "",
    "```mermaid",
    "graph LR",
    '  A["<img src=x onerror=\\"window.pwned = \'diagram\'\\">"] --> B[Relay]',
    '  click A "javascript:window.pwned = \'click\'"',
    "```",
    "",
  ].join("\n");
  await writeFile(file, body);
  const note = 'Note <script>window.pwned = "note script"</script> <img src=x onerror="window.pwned = \'note img\'"> [x](javascript:window.pwned=1)';
  endmark("comment", "--by", "mallory", "--on", "A paragraph", "--note", note, file);
  endmark("suggest", "--by", "mallory", "--id", "s-ev11", "--note", note, "--replace", "# Hostile", "--with", '# Hostile <img src=x onerror="window.pwned = \'suggested\'">', file);
  const server = await view(file, "--by", "adam");
  const page = await open(server);
  try {
    console.log("\n# A hostile document");
    await page.waitFor(`document.querySelector("#doc .em-diagram.em-drawn, #doc .em-diagram.em-failed")`);
    for (const tab of ["Review", "Final", "Preview", "History", "Review"]) {
      await page.evaluate(`t.tab("${tab}")`);
      await Bun.sleep(300);
      check(`${tab}: no script ran`, await page.evaluate(`window.pwned === undefined`), await page.evaluate<string>(`String(window.pwned)`));
      check(`${tab}: the HTML is not in the page as elements`, await page.evaluate(`!document.querySelector("#doc script, #doc iframe, #doc .em-md img, #doc [onerror], #doc [onload], #doc a[href^='javascript']")`));
    }
    check("the HTML is shown as text", await page.evaluate(`t.doc().textContent.includes('<script>window.pwned = "script"</script>') && t.doc().textContent.includes("<iframe")`));
    check("the note's HTML is shown as text", await page.evaluate(`document.querySelector("#doc .em-thread .em-note").textContent.includes('<script>window.pwned = "note script"</script>')`));
    // Accepting a suggestion that adds HTML puts it in the file, and the page still shows it as text.
    await page.waitFor(`t.card("s-ev11")`);
    await page.evaluate(`t.click(t.card("s-ev11"), "Accept")`);
    await onDisk(file, /<!--endmark accept s-ev11 /);
    await page.waitFor(`!t.card("s-ev11")`);
    await Bun.sleep(300);
    check("after accepting a suggestion that adds HTML, still no script ran", await page.evaluate(`window.pwned === undefined && !document.querySelector("#doc .em-md img")`));
    check("every request went to the server itself", foreign(page, server).length === 0, foreign(page, server).join(", "));
    check("no console errors and no uncaught exceptions", page.errors.length === 0, page.errors.join("\n      "));
    check("nothing tried to break the content security policy", (await page.evaluate<string[]>(`t.violations`)).length === 0, (await page.evaluate<string[]>(`t.violations`)).join(", "));
  } finally {
    await page.close();
    await server.stop();
  }
}

async function narrow(dir: string): Promise<void> {
  const file = join(dir, "narrow.md");
  await writeFile(file, await readFile(join(root, "conformance/worked-example.md")));
  const server = await view(file, "--by", "agent:claude", "--for", "adam");
  const page = await open(server, { width: 375, height: 800 });
  try {
    console.log("\n# Phone width, and an agent's handle");
    await page.waitFor(`document.querySelectorAll("#doc .em-diagram.em-drawn").length === 2`);
    check("the header names the agent and the person", (await page.evaluate<string>(`document.querySelector("#who").textContent`)) === "Writing as agent:claude for adam");
    check("no horizontal scroll at 375 px", await page.evaluate(`document.documentElement.scrollWidth <= window.innerWidth`), await page.evaluate<string>(`document.documentElement.scrollWidth + " > " + window.innerWidth`));
    check("no console errors at phone width", page.errors.length === 0, page.errors.join("\n      "));
  } finally {
    await page.close();
    await server.stop();
  }
}

async function themes(dir: string): Promise<void> {
  const file = join(dir, "themes.md");
  await writeFile(file, await readFile(join(root, "conformance/worked-example.md")));
  const original = await readFile(file, "utf8");
  const server = await view(file, "--by", "adam");
  const page = await open(server);
  const system = (value: "light" | "dark") =>
    page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value }] });
  const background = () => page.evaluate<string>(`getComputedStyle(document.body).backgroundColor`);
  const card = () => page.evaluate<string>(`getComputedStyle(t.card("s-4q8d")).backgroundColor`);
  const pressed = () => page.evaluate<string>(`document.querySelector('#theme [aria-pressed="true"]')?.dataset.theme ?? ""`);
  // The fill of the first box in the first diagram, and a mark that it was drawn anew.
  const diagram = () =>
    page.evaluate<string>(`(() => {
      const svg = document.querySelector("#doc .em-pictures .em-figure svg");
      const box = svg?.querySelector(".node rect, .node polygon, .node path");
      return svg && box ? svg.id + " " + getComputedStyle(box).fill : "";
    })()`);
  const redrawn = async (before: string) => {
    await page.waitFor(`(() => {
      const svg = document.querySelector("#doc .em-pictures .em-figure svg");
      return document.querySelectorAll("#doc .em-diagram.em-drawn").length === 2 && svg && svg.id !== ${JSON.stringify(before.split(" ")[0])};
    })()`);
    return diagram();
  };
  const LIGHT = "rgb(255, 255, 255)";
  const DARK = "rgb(13, 17, 23)";
  try {
    console.log("\n# Light, dark and auto");
    await page.waitFor(`document.querySelectorAll("#doc .em-diagram.em-drawn").length === 2`);
    check("Auto is chosen at first", (await pressed()) === "auto");
    check("with a light system, the page is light", (await background()) === LIGHT, await background());
    const lightCard = await card();
    const lightDiagram = await diagram();
    check("the theme switch is labelled for assistive technology", await page.evaluate(`document.querySelector("#theme").getAttribute("aria-label") === "Colours" && document.querySelectorAll("#theme button").length === 3`));

    await page.evaluate(`t.click(document.querySelector("#theme"), "Dark")`);
    const darkDiagram = await redrawn(lightDiagram);
    check("Dark makes the page dark", (await background()) === DARK, await background());
    check("Dark reaches the renderer's cards", (await card()) !== lightCard, await card());
    check("Dark redraws the diagrams in dark colours", darkDiagram.split(" ")[1] !== lightDiagram.split(" ")[1], `${lightDiagram} / ${darkDiagram}`);
    check("Dark is shown as chosen", (await pressed()) === "dark");
    check("the choice is kept in this browser", (await page.evaluate<string>(`localStorage.getItem("endmark-view-theme") ?? ""`)) === "dark");

    await page.send("Page.reload");
    await page.waitFor(`document.documentElement.dataset.ready === "true"`);
    await page.evaluate(HELPERS);
    await page.waitFor(`document.querySelectorAll("#doc .em-diagram.em-drawn").length === 2`);
    check("after a reload the page is still dark", (await background()) === DARK && (await pressed()) === "dark", await background());

    await system("dark");
    await page.evaluate(`t.click(document.querySelector("#theme"), "Light")`);
    await redrawn(await diagram());
    check("Light overrides a dark system setting", (await background()) === LIGHT, await background());
    check("Light reaches the renderer's cards", (await card()) === lightCard, await card());

    const before = await diagram();
    await page.evaluate(`t.click(document.querySelector("#theme"), "Auto")`);
    await redrawn(before);
    check("Auto follows a dark system setting", (await background()) === DARK, await background());
    check("Auto forgets the stored choice", await page.evaluate(`localStorage.getItem("endmark-view-theme") === null && !document.documentElement.dataset.theme`));
    const autoDark = await diagram();
    await system("light");
    const autoLight = await redrawn(autoDark);
    check("in Auto, a change of system setting redraws the page and its diagrams", (await background()) === LIGHT && autoLight.split(" ")[1] === lightDiagram.split(" ")[1], `${autoDark} / ${autoLight}`);

    check("changing theme wrote nothing", (await readFile(file, "utf8")) === original);
    check("no console errors while changing theme", page.errors.length === 0, page.errors.join("\n      "));
    check("no request left the server", foreign(page, server).length === 0, foreign(page, server).join(", "));
  } finally {
    await page.close();
    await server.stop();
  }
}

const dir = await mkdtemp(join(tmpdir(), "endmark-view-"));
try {
  await writable(dir);
  await readOnly(dir);
  await hostile(dir);
  await narrow(dir);
  await themes(dir);
} catch (error) {
  failures.push(`the run stopped: ${error instanceof Error ? error.message : String(error)}`);
  console.log(`FAIL  the run stopped: ${error instanceof Error ? (error.stack ?? error.message) : String(error)}`);
} finally {
  await rm(dir, { recursive: true, force: true });
}

console.log(`\n${passed} passed, ${failures.length} failed`);
for (const failure of failures) console.log(`  failed: ${failure}`);
process.exit(failures.length === 0 ? 0 : 1);
