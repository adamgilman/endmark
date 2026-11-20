// A minimal Chrome DevTools Protocol client for the smoke test.
// It starts headless Chromium, opens one page and evaluates scripts in it.

import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

export interface Page {
  /** Console errors and uncaught exceptions seen so far. */
  errors: string[];
  /** URLs of every request the page made. */
  requests: string[];
  evaluate<T>(expression: string): Promise<T>;
  send(method: string, params?: Record<string, unknown>): Promise<any>;
  waitFor(expression: string, timeoutMs?: number): Promise<void>;
  screenshot(path: string): Promise<void>;
  close(): Promise<void>;
}

export async function openPage(url: string, options: { width?: number; height?: number; dark?: boolean } = {}): Promise<Page> {
  const binary = process.env.CHROMIUM ?? "chromium";
  const profile = await mkdtemp(join(tmpdir(), "endmark-smoke-"));
  const child = Bun.spawn(
    [
      binary,
      "--headless=new",
      "--no-sandbox",
      "--disable-gpu",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      `--window-size=${options.width ?? 1280},${options.height ?? 900}`,
      "about:blank",
    ],
    { stderr: "pipe", stdout: "ignore" },
  );

  // Chromium prints the address of its debugging socket on stderr.
  const browserUrl = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Chromium did not report a DevTools address in 20 s")), 20_000);
    (async () => {
      const decoder = new TextDecoder();
      let seen = "";
      for await (const chunk of child.stderr) {
        seen += decoder.decode(chunk);
        const match = /DevTools listening on (ws:\/\/\S+)/.exec(seen);
        if (match) {
          clearTimeout(timer);
          resolve(match[1]!);
          return;
        }
      }
      reject(new Error(`Chromium exited before reporting a DevTools address:\n${seen}`));
    })();
  });
  const port = new URL(browserUrl).port;
  const targets = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()) as { type: string; webSocketDebuggerUrl: string }[];
  const target = targets.find((t) => t.type === "page");
  if (!target) throw new Error("Chromium has no page target");

  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise<void>((resolve, reject) => {
    socket.onopen = () => resolve();
    socket.onerror = () => reject(new Error("could not connect to the page"));
  });
  let next = 0;
  const waiting = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void }>();
  const errors: string[] = [];
  const requests: string[] = [];
  socket.onmessage = (event) => {
    const message = JSON.parse(String(event.data));
    if (message.id !== undefined) {
      const entry = waiting.get(message.id);
      waiting.delete(message.id);
      if (message.error) entry?.reject(new Error(message.error.message));
      else entry?.resolve(message.result);
      return;
    }
    const p = message.params;
    if (message.method === "Runtime.exceptionThrown") {
      errors.push(`exception: ${p.exceptionDetails.exception?.description ?? p.exceptionDetails.text}`);
    } else if (message.method === "Runtime.consoleAPICalled" && p.type === "error") {
      errors.push(`console.error: ${p.args.map((a: any) => a.value ?? a.description).join(" ")}`);
    } else if (message.method === "Log.entryAdded" && p.entry.level === "error") {
      errors.push(`log: ${p.entry.text} ${p.entry.url ?? ""}`);
    } else if (message.method === "Network.requestWillBeSent") {
      requests.push(p.request.url);
    }
  };
  const send = (method: string, params: Record<string, unknown> = {}) =>
    new Promise<any>((resolve, reject) => {
      const id = ++next;
      waiting.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async <T>(expression: string): Promise<T> => {
    const result = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
    }
    return result.result.value as T;
  };

  await send("Runtime.enable");
  await send("Log.enable");
  await send("Network.enable");
  await send("Page.enable");
  await send("Emulation.setDeviceMetricsOverride", {
    width: options.width ?? 1280,
    height: options.height ?? 900,
    deviceScaleFactor: 1,
    mobile: false,
  });
  if (options.dark) {
    await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
  }
  await send("Page.navigate", { url });

  return {
    errors,
    requests,
    evaluate,
    send,
    async waitFor(expression, timeoutMs = 20_000) {
      const end = Date.now() + timeoutMs;
      for (;;) {
        if (await evaluate<boolean>(`!!(${expression})`).catch(() => false)) return;
        if (Date.now() > end) throw new Error(`timed out waiting for: ${expression}`);
        await Bun.sleep(100);
      }
    },
    async screenshot(path) {
      const { data } = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true });
      await Bun.write(path, Buffer.from(data, "base64"));
    },
    async close() {
      socket.close();
      child.kill();
      await child.exited;
      await rm(profile, { recursive: true, force: true });
    },
  };
}
