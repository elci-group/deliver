# Cron next-occurrence task

Implement `next_occurrence(expr: str, start: datetime) -> datetime` in `cron.py`,
using only the Python standard library. The module may contain helpers, but the
public function must have exactly that name and signature.

## Grammar

`expr` is a five-field cron expression separated by whitespace:

```
minute hour day-of-month month day-of-week
 0-59   0-23     1-31       1-12      0-6
```

Day-of-week `0` is Sunday. Each field is one of:

- `*` (every value)
- a single number, e.g. `30`
- a list of numbers or ranges, e.g. `6,18` or `8-11,17`
- a range `a-b` (inclusive, `a` must be `<=` `b`)
- a step: `*/n` or `a-b/n` (every n-th value within the range)

Anything else — wrong field count, out-of-range numbers, reversed ranges,
`*/0`, non-numeric tokens — raises `ValueError`.

## Matching semantics

- Return the first matching time **strictly after** `start`; `start` itself is
  never returned even if it matches.
- Times are naive `datetime` objects at minute resolution; ignore timezone
  and DST entirely.
- Day-of-month and day-of-week use the standard cron rule: if **both** fields
  are restricted (neither is `*`), a date matches when **either** field
  matches; if only one is restricted, that field alone must match.
- There is no clamping to short months: `0 0 31 2 *` never matches, because
  February never has a 31st day.
- If no occurrence exists within five years after `start`, raise
  `ValueError`. The exact wording is not checked; only the exception type is.

The independent checker after your run uses the same tests that ship in this
directory. The exact behaviour is verified after every agent run.
