"""Small parsing utility used by the benchmark fixture."""

import re


def parse_percent(value: str) -> float:
    """Parse a percentage from 0 through 100, raising ValueError if invalid."""
    if not isinstance(value, str):
        raise ValueError("percentage must be a string")

    number = value.strip()
    if number.endswith("%"):
        number = number[:-1].strip()
    if re.fullmatch(r"[0-9]+(?:\.[0-9]*)?|\.[0-9]+", number) is None:
        raise ValueError("invalid percentage")

    percent = float(number)
    if not 0 <= percent <= 100:
        raise ValueError("percentage must be between 0 and 100")
    return percent / 100
