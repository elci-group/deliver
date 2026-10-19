# Release candidate procedure

The [manual candidate workflow](.github/workflows/release-candidate.yml) tests
and builds the current commit on Linux, macOS (Apple Silicon), and Windows.
It uploads a binary and a SHA-256 checksum file for each platform and requests
GitHub artifact attestations for both files. It does not create a GitHub
Release, tag, or customer distribution. The runner architecture and toolchain
version must be reviewed in each run before a platform is declared supported.

## Produce a candidate

1. Review the commit, dependency lockfile, workflow changes, and CI results.
2. From the repository Actions tab, run **Release candidate** against the
   reviewed commit. Record the commit SHA and all three run IDs.
3. Require all three candidate jobs to pass. Download the three artifacts from
   that run. Keep the binary and its `.sha256` file together.
4. Record the checksum, source commit, workflow run URL, attestation
   verification result, platform, and validation date in the pilot evidence
   register. Keep pilot source and raw logs in the customer's environment.

## Verify before a pilot install

From each downloaded artifact directory, compare the file's computed SHA-256
with the value in its `.sha256` file. For example, on Linux:

```sh
sha256sum -c deliver-x86_64-unknown-linux-gnu.sha256
```

On macOS, use `shasum -a 256` on the binary and compare its output with the
checksum file. On Windows, use `Get-FileHash -Algorithm SHA256` and compare the
hash with the checksum file. A checksum detects accidental changes, but by
itself does not prove where a file came from.

Verify each binary's GitHub attestation against the repository and expected
source commit:

```sh
gh attestation verify ./deliver-x86_64-unknown-linux-gnu --repo elci-group/deliver
```

Inspect the verification output for the expected repository, commit SHA, and
workflow identity. Repeat for the checksum file if it is retained as evidence.
Do not install a file when its checksum or attestation fails verification.
The GitHub CLI and network access are required for this verification path;
for an offline pilot, verify and record the result before transferring the
binary to the isolated environment.

Run `deliver --version` after install, then validate the pilot spec with
`deliver validate <spec>` and run the agreed smoke check. Specs containing
commands require a trusted reviewer to approve the spec and its `extends`
chain, followed by `--allow-commands` at execution time. The flag does not
sandbox the commands.

## Rollback

Keep the last verified binary and its evidence record. If the candidate
fails a pilot gate, restore that binary, rerun the smoke check and pilot
spec, and record the failed candidate SHA, reason, and rollback result. Do
not relabel the restored binary as the new candidate.

This procedure is preparation until a real workflow run and independent
verification have succeeded. No signed or attested customer binary exists
solely because the workflow file is present.
