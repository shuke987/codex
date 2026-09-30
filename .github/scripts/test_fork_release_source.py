import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path


WORKFLOW = Path(__file__).parents[1] / "workflows" / "fork-release-candidate.yml"


class ForkReleaseSourceTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.origin = self.root / "origin"
        self.origin.mkdir()
        self.git(self.origin, "init", "-b", "codex/exec-goal")
        self.git(self.origin, "config", "user.name", "Release test")
        self.git(self.origin, "config", "user.email", "release@example.invalid")
        smoke = self.origin / ".github/scripts/smoke-fork-release.py"
        smoke.parent.mkdir(parents=True)
        smoke.write_text("# smoke script from the workflow revision\n")
        self.previous = self.commit("previous")
        self.maintenance = self.commit("maintenance")
        self.git(self.origin, "checkout", "-b", "upgrade")
        self.candidate = self.commit("candidate")
        # A later maintenance commit makes the synthetic merge distinct from
        # both source commits, as on a PR whose target branch has moved forward.
        self.git(self.origin, "checkout", "codex/exec-goal")
        self.maintenance = self.commit("new-maintenance")
        self.git(self.origin, "checkout", "upgrade")
        self.git(
            self.origin, "merge", "--no-ff", "codex/exec-goal", "-m", "synthetic merge"
        )
        self.checkout = self.root / "checkout"
        self.git(self.root, "clone", str(self.origin), str(self.checkout))
        self.runtime = self.root / "runtime"
        self.runtime.mkdir()

    def git(self, directory, *arguments):
        return subprocess.check_output(
            ["git", *arguments], cwd=directory, text=True, stderr=subprocess.PIPE
        ).strip()

    def commit(self, name):
        (self.origin / name).write_text(name)
        self.git(self.origin, "add", ".")
        self.git(self.origin, "commit", "-m", name)
        return self.git(self.origin, "rev-parse", "HEAD")

    def resolve(self, event, requested="", pr_head=""):
        step = WORKFLOW.read_text().split(
            "      - name: Resolve immutable source revision\n", 1
        )[1]
        script = textwrap.dedent(
            step.split("        run: |\n", 1)[1].split("      - uses:", 1)[0]
        )
        return subprocess.run(
            ["bash", "-c", script],
            cwd=self.checkout,
            env={
                **os.environ,
                "GITHUB_EVENT_NAME": event,
                "REQUESTED_REF": requested,
                "PR_HEAD_SHA": pr_head,
                "RUNNER_TEMP": str(self.runtime),
                "GITHUB_OUTPUT": str(self.runtime / "output"),
                "GITHUB_ENV": str(self.runtime / "environment"),
                "GITHUB_STEP_SUMMARY": str(self.runtime / "summary"),
            },
            text=True,
            capture_output=True,
            timeout=15,
        )

    def assert_source(self, result, expected):
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(expected, self.git(self.checkout, "rev-parse", "HEAD"))
        self.assertEqual(f"sha={expected}\n", (self.runtime / "output").read_text())
        self.assertEqual(
            f"SOURCE_SHA={expected}\n", (self.runtime / "environment").read_text()
        )
        self.assertTrue((self.runtime / "smoke-fork-release.py").exists())

    def test_pull_request_builds_its_exact_head_not_maintenance_or_merge(self):
        self.assertNotEqual(
            self.candidate, self.git(self.checkout, "rev-parse", "HEAD")
        )
        self.assert_source(
            self.resolve("pull_request", pr_head=self.candidate), self.candidate
        )

    def test_dispatch_defaults_to_current_maintenance_head(self):
        self.assert_source(self.resolve("workflow_dispatch"), self.maintenance)

    def test_dispatch_accepts_an_older_maintenance_commit(self):
        self.assert_source(
            self.resolve("workflow_dispatch", requested=self.previous), self.previous
        )

    def test_dispatch_rejects_a_commit_outside_maintenance_history(self):
        result = self.resolve("workflow_dispatch", requested=self.candidate)
        self.assertNotEqual(0, result.returncode)
        self.assertFalse((self.runtime / "output").exists())

    def test_dispatch_rejects_unpinned_or_missing_revisions(self):
        for requested in ("upgrade", "a" * 40, "$(touch unexpected)"):
            with self.subTest(requested=requested):
                self.assertNotEqual(
                    0, self.resolve("workflow_dispatch", requested=requested).returncode
                )
                self.assertFalse((self.runtime / "output").exists())

    def test_pull_request_requires_an_existing_exact_commit(self):
        for head in ("", "upgrade", "a" * 40):
            with self.subTest(head=head):
                self.assertNotEqual(
                    0, self.resolve("pull_request", pr_head=head).returncode
                )
                self.assertFalse((self.runtime / "output").exists())


if __name__ == "__main__":
    unittest.main()
