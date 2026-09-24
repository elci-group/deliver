"""Small parsing utility used by the benchmark fixture."""


def parse_percent(value: str) -> float:
    """Return a percentage in [0, 100] as a fraction, or raise ValueError."""
    if not isinstance(value, str):
        raise ValueError("percentage must be a string")

    number = value.strip()
    if number.endswith("%"):
        number = number[:-1].strip()
    if not number or "_" in number:
        raise ValueError("invalid percentage")

    percent = float(number)
    if not 0 <= percent <= 100:
        raise ValueError("percentage must be between 0 and 100")
    return percent / 100
