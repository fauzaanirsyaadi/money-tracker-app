import tempfile
import unittest
from pathlib import Path

import check_workflows as guard

SHA = "11d5960a326750d5838078e36cf38b85af677262"
HEAD = "name: CI\non:\n  push:\npermissions:\n  contents: read\njobs:\n  a:\n    steps:\n"


def workflow(*steps: str) -> str:
    return HEAD + "".join(f"      {step}\n" for step in steps)


class WorkflowTests(unittest.TestCase):
    def test_pinned_actions_pass_and_a_version_comment_is_fine(self):
        text = workflow(f"- uses: actions/checkout@{SHA} # v4.4.0", "- uses: ./local-action", "- run: echo hi")
        self.assertEqual(guard.check_workflow(text, "ci.yml"), [])

    def test_tags_branches_and_short_shas_are_refused(self):
        for ref in ("actions/checkout@v4", "actions/checkout@main", "dtolnay/rust-toolchain@stable",
                    "actions/checkout@11d5960", "actions/checkout", f"actions/checkout@{SHA[:-1]}",
                    f"actions/checkout@{SHA.upper()}"):
            found = guard.check_workflow(workflow(f"- uses: {ref}"), "ci.yml")
            self.assertEqual(len(found), 1, ref)

    def test_a_reusable_workflow_path_is_pinned_the_same_way(self):
        self.assertEqual(len(guard.check_workflow(workflow("uses: org/repo/.github/workflows/x.yml@main"), "ci.yml")), 1)
        self.assertEqual(guard.check_workflow(workflow(f"uses: org/repo/.github/workflows/x.yml@{SHA}"), "ci.yml"), [])

    def test_docker_images_need_a_digest(self):
        self.assertEqual(len(guard.check_workflow(workflow("- uses: docker://alpine:3.20"), "ci.yml")), 1)
        digest = "sha256:" + "a" * 64
        self.assertEqual(guard.check_workflow(workflow(f"- uses: docker://alpine@{digest}"), "ci.yml"), [])

    def test_dangerous_triggers_are_refused(self):
        for trigger in ("pull_request_target:", "workflow_run:"):
            text = HEAD.replace("  push:\n", f"  {trigger}\n") + "      - run: true\n"
            self.assertTrue(guard.check_workflow(text, "ci.yml"), trigger)
        text = HEAD.replace("  push:\n", "").replace("on:", "on: [push]") + "      - run: true\n"
        self.assertEqual(guard.check_workflow(text, "ci.yml"), [])

    def test_permissions_must_be_declared_at_the_top(self):
        text = "name: CI\non: [push]\njobs:\n  a:\n    steps:\n      - run: true\n"
        self.assertTrue(any("permissions" in p for p in guard.check_workflow(text, "ci.yml")))

    def test_piping_a_download_into_a_shell_is_refused(self):
        for run in ("curl -sSf https://x.example/install.sh | sh", "wget -qO- https://x.example | sudo bash"):
            self.assertEqual(len(guard.check_workflow(workflow(f"- run: {run}"), "ci.yml")), 1, run)


class CodeownersTests(unittest.TestCase):
    FULL = (
        "* @me\n/.github/workflows/ @me\n/.github/CODEOWNERS @me\n/scripts/ @me\n"
        "/scripts/guard_exceptions.json @me\n/deny.toml @me\n"
    )

    def test_a_complete_file_passes(self):
        self.assertEqual(guard.check_codeowners(self.FULL), ([], []))

    def test_a_missing_file_fails(self):
        problems, _ = guard.check_codeowners(None)
        self.assertEqual(len(problems), 1)

    def test_every_protected_path_needs_an_owner(self):
        problems, _ = guard.check_codeowners("/docs/ @me\n")
        self.assertEqual(len(problems), len(guard.CODEOWNERS_REQUIRED))
        # A catch-all rule covers them all.
        self.assertEqual(guard.check_codeowners("* @me\n")[0], [])

    def test_the_placeholder_owner_only_warns(self):
        problems, warnings = guard.check_codeowners(self.FULL.replace("@me", "@REPLACE-WITH-GITHUB-USERNAME"))
        self.assertEqual(problems, [])
        self.assertEqual(len(warnings), 1)

    def test_comments_are_ignored(self):
        problems, _ = guard.check_codeowners("# * @me\n/docs/ @me\n")
        self.assertTrue(problems)


class RepoTests(unittest.TestCase):
    def test_the_repository_itself_passes(self):
        repo = Path(__file__).resolve().parent.parent
        problems, _ = guard.check_repo(repo)
        self.assertEqual(problems, [])

    def test_a_repo_without_workflows_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            problems, _ = guard.check_repo(Path(tmp))
            self.assertTrue(problems)


if __name__ == "__main__":
    unittest.main()
