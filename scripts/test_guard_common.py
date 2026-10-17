import json
import tempfile
import unittest
from pathlib import Path

import guard_common


def repo_with(tmp: str, data) -> Path:
    repo = Path(tmp)
    (repo / "scripts").mkdir()
    text = data if isinstance(data, str) else json.dumps(data)
    (repo / "scripts" / "guard_exceptions.json").write_text(text)
    return repo


class ExceptionsFileTests(unittest.TestCase):
    def test_a_missing_file_means_no_exceptions(self):
        with tempfile.TemporaryDirectory() as tmp:
            ex = guard_common.load(Path(tmp))
            self.assertEqual((ex.problems, ex.entries), ([], {}))

    def test_a_valid_file_loads_and_matches(self):
        data = {
            "comment": "free text",
            "debug_redaction": [{"file": "a.rs", "type": "T", "reason": "because"}],
            "offline_source": [{"file": "b.rs", "contains": "std::net", "reason": "docs"}],
        }
        with tempfile.TemporaryDirectory() as tmp:
            ex = guard_common.load(repo_with(tmp, data))
            self.assertEqual(ex.problems, [])
            self.assertTrue(ex.allows("debug_redaction", file="a.rs", type="T"))
            self.assertFalse(ex.allows("debug_redaction", file="a.rs", type="U"))
            self.assertTrue(ex.allows("offline_source", file="b.rs", contains="use std::net::Foo;"))
            self.assertFalse(ex.allows("offline_source", file="c.rs", contains="use std::net::Foo;"))
            self.assertEqual(ex.unused(("debug_redaction", "offline_source")), [])

    def test_every_entry_needs_a_reason(self):
        for entry in ({"file": "a.rs", "type": "T"}, {"file": "a.rs", "type": "T", "reason": "  "},
                      {"file": "a.rs", "reason": "x"}, "not an object"):
            with tempfile.TemporaryDirectory() as tmp:
                ex = guard_common.load(repo_with(tmp, {"debug_redaction": [entry]}))
                self.assertTrue(ex.problems, entry)
                self.assertEqual(ex.entries, {})

    def test_unknown_guards_and_broken_files_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertTrue(guard_common.load(repo_with(tmp, {"whatever": []})).problems)
        with tempfile.TemporaryDirectory() as tmp:
            self.assertTrue(guard_common.load(repo_with(tmp, "{not json")).problems)
        with tempfile.TemporaryDirectory() as tmp:
            self.assertTrue(guard_common.load(repo_with(tmp, "[]")).problems)

    def test_an_empty_contains_never_matches_everything(self):
        ex = guard_common.Exceptions()
        ex.entries["offline_source"] = [{"file": "a.rs", "contains": "", "reason": "x"}]
        self.assertFalse(ex.allows("offline_source", file="a.rs", contains="anything"))

    def test_unused_entries_are_reported(self):
        ex = guard_common.Exceptions()
        ex.entries["debug_redaction"] = [{"file": "a.rs", "type": "T", "reason": "x"}]
        self.assertEqual(len(ex.unused(("debug_redaction",))), 1)
        ex.allows("debug_redaction", file="a.rs", type="T")
        self.assertEqual(ex.unused(("debug_redaction",)), [])

    def test_inline_markers_are_found_with_their_line(self):
        found = guard_common.inline_marker_problems("a\n// debug-safe: x\nb // offline-check: allow", "f.rs")
        self.assertEqual(len(found), 2)
        self.assertIn("f.rs:2", found[0])
        self.assertEqual(guard_common.inline_marker_problems("nothing here", "f.rs"), [])

    def test_the_real_exceptions_file_is_valid_and_every_entry_has_a_reason(self):
        repo = Path(__file__).resolve().parent.parent
        ex = guard_common.load(repo)
        self.assertEqual(ex.problems, [])
        for entries in ex.entries.values():
            for entry in entries:
                self.assertGreater(len(entry["reason"]), 10)


if __name__ == "__main__":
    unittest.main()
