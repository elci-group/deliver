#!/usr/bin/env python3
"""Deterministic stand-in used only to test the benchmark runner."""
import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--cwd", required=True)
parser.add_argument("--exit", type=int, default=0)
parser.add_argument("prompt")
args = parser.parse_args()

if args.exit:
    print(json.dumps({"error": "intentional fake-agent failure"}))
    raise SystemExit(args.exit)

Path(args.cwd, "calculator.py").write_text(
    "import csv\nfrom decimal import Decimal, ROUND_HALF_UP\n"
    "def build_sales_summary(csv_text: str) -> dict:\n"
    "    if not isinstance(csv_text, str): raise ValueError('input must be text')\n"
    "    rows = list(csv.reader(csv_text.splitlines()))\n"
    "    if not rows or rows[0] != ['product', 'quantity', 'unit_price']: raise ValueError('bad header')\n"
    "    totals = {}\n"
    "    for line, row in enumerate(rows[1:], 2):\n"
    "        if not row: continue\n"
    "        if len(row) != 3: raise ValueError(f'line {line}: bad row')\n"
    "        name = row[0].strip()\n"
    "        try: quantity = int(row[1]); price = Decimal(row[2].strip())\n"
    "        except Exception: raise ValueError(f'line {line}: invalid value') from None\n"
    "        if not name or quantity <= 0 or price < 0: raise ValueError(f'line {line}: invalid value')\n"
    "        total = totals.setdefault(name, [0, Decimal(0)]); total[0] += quantity; total[1] += quantity * price\n"
    "    def money(value): return float(value.quantize(Decimal('0.01'), rounding=ROUND_HALF_UP))\n"
    "    products = sorted(({'product': n, 'quantity': q, 'revenue': money(r)} for n, (q, r) in totals.items()), key=lambda p: (-p['revenue'], p['product']))\n"
    "    return {'products': products, 'grand_total': money(sum((Decimal(str(p['revenue'])) for p in products), Decimal(0)))}\n"
)
print(json.dumps({"usage": {"input_tokens": 40, "output_tokens": 7}}))
