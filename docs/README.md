# deliver documentation site

The rendered documentation for the `deliver` CLI and library, served
locally. `index.html` is the single-page site; `styles.css` its only asset;
`server/` is a standard-library-only loopback server (same pattern as
`website/server`).

## Serve

```bash
cd docs/server
cargo run --release -- 127.0.0.1:8743
# open http://127.0.0.1:8743/
```

## Sources

`README.md` and `man/deliver.1` are canonical; this site is their rendered
form. When behavior changes, update those first, then this page. The
project's own `deliver.toml` gate greps this page for shipped-feature
markers, so drift between the docs and the code alarms the same way it does
for the README.
