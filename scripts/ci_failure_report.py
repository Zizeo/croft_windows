#!/usr/bin/env python3
"""The failure report CI writes for whoever - or whatever - has to fix the red run.

A failed run here is seven jobs on two runner OSes and a six-figure line count
of log, and all of it is reachable through a web page that renders one job at a
time. The first minute of every fix is spent grepping for the line that says
what broke: a rustc error six thousand lines into `clippy + tests`, a python
gate's `::error::` under one of the doc-ownership steps, a zigbuild linker
message under a cross target. An agent asked to "make CI green" starts with the
same problem, except it pays for the whole log in its context window.

So the last job in ci.yml reads the run's own jobs, downloads the logs of the
ones that failed, and reduces them to this report: the failing steps, the lines
that look like causes with a little context, and the command GitHub echoed for
the step that produced them - which is also the command to run locally.

It is also runnable over a run that already happened - `workflow_dispatch` on
ci-failure-report.yml takes a run id, and locally this script reads a jobs dump
and a logs directory - so asking "what broke there?" costs one log download
rather than one full CI run.

The full logs travel with it as an artifact, so nothing here is a lossy
bottleneck: the report is the entry point, not the ceiling.

Usage:
    ci_failure_report.py JOBS_JSON LOGS_DIR [--run RUN_JSON]

JOBS_JSON is `GET /repos/{owner}/{repo}/actions/runs/{id}/jobs`, LOGS_DIR holds
`<job id>.log` for whichever jobs downloaded, and RUN_JSON is
`GET /repos/{owner}/{repo}/actions/runs/{id}` when the report is about a run
other than the one writing it - the header then describes THAT run, not the
reader's. Without it, the GitHub env of the current run is used. Exit 0 when a
report was written, including when the logs were missing - a report that names
the failing steps is still worth more than nothing, and this must not add a
failure of its own.
"""

import json
import os
import re
import sys
from pathlib import Path

# A job that did not pass. `cancelled` counts: a run cancelled mid-build is a
# red run, and the report should say which job it was in.
FAILED = ("failure", "timed_out", "cancelled", "startup_failure")

# How much of each error's surroundings to print, and how many errors a job may
# contribute. Enough to see the assertion and its inputs; not the whole test.
CONTEXT_BEFORE = 4
CONTEXT_AFTER = 2
MAX_HITS = 8
# Two excerpts at most: past that point the reader wants the log, and the
# artifact has it.
MAX_RANGES_MERGED = 2
MAX_LINE = 400
# How much of a step's output to show when nothing in it matched a pattern.
TAIL = 15

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
# Raw job logs prefix every line with the instant it was written.
STAMP = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z ")
# `##[group]Run cargo clippy ...` opens a step; `##[endgroup]` closes it.
GROUP = re.compile(r"^##\[(group|endgroup)\](.*)$")

# What a cause looks like, across the toolchains this repo's jobs run. Kept to
# shapes that are wrong in every context: a line that matches is worth reading
# even when it turns out to be the consequence rather than the cause.
ERROR = re.compile(
    r"^error(?:\[E\d+\])?:"  # rustc, and cargo's "could not compile"
    r"|^error\b[^:]{0,80}$"  # a bare `error` marker line
    r"|panicked at"
    r"|^thread '.*' panicked"
    r"|^test result: FAILED"
    r"|^---- .* stdout ----"
    r"|^FAILED \("
    r"|^FAILED \S"  # unittest's "FAILED test_x (module.Class)"
    r"|^Traceback \(most recent call last\)"
    r"|^[A-Za-z_.]*(?:Error|Exception): "
    r"|error TS\d+"  # tsc
    r"|^npm ERR!"
    r"|^Diff in "  # cargo fmt --check
    r"|undefined symbol"  # NDK / zig linking
    r"|no artifact at "  # this repo's own artifact assertions
)

# `##[error]Process completed with exit code 1` is the runner's own bookkeeping
# and says nothing the step's output did not already say.
RUNNER_NOISE = "Process completed with exit code"


def clean(raw: str) -> str:
    """One log line without its timestamp, colour codes or carriage return."""
    line = ANSI.sub("", STAMP.sub("", raw))
    return line.rstrip("\r\n").rstrip()


def looks_like_error(line: str) -> bool:
    if RUNNER_NOISE in line:
        return False
    if "::error::" in line or "##[error]" in line:
        return True
    return bool(ERROR.search(line))


def printable(line: str) -> bool:
    """Whether a log line belongs in a printed excerpt.

    Commands (`##[group]...`) and the runner's own `Process completed with exit
    code` line are bookkeeping around the output, not output: inside an excerpt
    they read as two more lines of the compiler's complaint.
    """
    return not GROUP.match(line) and RUNNER_NOISE not in line


def read_text(text: str):
    """A log as lines, cleaned. Split out from `read_log` so the parser can be
    exercised on a fixture without a log file."""
    return [clean(raw) for raw in text.splitlines()]


def read_log(path: Path):
    """The log's lines, cleaned. A missing file is not an error here: the job
    may not have finished, or its log may not be downloadable yet."""
    if not path.is_file():
        return None
    return read_text(path.read_text(errors="replace"))


def hits(lines):
    """(line number, text, step command) for each line that looks like a cause.

    `step command` is the `Run ...` group header above it, taken verbatim from
    the log: GitHub echoes what the step actually ran, so it cannot drift from
    the workflow the way a copy of the command would.
    """
    found = []
    seen = set()
    command = ""
    for number, line in enumerate(lines, start=1):
        group = GROUP.match(line)
        if group:
            kind, text = group.groups()
            if kind == "group" and text.startswith("Run "):
                command = text[len("Run ") :].strip()
            continue
        if not looks_like_error(line):
            continue
        if len(line) > MAX_LINE:
            line = line[:MAX_LINE] + " ..."
        if line in seen:
            continue
        seen.add(line)
        found.append((number, line, command))
        if len(found) == MAX_HITS:
            break
    return found, command


def step_tail(lines, count=TAIL):
    """The end of the last step's output.

    The whole log's last lines are the runner's cleanup, which runs after the
    step that failed and says nothing about it; the failing step is the last
    `Run ...` group before those.
    """
    starts = []
    for index, line in enumerate(lines):
        group = GROUP.match(line)
        if group and group.group(1) == "group":
            starts.append((index, group.group(2).startswith("Run ")))
    for position in range(len(starts) - 1, -1, -1):
        start, is_run = starts[position]
        if not is_run:
            continue
        end = starts[position + 1][0] if position + 1 < len(starts) else len(lines)
        body = [l for l in lines[start:end] if printable(l) and l.strip()]
        if body:
            return body[-count:]
    return [l for l in lines if printable(l) and l.strip()][-count:]


def ranges(numbers, total):
    """Context windows around the hits, merged where they would overlap."""
    spans = []
    for number in numbers:
        start = max(1, number - CONTEXT_BEFORE)
        end = min(total, number + CONTEXT_AFTER)
        if spans and start <= spans[-1][1] + 1:
            spans[-1] = (spans[-1][0], max(spans[-1][1], end))
        else:
            spans.append((start, end))
    return spans


def render_job(job, logs_dir: Path, out):
    name = job.get("name", "(unnamed job)")
    conclusion = job.get("conclusion") or "unknown"
    out.write(f"## {name} - {conclusion}\n\n")
    url = job.get("html_url")
    if url:
        out.write(f"- Job page: {url}\n")
    steps = [
        step.get("name", "(unnamed step)")
        for step in job.get("steps") or []
        if step.get("conclusion") in FAILED
    ]
    if steps:
        out.write("- Failing steps: " + ", ".join(f"`{s}`" for s in steps) + "\n")

    lines = read_log(logs_dir / f"{job.get('id')}.log")
    if lines is None:
        out.write(
            "- Log: not downloaded. Open the job page above; nothing else in\n"
            "  this report was recoverable for it.\n\n"
        )
        return

    found, last_command = hits(lines)
    commands = []
    for _, _, command in found:
        if command and command not in commands:
            commands.append(command)
    if not commands and last_command:
        commands = [last_command]
    if commands:
        out.write("- Reproduce with:\n")
        for command in commands[:3]:
            out.write(f"  ```\n  {command}\n  ```\n")
    out.write("\n")

    if not found:
        out.write(
            "No line matched this report's error patterns. The log's last lines\n"
            "are below; the full log is in the `ci-failure-report` artifact.\n\n"
        )
        tail = step_tail(lines)
        out.write("```text\n" + "\n".join(tail) + "\n```\n\n")
        return

    spans = ranges([number for number, _, _ in found], len(lines))[:MAX_RANGES_MERGED]
    for index, (start, end) in enumerate(spans):
        label = "First error" if index == 0 else f"Error {index + 1}"
        out.write(f"### {label} (line {start}-{end} of the log)\n\n")
        out.write("```text\n")
        # Line numbers stay the raw log's, so a reader can grep the artifact
        # for the same line; the markers between them are dropped instead.
        out.write("\n".join(l for l in lines[start - 1 : end] if printable(l)))
        out.write("\n```\n\n")

    # A hit inside a printed excerpt has been read already.
    rest = [
        (number, line)
        for number, line, _ in found
        if not any(start <= number <= end for start, end in spans)
    ]
    if rest:
        out.write(f"### {len(rest)} further line(s) worth reading\n\n")
        for number, line in rest:
            out.write(f"- line {number}: `{line}`\n")
        out.write("\n")


def header(out, jobs, run=None):
    """The page's first lines: which run this is, and how many jobs failed.

    `run` is the runs API object when the report is about another run - a
    manual dispatch over yesterday's red build - and then its own fields win.
    Otherwise the current run's GitHub env is all there is.
    """
    failed = [j for j in jobs if j.get("conclusion") in FAILED]
    run = run or {}
    server = os.environ.get("GITHUB_SERVER_URL", "")
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    where = run.get("html_url") or ""
    if not where:
        run_id = os.environ.get("GITHUB_RUN_ID", "")
        where = f"{server}/{repo}/actions/runs/{run_id}" if repo and run_id else ""
    target = str(run.get("id") or os.environ.get("GITHUB_RUN_ID", ""))
    event = run.get("event") or os.environ.get("GITHUB_EVENT_NAME", "")
    ref = run.get("head_branch") or os.environ.get("GITHUB_REF_NAME", "")
    sha = (run.get("head_sha") or os.environ.get("GITHUB_SHA", "") or "")[:7]
    title = run.get("display_title") or ""

    out.write("# CI failure report\n\n")
    if where:
        out.write(f"Run: {where}\n")
    if title:
        out.write(f"Title: {title}\n")
    line = " · ".join(part for part in (event, ref, sha) if part)
    if line:
        out.write(f"Event: {line}\n")
    out.write("\n" if line or where else "")
    out.write(f"{len(failed)} of {len(jobs)} job(s) did not pass.\n\n")
    out.write(
        "Written by the `failure report for AI agents` job that runs after a red\n"
        "run. Each section names a failed job, the step(s) that failed, and the\n"
        "command GitHub echoed for it - the same command to reproduce locally.\n"
        "The full logs are the `ci-failure-report` artifact of this run"
    )
    if target:
        out.write(f", or `gh run view {target} --log-failed`")
    out.write(".\n\n")


def parse(argv):
    """Positional arguments, the optional `--run` metadata file, and whether
    `--run` was given without a value."""
    positional, run, bad = [], None, False
    arguments = iter(argv[1:])
    for argument in arguments:
        if argument == "--run":
            value = next(arguments, None)
            if value is None:
                bad = True
            else:
                run = value
        elif argument.startswith("--run="):
            run = argument.split("=", 1)[1]
            bad = bad or not run
        else:
            positional.append(argument)
    return positional, Path(run) if run else None, bad


def load(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError) as error:
        sys.stderr.write(f"error: cannot read {path}: {error}\n")
        return None


def main(argv):
    positional, run_path, bad = parse(argv)
    if bad or len(positional) != 2:
        sys.stderr.write(__doc__)
        return 2
    jobs_path, logs_dir = Path(positional[0]), Path(positional[1])
    payload = load(jobs_path)
    if payload is None:
        return 1
    run = None
    if run_path is not None:
        run = load(run_path)
        if run is None:
            return 1
    jobs = payload.get("jobs") or []
    out = sys.stdout
    header(out, jobs, run)
    for job in jobs:
        if job.get("conclusion") in FAILED:
            render_job(job, logs_dir, out)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
