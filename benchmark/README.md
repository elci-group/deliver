# `deliver` A/B benchmark

This is a small paired benchmark for the question: with the same model, same
agent invocation, and same implementation task, does giving an agent the
`deliver` gate improve acceptance, token efficiency, or elapsed time?

It runs each condition once per pair against an isolated copy of the same tiny
Python task. The only experimental difference is the final instruction:

| Control | Treatment |
| --- | --- |
| Do not use `deliver`. | Run `deliver --spec deliver.toml --strict` before finishing. |

The runner randomizes the order within each pair to reduce warm-up and cache
effects. It externally grades *both* conditions with the same `deliver` spec,
after the agent has stopped. This is important: a treatment run's self-check is
not treated as evidence of quality, and the control arm is not disadvantaged by
being unable to self-check during its run.

## Run it

Use an agent command template. It is tokenized safely (not passed through a
shell); `{prompt}` must be a standalone template token. Substitute flags to
match your installed agent. For example, if the agent accepts model, cwd, and a
final positional prompt:

```bash
python3 benchmark/run_benchmark.py \
  --model YOUR_FIXED_MODEL \
  --repetitions 5 \
  --agent-cmd 'your-agent --model {model} --cwd {workdir} {prompt}'
```

For a meaningful comparison, hold fixed the agent binary/version, model,
reasoning level, tool permissions, environment, task prompt, and repetition
count. Run from a quiet machine if elapsed time matters. Do not include any
`deliver` instruction in the agent's global configuration; the runner supplies
it only in the treatment prompt.

The command creates `benchmark/results/<UTC timestamp>/` containing:

- `trials.csv`: raw paired measurements and pass/fail result.
- `summary.json`: group metrics plus `deliver − control` paired deltas.
- one directory per trial with the exact prompt, transcript, final answer, and
  external assessment output.

The runner reads standard JSON/JSONL usage objects such as `total_tokens` or
`input_tokens`/`output_tokens`. It reports model-inference input, cached-input,
output, and reasoning-output tokens separately. `deliver` itself is a
deterministic local checker: its inference-token and inference-cost columns
are always zero; its tool-call count is reported separately. Any extra context
needed to use the deliver skill is still model input and is included in model
spend. If the chosen agent does not emit usage, coverage is `0/N` rather than
an invented estimate.

For Codex CLI, use a supported model for the account, include
`--skip-git-repo-check` because the trials are isolated copies, and perform a
one-prompt smoke test first. For example:

```bash
codex exec --ignore-user-config --skip-git-repo-check --ephemeral \
  -C /tmp 'Reply with only: ready'
```

After the smoke test succeeds, the corresponding runner invocation is:

```bash
python3 benchmark/run_benchmark.py --model YOUR_SUPPORTED_MODEL --repetitions 5 \
  --agent-cmd 'codex exec --ignore-user-config --skip-git-repo-check --json --approve-for-me -m {model} -C {workdir} -o {answer_file} {prompt}'
```

If the CLI reports a model, authentication, or quota error, resolve it before
benchmarking. The runner records `valid_comparison: false` when any agent run
does not complete successfully. Such a result is diagnostic data, not evidence
that either condition produces lower-quality work.

## Interpretation

Primary outcome: external acceptance rate. Secondary outcomes: reported total
tokens and wall-clock elapsed seconds. Inspect the individual pairs before
trusting averages; five or more pairs is a useful modest start, but this is an
engineering signal, not a statistical claim. Treatment can reasonably cost
more tokens and time while increasing accepted work; report that trade-off
rather than declaring a blanket winner.

### Cost accounting

Pass the provider's current USD rates per million tokens to calculate spend:

```bash
python3 benchmark/run_benchmark.py ... \
  --input-usd-per-million 2.50 \
  --cached-input-usd-per-million 0.25 \
  --output-usd-per-million 10
```

Without all three rates, cost is `null`, never guessed. The formula uses
uncached input (`input_tokens - cached_input_tokens`), cached input, and
`output_tokens`; reasoning output is included in output billing and exposed
separately for diagnostics. The summary includes paired model-cost deltas and
explicit zeroes for deliver inference.

Verified rate presets (USD per 1M tokens, checked 2026-09-09):

| Provider/model | Input | Cached input | Output |
| --- | ---: | ---: | ---: |
| OpenAI GPT-5.6 Terra (ChatGPT Work/Codex) | 2.00 | 0.20 | 12.00 |
| OpenAI GPT-5 (API) | 1.25 | 0.125 | 10.00 |
| Groq `openai/gpt-oss-120b` | 0.15 | 0.075 | 0.60 |
| Groq `openai/gpt-oss-20b` | 0.075 | 0.0375 | 0.30 |

Use the row matching the actual model and endpoint. Groq Compound and
Compound Mini are priced as systems without a published per-token rate, so
their cost must remain `null` unless you supply an agreed account rate.
`deliver` itself is deterministic and has no inference charge.

The fixture intentionally includes validation the treatment agent can discover
via `deliver.toml`, while the independent scorer protects tests and task
instructions from modification. Replace `fixture/` with a representative task
only if you keep the protected files and external scoring principle intact.
