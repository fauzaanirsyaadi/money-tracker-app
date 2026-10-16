import unittest

import check_debug_redaction as guard


def problems(source: str) -> list[str]:
    return guard.check_text(source, "x.rs")


class DebugRedactionTests(unittest.TestCase):
    def test_a_struct_with_an_amount_is_flagged(self):
        self.assertEqual(len(problems("#[derive(Debug, Clone)]\npub struct T { pub amount_minor: i64 }")), 1)

    def test_names_notes_and_money_fields_are_flagged(self):
        for field in ("name: String", "note: String", "limit: Money", "balance: i64", "pin_hash: String"):
            self.assertEqual(len(problems(f"#[derive(Debug)]\nstruct T {{ {field} }}")), 1, field)

    def test_tuple_structs_are_checked_too(self):
        self.assertEqual(len(problems("#[derive(Debug)]\nstruct T(Money, u8);")), 1)

    def test_redacted_and_plain_types_pass(self):
        self.assertEqual(problems("#[derive(Clone)]\npub struct T { pub amount_minor: i64 }"), [])
        self.assertEqual(problems("#[derive(Debug)]\npub struct Cursor { id: i64, date: String }"), [])

    def test_the_exemption_comment_works_within_three_lines(self):
        src = "// debug-safe: counts only\n#[derive(Debug)]\npub struct T { pub total_rows: u32 }"
        self.assertEqual(problems(src), [])

    def test_comments_do_not_trigger_it(self):
        src = "#[derive(Debug)]\npub struct T {\n    // the amount of retries\n    pub tries: u32,\n}"
        self.assertEqual(problems(src), [])

    def test_enums_are_judged_by_their_payloads_not_their_variant_names(self):
        self.assertEqual(problems("#[derive(Debug)]\npub enum E { Overflow, InvalidAmount, NonPositiveLimit }"), [])
        self.assertEqual(len(problems("#[derive(Debug)]\npub enum E { Paid(Money), Other }")), 1)
        self.assertEqual(len(problems("#[derive(Debug)]\npub enum E { Named { note: String } }")), 1)

    def test_the_repository_itself_is_clean(self):
        from pathlib import Path

        repo = Path(__file__).resolve().parent.parent
        self.assertEqual(guard.check_repo(repo), [])


if __name__ == "__main__":
    unittest.main()
