# Percentage parser task

Implement `parse_percent(value: str) -> float` in `calculator.py`.

It must accept a string containing a non-negative number from 0 through 100,
with optional surrounding whitespace and one optional trailing `%`. Return the
number as a fraction (so `"12.5%"` becomes `0.125`). Reject malformed input,
negative values, values above 100, and non-string input by raising `ValueError`.

The exact behaviour is checked independently after every agent run.
