# Compatibility matrix

Endmark promises that a Markdown renderer which knows nothing about it shows the document and nothing else. This directory checks that promise against real engines (RFC 0001, section 12.7).

`run.py` renders every conformance fixture that a reader accepts without errors, once whole and once as the body alone, through each engine. Records are **hidden** when the two results are the same HTML once comments are removed.

```sh
pip install -r compat/requirements.txt
(cd compat && npm install)
# and install cmark-gfm, for example: apt-get install cmark-gfm
cargo build
python3 compat/run.py --write     # rewrites RESULTS.md
```

CI runs it with `--require-all` and fails if an engine that should hide records shows one. The results are in [RESULTS.md](RESULTS.md).

Hosted renderers, such as GitHub and GitLab web views, cannot be run here. They are checked by hand; see section 12.7 of the RFC.
