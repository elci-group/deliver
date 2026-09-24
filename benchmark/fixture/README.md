# Sales report task

Implement `build_sales_summary(csv_text: str) -> dict` in `calculator.py`.

The input is CSV text with exactly this header: `product,quantity,unit_price`.
Use the standard-library `csv` module so quoted product names and commas are
handled correctly. Ignore completely blank lines, but reject non-string input,
a missing/wrong header, malformed rows, empty product names, quantities that
are not positive integers, and prices that are not non-negative decimal
numbers. Raise `ValueError` and include the 1-based CSV line number for row
errors. Do not use binary floating point for money.

Return this JSON-friendly shape:

```python
{
    "products": [
        {"product": "Widget", "quantity": 3, "revenue": 29.97},
    ],
    "grand_total": 29.97,
}
```

Aggregate duplicate products after trimming surrounding product whitespace.
Order products by descending revenue, then product name for ties. Round each
revenue and `grand_total` to exactly two decimal places using decimal
half-up rounding, and return ordinary Python floats in the result. The exact
behaviour is checked independently after every agent run.
