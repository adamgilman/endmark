"""Renders a Markdown file with one Python engine and prints the HTML."""
import sys

def render(engine: str, text: str) -> str:
    if engine == "commonmark":
        import commonmark
        return commonmark.commonmark(text)
    if engine == "markdown-it-py":
        from markdown_it import MarkdownIt
        return MarkdownIt("commonmark").render(text)
    if engine == "markdown-it-py-html-off":
        from markdown_it import MarkdownIt
        return MarkdownIt("commonmark", {"html": False}).render(text)
    if engine == "mistune":
        import mistune
        return mistune.create_markdown(escape=False)(text)
    if engine == "python-markdown":
        import markdown
        return markdown.markdown(text, extensions=["fenced_code"])
    raise SystemExit(f"unknown engine {engine}")

if __name__ == "__main__":
    sys.stdout.write(render(sys.argv[1], open(sys.argv[2], encoding="utf-8").read()))
