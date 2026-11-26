What this changes, and why.

- [ ] A change to the format changes the RFC, the implementation and the fixtures together.
- [ ] A change in behaviour has a fixture in `conformance/`, written from the RFC.
- [ ] After changing the renderer, `view/`, `endmark-core` or `endmark-wasm`: rebuilt with `bun run build` in `packages/render/`.
- [ ] After changing dependencies: ran `python3 scripts/notices.py`.
- [ ] `CHANGELOG.md` says what a user would notice.
