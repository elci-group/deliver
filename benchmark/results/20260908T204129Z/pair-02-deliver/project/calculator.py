"""Small parsing utility used by the benchmark fixture."""

import re


def parse_percent(value: str) -> float:
    """Return a percentage in [0, 100] as a fraction; reject invalid input."""
    if not isinstance(value, str):
        raise ValueError("percentage must be a string")

    text = value.strip()
    if text.endswith("%"):
        text = text[:-1].strip()
    if re.fullmatch(r"[0-9]+(?:\.[0-9]*)?|\.[0-9]+", text) is None:
        raise ValueError("invalid percentage")

    number = float(text)
    if not 0 <= number <= 100:
        raise ValueError("percentage must be between 0 and 100")
    return number / 100
