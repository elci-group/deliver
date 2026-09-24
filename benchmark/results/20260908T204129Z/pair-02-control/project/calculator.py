"""Small parsing utility used by the benchmark fixture."""

import re


def parse_percent(value: str) -> float:
    """Return a percentage in [0, 100] as a fraction, or raise ValueError."""
    if not isinstance(value, str):
        raise ValueError("percentage must be a string")

    number = value.strip()
    if number.endswith("%"):
        number = number[:-1].strip()
    if not re.fullmatch(r"\+?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?", number):
        raise ValueError("invalid percentage")

    percent = float(number)
    if not 0 <= percent <= 100:
        raise ValueError("percentage must be between 0 and 100")
    return percent / 100
