import unittest

from calculator import build_sales_summary


class SalesSummaryTests(unittest.TestCase):
    def test_aggregates_sorts_and_rounds(self):
        report = "product,quantity,unit_price\nWidget,2,4.999\nGadget,1,20\nWidget,1,0.01\n"
        self.assertEqual(build_sales_summary(report), {
            "products": [
                {"product": "Gadget", "quantity": 1, "revenue": 20.0},
                {"product": "Widget", "quantity": 3, "revenue": 10.01},
            ],
            "grand_total": 30.01,
        })

    def test_supports_quoted_commas_and_blank_lines(self):
        result = build_sales_summary('product,quantity,unit_price\n"Blue, large",2,3.50\n\n')
        self.assertEqual(result["products"], [{"product": "Blue, large", "quantity": 2, "revenue": 7.0}])

    def test_trims_names_and_sorts_ties_by_name(self):
        report = "product,quantity,unit_price\n Zed ,1,2\nAlpha,2,1\n"
        self.assertEqual([p["product"] for p in build_sales_summary(report)["products"]], ["Alpha", "Zed"])

    def test_rejects_non_string_and_bad_header(self):
        for value in (None, "", "product,quantity\nA,1\n", "name,quantity,unit_price\nA,1,1\n"):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    build_sales_summary(value)

    def test_rejects_bad_rows_with_line_number(self):
        cases = ("A,0,1", "A,1,-1", "A,one,1", ",1,1", "A,1,wat")
        for row in cases:
            with self.subTest(row=row):
                with self.assertRaisesRegex(ValueError, r"line 2"):
                    build_sales_summary("product,quantity,unit_price\n" + row + "\n")


if __name__ == "__main__":
    unittest.main()
