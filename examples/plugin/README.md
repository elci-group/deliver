# Plugin checks (subprocess protocol v1)

`[[plugin]]` checks let a team add first-class, organization-specific check
kinds without forking `deliver`. A plugin is any executable that speaks a
one-line JSON protocol over stdin/stdout.

> Plugins are arbitrary code. Specs containing `[[plugin]]` checks require
> the same explicit `--allow-commands` trust decision (and the same spec
> review of the `extends` chain) as `[[command]]` checks. There is no
> sandboxing: a plugin can do anything the runner can.

## Protocol (v1)

1. `deliver` resolves the check's `cwd` under `--base` (the path guard
   rejects escapes, including through symlinks), spawns `cmd`, and writes
   exactly one JSON request line to the plugin's stdin, then closes stdin:

   ```json
   {"protocol": 1, "name": "docs completeness", "base": "/abs/path/to/base"}
   ```

2. The plugin must write exactly one JSON object to stdout and exit `0`:

   ```json
   {"pass": true, "message": "all docs/*.md files are non-empty"}
   ```

   - `pass` (boolean) is required; `message` (string) is optional and
     becomes the report message. Extra fields are ignored so the protocol
     can evolve.
   - A nonzero exit, unparseable/partial stdout, a missing `pass` boolean,
     or output beyond 1 MiB per stream is a plugin failure, and the
     failure message quotes the plugin's stderr (truncated to 500 chars).
   - `timeout_secs` (default 300) bounds the plugin; on Unix the whole
     process group is killed on timeout, mirroring command checks.

## Files here

- `plugin.sh` — a working example plugin (POSIX sh): reads the request,
  fails when any `docs/*.md` file under the base is empty.
- `spec.toml` — an example spec exercising the plugin, including a `when`
  skip condition.

Try it:

```bash
deliver --spec spec.toml --base . --allow-commands
```

Plugin checks support `weight`, `when`, `cwd`, and `timeout_secs`, run in
the parallel check pool (report order stays spec order), participate in
`extends` accumulation, and count toward weighted completion like any other
check. The `deliver schema` subcommand includes a `plugin_check` JSON Schema
definition for editor support.
