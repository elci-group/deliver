"""Small parsing utility used by the benchmark fixture."""

import re
from decimal import Decimal


def parse_percent(value: str) -> float:
    """Parse a decimal percentage in [0, 100], or raise ValueError."""
    if not isinstance(value, str):
        raise ValueError("percentage must be a string")

    text = value.strip()
    if text.endswith("%"):
        text = text[:-1].rstrip()
    if re.fullmatch(r"[0-9]+(?:\.[0-9]*)?|\.[0-9]+", text) is None:
        raise ValueError("invalid percentage")

    number = Decimal(text)
    if number > 100:
        raise ValueError("percentage must be between 0 and 100")
    return float(number / 100)
