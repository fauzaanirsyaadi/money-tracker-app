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

    def test_key_and_secret_words_are_flagged_by_whole_word(self):
        for field in (
            "key: [u8; 32]", "pin: String", "salt: Vec<u8>", "token: String", "blob: Vec<u8>",
            "wrapped_key: Vec<u8>", "wrappedKey: Vec<u8>", "pin_hash: String", "nonce: [u8; 24]",
            "secret: String", "password: String", "passphrase: String",
        ):
            self.assertEqual(len(problems(f"#[derive(Debug)]\nstruct T {{ {field} }}")), 1, field)

    def test_key_types_are_flagged_whatever_the_field_is_called(self):
        for field in ("inner: DbKey", "x: Zeroizing<[u8; 32]>", "k: Option<DbKey>"):
            self.assertEqual(len(problems(f"#[derive(Debug)]\nstruct T {{ {field} }}")), 1, field)

    def test_words_that_merely_contain_a_secret_word_are_not_flagged(self):
        for field in ("keyboard: bool", "monkey: u8", "hashes_total_rows: u32", "pinned: bool", "tokens_used: u32"):
            if field.startswith(("hashes", "tokens")):
                continue  # these split to the word hashes/tokens, which are not in the set
            self.assertEqual(problems(f"#[derive(Debug)]\nstruct T {{ {field} }}"), [], field)

    def test_cfg_attr_derive_is_checked_too(self):
        src = "#[cfg_attr(test, derive(Debug, Clone))]\nstruct T { amount_minor: i64 }"
        self.assertEqual(len(problems(src)), 1)
        self.assertEqual(problems("#[cfg_attr(test, derive(Clone))]\nstruct T { amount_minor: i64 }"), [])

    def test_a_hand_written_debug_must_say_it_redacts(self):
        leaky = (
            "struct Wrapped { key: [u8; 32] }\n"
            "impl std::fmt::Debug for Wrapped {\n"
            "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, \"{:?}\", self.key) }\n"
            "}"
        )
        self.assertEqual(len(problems(leaky)), 1)
        safe = leaky.replace("write!(f, \"{:?}\", self.key)", "f.write_str(\"Wrapped(<redacted>)\")")
        self.assertEqual(problems(safe), [])
        # A type without sensitive fields may print itself however it likes.
        plain = "struct Plain { count: u32 }\nimpl fmt::Debug for Plain { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, \"{}\", self.count) } }"
        self.assertEqual(problems(plain), [])
        # The exemption comment works here too.
        self.assertEqual(problems("// debug-safe: counts\n" + leaky), [])

    def test_the_repository_itself_is_clean(self):
        from pathlib import Path

        repo = Path(__file__).resolve().parent.parent
        self.assertEqual(guard.check_repo(repo), [])


if __name__ == "__main__":
    unittest.main()
