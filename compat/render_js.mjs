// Renders a Markdown file with one JavaScript engine and prints the HTML.
import { readFileSync } from "node:fs";

const [engine, path] = process.argv.slice(2);
const text = readFileSync(path, "utf8");

async function render() {
  if (engine === "markdown-it") {
    const { default: MarkdownIt } = await import("markdown-it");
    return new MarkdownIt("commonmark").render(text);
  }
  if (engine === "markdown-it-html-off") {
    const { default: MarkdownIt } = await import("markdown-it");
    return new MarkdownIt().render(text); // the library's default: html off
  }
  if (engine === "marked") {
    const { marked } = await import("marked");
    return marked.parse(text);
  }
  if (engine === "commonmark.js") {
    const { Parser, HtmlRenderer } = await import("commonmark");
    return new HtmlRenderer().render(new Parser().parse(text));
  }
  throw new Error(`unknown engine ${engine}`);
}

process.stdout.write(await render());
