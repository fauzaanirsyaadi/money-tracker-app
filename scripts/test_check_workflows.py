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


class HardeningTests(unittest.TestCase):
    def problems(self, *steps: str, head: str = HEAD) -> list[str]:
        return guard.check_workflow(head + "".join(f"      {step}\n" for step in steps), "ci.yml")

    def test_uses_in_every_yaml_spelling_is_pinned(self):
        for step in ('- "uses": actions/checkout@v4', "- uses : actions/checkout@main", "- {uses: actions/checkout@v4}",
                     "- name: x, uses: actions/checkout@v4", "- uses:\n          actions/checkout@v4",
                     "- 'uses': actions/checkout@main"):
            self.assertEqual(len(self.problems(step)), 1, step)
        self.assertEqual(self.problems(f"- {{uses: actions/checkout@{SHA}}}"), [])
        self.assertEqual(self.problems(f"- uses:\n          actions/checkout@{SHA}"), [])

    def test_container_and_service_images_need_a_digest(self):
        self.assertEqual(len(self.problems("container: node:22")), 1)
        self.assertEqual(len(self.problems("image: postgres:16")), 1)
        self.assertEqual(self.problems("image: postgres@sha256:" + "a" * 64), [])
        self.assertEqual(self.problems("image: ${{ matrix.image }}"), [])

    def test_the_dangerous_triggers_in_any_form(self):
        for on in ("on: pull_request_target", "on: [push, pull_request_target]", "on: {workflow_run: {workflows: [x]}}",
                   "on:\n  workflow_run:\n    workflows: [x]"):
            text = "name: X\n" + on + "\npermissions:\n  contents: read\njobs: {}\n"
            self.assertTrue(any("triggers" in p for p in guard.check_workflow(text, "x.yml")), on)

    def test_write_permissions_are_refused_in_every_shape(self):
        for perm in ("permissions: write-all", "permissions: write", "permissions:\n      contents: write",
                     "permissions:\n      id-token: write\n      contents: read"):
            text = "name: X\non: [push]\npermissions:\n  contents: read\njobs:\n  a:\n    " + perm + "\n    steps: []\n"
            self.assertTrue(any("permission" in p for p in guard.check_workflow(text, "x.yml")), perm)
        self.assertEqual(guard.check_workflow("name: X\non: [push]\npermissions: read-all\njobs: {}\n", "x.yml"), [])

    def test_untrusted_expressions_are_refused_inside_scripts_but_fine_in_env(self):
        for expr in ("${{ github.event.pull_request.title }}", "${{ github.head_ref }}", "${{ inputs.name }}",
                     "${{ github.event.issue.body }}"):
            self.assertEqual(len(self.problems(f"- run: echo {expr}")), 1, expr)
            self.assertEqual(len(self.problems("- run: |", f"    echo {expr}")), 1, expr)
        self.assertEqual(self.problems("- run: echo $TITLE", "  env:", "    TITLE: ${{ github.event.pull_request.title }}"), [])
        self.assertEqual(self.problems("- run: echo ${{ github.sha }}"), [])

    def test_downloads_piped_into_an_interpreter_in_any_form_are_refused(self):
        for command in ("curl -s https://x.example | sh", "curl https://x.example |sudo bash", "wget -qO- https://x.example | python3",
                        "bash <(curl -s https://x.example)", 'sh -c "$(curl -fsSL https://x.example)"'):
            self.assertEqual(len(self.problems(f"- run: {command}")), 1, command)
        self.assertEqual(len(self.problems("- run: |", "    curl -s https://x.example \\", "      | sh")), 1)

    def test_composite_actions_in_the_repository_are_scanned(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            (repo / ".github" / "workflows").mkdir(parents=True)
            (repo / ".github" / "workflows" / "ci.yml").write_text(HEAD + "      - uses: ./.github/actions/x\n")
            action = repo / ".github" / "actions" / "x"
            action.mkdir(parents=True)
            (action / "action.yml").write_text("name: x\nruns:\n  using: composite\n  steps:\n    - uses: actions/checkout@v4\n")
            problems, _ = guard.check_repo(repo)
            self.assertTrue(any("action.yml" in p for p in problems), problems)


class CodeownersTests(unittest.TestCase):
    FULL = (
        "* @me\n/.github/workflows/ @me\n/.github/actions/ @me\n/.github/CODEOWNERS @me\n/scripts/ @me\n"
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
        # A catch-all rule does NOT stand in for them: a later specific rule can override it.
        self.assertEqual(len(guard.check_codeowners("* @me\n")[0]), len(guard.CODEOWNERS_REQUIRED))

    def test_the_placeholder_owner_fails_in_ci_and_warns_locally(self):
        text = self.FULL.replace("@me", "@REPLACE-WITH-GITHUB-USERNAME")
        problems, warnings = guard.check_codeowners(text, in_ci=True)
        self.assertEqual((len(problems), warnings), (1, []))
        problems, warnings = guard.check_codeowners(text, in_ci=False)
        self.assertEqual((problems, len(warnings)), ([], 1))

    def test_the_placeholder_owner_only_warns(self):
        problems, warnings = guard.check_codeowners(self.FULL.replace("@me", "@REPLACE-WITH-GITHUB-USERNAME"))
        self.assertEqual(problems, [])
        self.assertEqual(len(warnings), 1)

    def test_comments_are_ignored(self):
        problems, _ = guard.check_codeowners("# * @me\n/docs/ @me\n")
        self.assertTrue(problems)


class TrackedFilesTests(unittest.TestCase):
    def test_bytecode_keys_and_packages_are_refused_wherever_they_sit(self):
        for path in ("scripts/__pycache__/guard_common.cpython-312.pyc", "x/y.pyc", "a/B.PYC", "release.jks",
                     "k/my.keystore", "c.p12", "app.apk", "bundle.aab", "src/__pycache__/x"):
            self.assertEqual(len(guard.forbidden_tracked([path])), 1, path)

    def test_ordinary_files_pass(self):
        paths = ["scripts/guard_common.py", "src/lib/api.ts", "docs/pycache-notes.md", "apkg.txt", "jks.md"]
        self.assertEqual(guard.forbidden_tracked(paths), [])


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
