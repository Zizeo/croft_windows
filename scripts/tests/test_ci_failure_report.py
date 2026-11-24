"""Tests for scripts/ci_failure_report.py: the report ci.yml writes when it goes red.

The report is only worth having if it can be trusted when the run is already
broken, so the cases here are the ways a log breaks a naive reader rather than a
coverage sweep: every line carries a timestamp prefix, the step's command lives
in a `##[group]Run ...` header rather than on the error line, the runner prints
its own `##[error]Process completed with exit code 1` after any failure, a
single rustc error repeats its line, and a step can fail after printing nothing
this script recognises. Each of those, read literally, produces a report that
points at the wrong thing - or at nothing - exactly when someone is relying on
it.
"""

from __future__ import annotations

import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import ci_failure_report as report  # noqa: E402

STAMP = "2026-11-24T01:03:44.1234567Z "


def log(*lines):
    """A raw job log: the timestamp prefix GitHub's log download puts on each."""
    return "".join(f"{STAMP}{line}\n" for line in lines)


def job(name="clippy + tests", conclusion="failure", job_id=42, steps=None):
    return {
        "name": name,
        "conclusion": conclusion,
        "id": job_id,
        "html_url": f"https://github.com/o/r/actions/runs/1/job/{job_id}",
        "steps": steps
        if steps is not None
        else [
            {"name": "Install the pinned toolchain", "conclusion": "success"},
            {"name": "Clippy (zero warnings)", "conclusion": "failure"},
            {"name": "Tests", "conclusion": "skipped"},
        ],
    }


class LogReading(unittest.TestCase):
    def test_the_timestamp_and_colour_codes_are_not_part_of_the_line(self):
        self.assertEqual(
            report.clean(f"{STAMP}\x1b[31merror[E0308]\x1b[0m: mismatched types\r"),
            "error[E0308]: mismatched types",
        )

    def test_a_timestamp_that_is_not_there_is_left_alone(self):
        self.assertEqual(report.clean("error: could not compile `croft`"), "error: could not compile `croft`")


class Causes(unittest.TestCase):
    def test_a_run_group_becomes_the_reproduce_command(self):
        lines = report.read_text(log(
            "##[group]Run rustup show",
            "rustup show",
            "##[endgroup]",
            "##[group]Run cargo clippy --all-targets -- -D warnings",
            "cargo clippy --all-targets -- -D warnings",
            "error[E0308]: mismatched types",
            "##[endgroup]",
        ))
        found, _ = report.hits(lines)
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0][2], "cargo clippy --all-targets -- -D warnings")

    def test_the_runners_exit_code_line_is_not_a_cause(self):
        """It follows every failure and names none of them."""
        lines = report.read_text(log("##[error]Process completed with exit code 1."))
        found, _ = report.hits(lines)
        self.assertEqual(found, [])

    def test_a_workflow_error_command_is_a_cause(self):
        """The python gates in ci.yml report with echo ::error::, which the log
        carries literally rather than as an annotation."""
        lines = report.read_text(log("::error::expected one croft-software .crate in target/package"))
        found, _ = report.hits(lines)
        self.assertEqual(len(found), 1)
        self.assertIn("expected one croft-software .crate", found[0][1])

    def test_a_repeated_line_is_reported_once(self):
        """rustc prints the failing expression twice; a report repeating it
        spends the reader's attention on the same fact."""
        lines = report.read_text(log("error[E0308]: mismatched types", "note: something", "error[E0308]: mismatched types"))
        found, _ = report.hits(lines)
        self.assertEqual(len(found), 1)

    def test_the_steps_own_script_is_not_output(self):
        """GitHub prints a `run:` step as the script, a `shell:` line and any
        `env:` inside the group. A setup step that says `echo "::error::pdftoppm
        is missing"` in that script would otherwise be quoted as the reason the
        NEXT step failed."""
        lines = report.read_text(log(
            "##[group]Run set -euo pipefail",
            '  || { echo "::error::pdftoppm is missing"; exit 1; }',
            "shell: /usr/bin/bash -e {0}",
            "env:",
            "  CARGO_TERM_COLOR: always",
            "##[endgroup]",
            "##[group]Run cargo clippy --all-targets -- -D warnings",
            "cargo clippy --all-targets -- -D warnings",
            "shell: /usr/bin/bash -e {0}",
            "##[endgroup]",
            "error[E0308]: mismatched types",
        ))
        found, _ = report.hits(lines)
        self.assertEqual([line for _, line, _ in found], ["error[E0308]: mismatched types"])
        self.assertEqual(found[0][2], "cargo clippy --all-targets -- -D warnings")

    def test_a_step_that_prints_inside_its_own_group_is_still_read(self):
        """Only a group carrying the `shell:`/`env:` trailer is the echoed
        command; anything else in a group is the step talking."""
        lines = report.read_text(log(
            "##[group]Run some composite step",
            "error: this step printed its own complaint",
            "##[endgroup]",
        ))
        found, _ = report.hits(lines)
        self.assertEqual([line for _, line, _ in found], ["error: this step printed its own complaint"])


class Rendering(unittest.TestCase):
    def render(self, jobs, logs, run=None, extra=()):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "jobs.json").write_text(json.dumps({"jobs": jobs}))
            logs_dir = root / "logs"
            logs_dir.mkdir()
            for job_id, text in logs.items():
                (logs_dir / f"{job_id}.log").write_text(text)
            if run is not None:
                (root / "run.json").write_text(json.dumps(run))
            out = io.StringIO()
            with redirect_stdout(out):
                status = report.main(
                    ["prog", str(root / "jobs.json"), str(logs_dir), *extra]
                )
        return status, out.getvalue()

    def test_overlapping_context_is_printed_once(self):
        """Two errors four lines apart share their context; printing both
        windows repeats the lines between them."""
        text = log(
            "##[group]Run cargo clippy --all-targets -- -D warnings",
            "a", "b", "c",
            "error[E0308]: mismatched types",
            "d", "e",
            "error[E0277]: the trait bound is not satisfied",
            "##[endgroup]",
        )
        status, out = self.render([job()], {42: text})
        self.assertEqual(status, 0)
        self.assertEqual(out.count("error[E0308]"), 1)
        self.assertEqual(out.count("First error"), 1)
        self.assertNotIn("Error 2", out)

    def test_a_job_with_no_recognised_line_shows_its_tail_instead(self):
        text = log("##[group]Run cargo zigbuild", "compiling something", "and then nothing")
        status, out = self.render([job()], {42: text})
        self.assertEqual(status, 0)
        self.assertIn("No line matched", out)
        self.assertIn("and then nothing", out)
        self.assertIn("cargo zigbuild", out)

    def test_the_tail_does_not_quote_the_steps_own_script(self):
        """When nothing matched, the tail is the fallback. If it is taken from
        the echoed command, the reader gets the setup step's source instead of
        what the failing step printed."""
        text = log(
            "##[group]Run set -euo pipefail",
            '  || { echo "::error::the setup step is not the failure"; exit 1; }',
            "shell: /usr/bin/bash -e {0}",
            "##[endgroup]",
            "only prose, no pattern matched",
            "##[group]Post job cleanup.",
            "Deleting the workspace directory",
            "##[endgroup]",
        )
        status, out = self.render([job()], {42: text})
        self.assertEqual(status, 0)
        self.assertIn("only prose, no pattern matched", out)
        self.assertNotIn("the setup step is not the failure", out)

    def test_the_tail_is_the_failing_steps_not_the_runners_cleanup(self):
        """The last lines of a job are the runner tearing it down. Printed as
        the step's output they show a cleanup message where the failure was."""
        text = log(
            "##[group]Run cargo build --locked --bin croft",
            "Compiling croft v0.2.11",
            "##[endgroup]",
            "##[group]Run the failing step's own prose",
            "this step failed after printing only prose",
            "##[endgroup]",
            "##[group]Post job cleanup.",
            "Deleting the workspace directory",
            "##[endgroup]",
        )
        status, out = self.render([job()], {42: text})
        self.assertEqual(status, 0)
        self.assertIn("this step failed after printing only prose", out)
        self.assertNotIn("Post job cleanup", out)
        self.assertNotIn("Deleting the workspace directory", out)

    def test_a_missing_log_names_the_job_and_still_writes_a_report(self):
        """The log download can fail - the job may still have been running - and
        a report that names the failed job is worth more than no report at all.
        This job must never be the reason a run reads redder than it is."""
        status, out = self.render([job()], {})
        self.assertEqual(status, 0)
        self.assertIn("clippy + tests", out)
        self.assertIn("Log: not downloaded", out)
        self.assertIn("Clippy (zero warnings)", out)

    def test_only_the_jobs_that_failed_get_a_section(self):
        status, out = self.render(
            [
                job(name="release notes", job_id=1, conclusion="success", steps=[]),
                job(name="windows", job_id=2, conclusion="cancelled", steps=[]),
            ],
            {2: log("error: could not compile `croft`")},
        )
        self.assertEqual(status, 0)
        self.assertNotIn("## release notes", out)
        self.assertIn("## windows - cancelled", out)
        self.assertIn("1 of 2 job(s) did not pass", out)

    def test_the_runners_bookkeeping_never_reaches_an_excerpt(self):
        """`##[error]Process completed with exit code 101.` is printed after
        every failure; inside the excerpt it reads as another line of the
        error. Only the step's own output belongs there."""
        text = log(
            "##[group]Run cargo test --locked",
            "error: test failed, to rerun pass --bin croft",
            "##[endgroup]",
            "##[error]Process completed with exit code 101.",
        )
        status, out = self.render([job()], {42: text})
        self.assertEqual(status, 0)
        self.assertIn("error: test failed", out)
        self.assertNotIn("Process completed with exit code", out)
        self.assertNotIn("##[endgroup]", out)

    def test_a_line_already_inside_an_excerpt_is_not_listed_again(self):
        text = log(
            "error[E0308]: mismatched types",
            *[f"filler {n}" for n in range(8)],
            "error: could not compile `croft` due to 1 previous error",
            *[f"pad {n}" for n in range(8)],
            "panicked at src/app/tests/files.rs:912:9:",
            *[f"pad2 {n}" for n in range(8)],
            "test result: FAILED. 1 passed; 1 failed",
        )
        status, out = self.render([job()], {42: text})
        self.assertEqual(status, 0)
        # Two excerpts at most; the two hits past them are listed, not excerpted.
        self.assertIn("### 2 further line(s) worth reading", out)
        self.assertIn("- line 19: `panicked at", out)
        self.assertIn("- line 28: `test result: FAILED", out)
        self.assertNotIn("- line 2: `error: could not compile", out)

    def test_the_failing_steps_come_from_the_run_not_from_the_log(self):
        """The log names commands, not steps; only the API says which step was
        red, and the reader needs both to map one to the other."""
        status, out = self.render([job()], {42: log("error: could not compile `croft`")})
        self.assertEqual(status, 0)
        self.assertIn("Failing steps: `Clippy (zero warnings)`", out)

    def test_an_unreadable_jobs_file_exits_nonzero_with_a_message(self):
        with tempfile.TemporaryDirectory() as tmp:
            out, err = io.StringIO(), io.StringIO()
            with redirect_stdout(out), redirect_stderr(err):
                status = report.main(["prog", str(Path(tmp) / "absent.json"), tmp])
        self.assertEqual(status, 1)
        self.assertIn("cannot read", err.getvalue())

    def test_no_arguments_prints_the_usage(self):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            status = report.main(["prog"])
        self.assertEqual(status, 2)
        self.assertIn("Usage:", err.getvalue())

    def test_run_without_a_value_is_a_usage_error(self):
        """The manual entry point is `--run <file>`; a bare `--run` would
        otherwise read as "no metadata" and silently describe the wrong run."""
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            status = report.main(["prog", "ci-jobs.json", "ci-logs", "--run"])
        self.assertEqual(status, 2)
        self.assertIn("Usage:", err.getvalue())


class Replay(unittest.TestCase):
    """The report over a run that is not the one writing it.

    A dispatch over last week's red build runs on today's default branch, so
    every field the header could take from its own environment - run id, event,
    branch, sha, title - describes the reader rather than the run under study.
    """

    target = {
        "id": 12345,
        "html_url": "https://github.com/o/r/actions/runs/12345",
        "display_title": "fix: the frame that was not there (#281)",
        "event": "pull_request",
        "head_branch": "fix/first-frame",
        "head_sha": "9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c",
    }

    def render(self, write_run=True, pass_run_flag=True):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "jobs.json").write_text(json.dumps({"jobs": [job()]}))
            (root / "logs").mkdir()
            if write_run:
                (root / "run.json").write_text(json.dumps(self.target))
            extra = ("--run", str(root / "run.json")) if pass_run_flag else ()
            out, err = io.StringIO(), io.StringIO()
            with redirect_stdout(out), redirect_stderr(err), mock.patch.dict(
                os.environ,
                {
                    "GITHUB_RUN_ID": "999",
                    "GITHUB_EVENT_NAME": "workflow_dispatch",
                    "GITHUB_REF_NAME": "main",
                    "GITHUB_SHA": "0000000000000000000000000000000000000000",
                    "GITHUB_REPOSITORY": "o/r",
                    "GITHUB_SERVER_URL": "https://github.com",
                },
            ):
                status = report.main(
                    ["prog", str(root / "jobs.json"), str(root / "logs"), *extra]
                )
        return status, out.getvalue(), err.getvalue()

    def test_the_header_describes_the_run_under_study(self):
        status, out, _ = self.render()
        self.assertEqual(status, 0)
        self.assertIn("https://github.com/o/r/actions/runs/12345", out)
        self.assertIn("fix: the frame that was not there (#281)", out)
        self.assertIn("pull_request · fix/first-frame · 9f8e7d6", out)
        self.assertIn("gh run view 12345 --log-failed", out)
        self.assertNotIn("runs/999", out)
        self.assertNotIn("workflow_dispatch", out)

    def test_a_run_file_that_cannot_be_read_is_an_error(self):
        """The workflow fetches it with the jobs, so a missing file means that
        call failed; reading on would print the reader's own run as if it were
        the one being explained."""
        status, _, err = self.render(write_run=False)
        self.assertEqual(status, 1)
        self.assertIn("cannot read", err)
        self.assertIn("run.json", err)

    def test_without_a_run_file_the_environment_is_the_source(self):
        status, out, _ = self.render(write_run=False, pass_run_flag=False)
        self.assertEqual(status, 0)
        self.assertIn("runs/999", out)
        self.assertIn("gh run view 999 --log-failed", out)


class Workflow(unittest.TestCase):
    """What ci-failure-report.yml promises, read from the file.

    GitHub, not a step, acts on these keys: only `workflow_dispatch` puts the
    manual entry point on the Actions page, and only reading a run id out of it
    lets that entry point report on a run other than its own.
    """

    workflow = (
        Path(__file__).resolve().parents[2]
        / ".github"
        / "workflows"
        / "ci-failure-report.yml"
    )

    def test_a_run_can_be_asked_for_by_id_without_rebuilding(self):
        text = self.workflow.read_text()
        self.assertIn("workflow_dispatch:", text)
        self.assertIn("run-id:", text)
        # Three callers, one variable: a dispatch names a run, a `workflow_call`
        # means the caller's own.
        self.assertIn("REQUESTED: ${{ inputs.run-id }}", text)
        self.assertIn("run=\"$CALLED\"", text)

    def test_the_render_step_is_told_which_run_it_describes(self):
        """Without `--run`, a replay's header would describe today's dispatch
        run - the reader - instead of last week's failure."""
        self.assertIn("--run ci-run.json", self.workflow.read_text())


if __name__ == "__main__":
    unittest.main()
