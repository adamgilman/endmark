# Compatibility results

Written by `python3 compat/run.py --write`. Do not edit by hand.

- **hidden**: the engine renders the file exactly as it renders the body alone.
- **visible**: the engine prints records as text. Expected where raw HTML is turned off (RFC 0001, D-4).
- **removed**: the engine drops raw HTML, records included, and the rest renders as the body alone.

| Engine | Expected | Result | Detail |
| --- | --- | --- | --- |
| commonmark-py | hidden | hidden | 34 of 34 files identical to the body alone |
| markdown-it-py, HTML on | hidden | hidden | 34 of 34 files identical to the body alone |
| markdown-it-py, HTML off | visible | visible | 0 of 34 files identical to the body alone |
| mistune | hidden | hidden | 34 of 34 files identical to the body alone |
| Python-Markdown | hidden | hidden | 33 of 33 files identical to the body alone. Known exception: `escapes-in-quotes` |
| commonmark.js | hidden | hidden | 34 of 34 files identical to the body alone |
| markdown-it (JavaScript), HTML on | hidden | hidden | 34 of 34 files identical to the body alone |
| markdown-it (JavaScript), HTML off, its default | visible | visible | 0 of 34 files identical to the body alone |
| marked | hidden | hidden | 34 of 34 files identical to the body alone |
| cmark-gfm, unsafe mode | hidden | not run: cmark-gfm is not installed |  |
| cmark-gfm, safe mode, its default | removed | not run: cmark-gfm is not installed |  |

**Known exceptions**

- Python-Markdown, `escapes-in-quotes`: the body contains an unclosed `<!--`. Python-Markdown is not a CommonMark engine: it reads that as the start of a comment, which swallows the start of the first record and leaves its end visible.
