#!/usr/bin/env python3
"""Run a paired, isolated A/B benchmark for the deliver acceptance gate.

The runner intentionally does not know how to invoke a particular agent.  Give
it a command template whose tokens use {model}, {workdir}, {prompt},
{prompt_file}, {answer_file}, and {condition}.  A token that is exactly
{prompt} is replaced as one argv item, so prompts with spaces are safe.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import random
import shutil
import statistics
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def protected_digests(fixture: Path) -> dict[str, str]:
    """Hash the fixture files agents must never modify.

    Convention: the task README, the deliver spec, and every test_*.py are
    protected; every other file is the agent's to edit or create.
    """
    names = sorted(entry.name for entry in fixture.iterdir()
                   if entry.name in ("README.md", "deliver.toml")
                   or (entry.name.startswith("test_") and entry.suffix == ".py"))
    return {name: digest(fixture / name) for name in names}


def substitute(tokens: list[str], values: dict[str, str]) -> list[str]:
    result = []
    for token in tokens:
        for name, value in values.items():
            token = token.replace("{" + name + "}", value)
        result.append(token)
    return result


def numeric(value: Any) -> int | None:
    return value if isinstance(value, int) and value >= 0 else None


def usage_from_json(value: Any) -> list[int]:
    """Collect one usage total per object; supports common CLI JSON shapes."""
    totals: list[int] = []
    if isinstance(value, dict):
        total = numeric(value.get("total_tokens"))
        if total is not None:
            totals.append(total)
        elif any(key in value for key in ("input_tokens", "output_tokens", "prompt_tokens", "completion_tokens")):
            parts = [numeric(value.get(key)) for key in ("input_tokens", "output_tokens", "prompt_tokens", "completion_tokens")]
            totals.append(sum(part for part in parts if part is not None))
        for child in value.values():
            totals.extend(usage_from_json(child))
    elif isinstance(value, list):
        for child in value:
            totals.extend(usage_from_json(child))
    return totals


def usage_breakdowns(value: Any) -> list[dict[str, int]]:
    records: list[dict[str, int]] = []
    if isinstance(value, dict):
        keys = ("input_tokens", "cached_input_tokens", "output_tokens", "reasoning_output_tokens")
        if any(key in value for key in keys):
            records.append({key: numeric(value.get(key)) or 0 for key in keys})
        for child in value.values():
            records.extend(usage_breakdowns(child))
    elif isinstance(value, list):
        for child in value:
            records.extend(usage_breakdowns(child))
    return records


def token_usage(output: str) -> tuple[int | None, str, dict[str, int]]:
    values: list[int] = []
    records: list[dict[str, int]] = []
    for line in output.splitlines():
        try:
            event = json.loads(line)
            values.extend(usage_from_json(event))
            records.extend(usage_breakdowns(event))
        except json.JSONDecodeError:
            pass
    detail = {key: max((record[key] for record in records), default=0)
              for key in ("input_tokens", "cached_input_tokens", "output_tokens", "reasoning_output_tokens")}
    if values:
        # Streaming clients sometimes repeat cumulative usage. The largest
        # value is the conservative final total for one invocation.
        return max(values), "agent JSON usage", detail
    return None, "not reported", detail


def cost_usd(detail: dict[str, int], rates: dict[str, float | None]) -> float | None:
    if any(rates[name] is None for name in ("input", "cached_input", "output")):
        return None
    uncached = max(detail["input_tokens"] - detail["cached_input_tokens"], 0)
    total = (uncached * rates["input"] + detail["cached_input_tokens"] * rates["cached_input"] +
             detail["output_tokens"] * rates["output"]) / 1_000_000
    return round(total, 8)


def assess(project: Path, protected: dict[str, str]) -> tuple[bool, str]:
    changed = [name for name, expected in protected.items() if digest(project / name) != expected]
    if changed:
        return False, "protected fixture files changed: " + ", ".join(changed)
    command = ["deliver", "--spec", "deliver.toml", "--strict"]
    completed = subprocess.run(command, cwd=project, capture_output=True, text=True, timeout=90)
    message = (completed.stdout + completed.stderr).strip()
    return completed.returncode == 0, message[-3000:]


def mean(values: list[float]) -> float | None:
    return round(statistics.mean(values), 3) if values else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent-cmd", required=True, help="argv template; quote it as one shell argument")
    parser.add_argument("--model", required=True)
    parser.add_argument("--repetitions", type=int, default=3, help="paired trials per condition (default: 3)")
    parser.add_argument("--seed", type=int, default=20260908)
    parser.add_argument("--fixture-dir", type=Path, default=None,
                        help="task fixture directory (default: benchmark/fixture)")
    parser.add_argument("--runs-dir", type=Path, default=None)
    parser.add_argument("--timeout-secs", type=int, default=900)
    parser.add_argument("--input-usd-per-million", type=float, default=None,
                        help="provider price for uncached input tokens")
    parser.add_argument("--cached-input-usd-per-million", type=float, default=None,
                        help="provider price for cached input tokens")
    parser.add_argument("--output-usd-per-million", type=float, default=None,
                        help="provider price for output tokens (including reasoning output)")
    args = parser.parse_args()
    if args.repetitions < 2:
        parser.error("--repetitions must be at least 2 for a paired comparison")
    if shutil.which("deliver") is None:
        parser.error("deliver must be on PATH: it is the neutral external scorer")

    import shlex
    template = shlex.split(args.agent_cmd)
    if "{prompt}" not in template:
        parser.error("--agent-cmd must include {prompt} as a standalone token")
    run_root = args.runs_dir or ROOT / "results" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_root.mkdir(parents=True, exist_ok=False)
    fixture = (args.fixture_dir or (ROOT / "fixture")).resolve()
    if not fixture.is_dir():
        parser.error(f"--fixture-dir is not a directory: {fixture}")
    prompt_template = (ROOT / "task_prompt.md").read_text()
    protected = protected_digests(fixture)
    rng = random.Random(args.seed)
    rates = {"input": args.input_usd_per_million, "cached_input": args.cached_input_usd_per_million,
             "output": args.output_usd_per_million}
    rows: list[dict[str, Any]] = []

    for pair in range(1, args.repetitions + 1):
        conditions = ["control", "deliver"]
        rng.shuffle(conditions)  # balances warm-up, cache, and fatigue effects.
        for condition in conditions:
            trial = run_root / f"pair-{pair:02d}-{condition}"
            project = trial / "project"
            shutil.copytree(fixture, project, ignore=shutil.ignore_patterns("__pycache__"))
            treatment = (
                "Before your final response, run `deliver --spec deliver.toml --strict` and fix any failure."
                if condition == "deliver"
                else "Do not invoke or use deliver; complete the task using your normal workflow."
            )
            prompt = prompt_template.format(workdir=project, condition_instruction=treatment)
            prompt_file = trial / "prompt.md"
            prompt_file.write_text(prompt)
            answer_file = trial / "final-answer.txt"
            command = substitute(template, {
                "model": args.model, "workdir": str(project), "prompt": prompt,
                "prompt_file": str(prompt_file), "answer_file": str(answer_file), "condition": condition,
            })
            started = time.monotonic()
            try:
                completed = subprocess.run(command, cwd=project, capture_output=True, text=True, timeout=args.timeout_secs)
                exit_code, timed_out = completed.returncode, False
                transcript = completed.stdout + "\n--- stderr ---\n" + completed.stderr
            except subprocess.TimeoutExpired as exc:
                exit_code, timed_out = None, True
                transcript = (exc.stdout or "") + "\n--- timeout ---\n" + (exc.stderr or "")
            elapsed = round(time.monotonic() - started, 3)
            (trial / "agent-transcript.txt").write_text(transcript)
            passed, assessment = assess(project, protected)
            (trial / "assessment.txt").write_text(assessment)
            tokens, token_source, detail = token_usage(transcript)
            agent_completed = exit_code == 0 and not timed_out
            deliver_calls = sum(1 for line in transcript.splitlines()
                                if '"type":"command_execution"' in line and "deliver --spec" in line)
            rows.append({"pair": pair, "condition": condition, "agent_exit": exit_code,
                         "timed_out": timed_out, "agent_completed": agent_completed, "elapsed_secs": elapsed, "tokens": tokens,
                         "token_source": token_source, "model_input_tokens": detail["input_tokens"],
                         "model_cached_input_tokens": detail["cached_input_tokens"],
                         "model_output_tokens": detail["output_tokens"],
                         "model_reasoning_output_tokens": detail["reasoning_output_tokens"],
                         "model_cost_usd": cost_usd(detail, rates),
                         "deliver_tool_calls": deliver_calls, "deliver_inference_tokens": 0,
                         "deliver_inference_cost_usd": 0.0, "acceptance_pass": passed,
                         "trial_dir": str(trial.relative_to(run_root))})

    with (run_root / "trials.csv").open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(rows[0]))
        writer.writeheader(); writer.writerows(rows)
    summary: dict[str, Any] = {"model": args.model, "seed": args.seed, "repetitions": args.repetitions,
                               "fixture": fixture.name,
                               "scorer": "deliver --spec deliver.toml --strict, outside the agent invocation", "conditions": {}}
    for condition in ("control", "deliver"):
        group = [row for row in rows if row["condition"] == condition]
        tokens = [row["tokens"] for row in group if row["tokens"] is not None]
        elapsed = [row["elapsed_secs"] for row in group]
        costs = [row["model_cost_usd"] for row in group if row["model_cost_usd"] is not None]
        completed = [row for row in group if row["agent_completed"]]
        summary["conditions"][condition] = {"agent_completion_rate": sum(row["agent_completed"] for row in group) / len(group),
            "acceptance_rate": sum(row["acceptance_pass"] for row in group) / len(group),
            "acceptance_rate_when_completed": (sum(row["acceptance_pass"] for row in completed) / len(completed)) if completed else None,
            "mean_elapsed_secs": mean(elapsed), "median_elapsed_secs": round(statistics.median(elapsed), 3),
            "mean_tokens_reported": mean(tokens), "token_coverage": f"{len(tokens)}/{len(group)}",
            "mean_model_cost_usd": mean(costs) if costs else None,
            "model_cost_coverage": f"{len(costs)}/{len(group)}",
            "total_deliver_tool_calls": sum(row["deliver_tool_calls"] for row in group),
            "deliver_inference_tokens": 0, "deliver_inference_cost_usd": 0.0}
    control = {row["pair"]: row for row in rows if row["condition"] == "control"}
    treatment = {row["pair"]: row for row in rows if row["condition"] == "deliver"}
    summary["paired_deltas_deliver_minus_control"] = {
        "acceptance": mean([float(treatment[p]["acceptance_pass"]) - float(control[p]["acceptance_pass"]) for p in control]),
        "elapsed_secs": mean([treatment[p]["elapsed_secs"] - control[p]["elapsed_secs"] for p in control]),
        "tokens": mean([treatment[p]["tokens"] - control[p]["tokens"] for p in control if treatment[p]["tokens"] is not None and control[p]["tokens"] is not None]),
        "model_cost_usd": mean([treatment[p]["model_cost_usd"] - control[p]["model_cost_usd"] for p in control if treatment[p]["model_cost_usd"] is not None and control[p]["model_cost_usd"] is not None]),
    }
    summary["pricing"] = {"input_usd_per_million": args.input_usd_per_million,
        "cached_input_usd_per_million": args.cached_input_usd_per_million,
        "output_usd_per_million": args.output_usd_per_million,
        "cost_formula": "(uncached input × input rate + cached input × cached rate + output × output rate) / 1,000,000",
        "reasoning_output": "included in output_tokens; reported separately for diagnostics"}
    failed_conditions = [condition for condition in ("control", "deliver")
                         if summary["conditions"][condition]["agent_completion_rate"] < 1]
    summary["valid_comparison"] = not failed_conditions
    if failed_conditions:
        summary["invalid_reason"] = ("Agent invocations did not all complete in: " + ", ".join(failed_conditions) +
            ". Do not interpret acceptance or timing as an agent-quality comparison; inspect agent-transcript.txt first.")
    (run_root / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    print(f"\nArtifacts: {run_root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
