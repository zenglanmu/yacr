#!/usr/bin/env python3
"""Push submodule commits referenced by outgoing superproject refs.

A superproject commit records a submodule commit that must exist on the
submodule's remote, otherwise every CI job dies in `actions/checkout` with
`fatal: remote error: upload-pack: not our ref <sha>`. This script runs as a
git `pre-push` hook (see `.githooks/pre-push`), inspects the gitlinks of every
ref being pushed, and pushes the containing submodule branch when the recorded
commit is not yet reachable from the submodule's remote-tracking refs.

Fail-closed contract: if a referenced submodule commit cannot be made reachable
(not cloned, detached/unreferenced, push rejected) the hook exits non-zero and
git aborts the superproject push. There is no silent success path.

Usage (git pre-push protocol: reads "<local-ref> <local-sha> <remote-ref>
<remote-sha>" lines on stdin):

    scripts/pre-push-submodules.py --remote origin --url <superproject-url>

Options:
    --dry-run   report what would be pushed, change nothing
    --no-push   verify only; block when a commit is missing from the remote

Environment:
    SKIP_SUBMODULE_PUSH_CHECK=1   bypass (prints a loud warning; CI will fail)
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from dataclasses import dataclass

ZERO_SHA = "0" * 40


class Failure(Exception):
    """Blocking condition: message is shown to the user, exit code 1."""


@dataclass(frozen=True)
class Gitlink:
    path: str
    sha: str


def run_git(args: list[str], cwd: str | None = None) -> str:
    proc = subprocess.run(
        ["git", *args], cwd=cwd, text=True, capture_output=True, check=False
    )
    if proc.returncode != 0:
        raise Failure(
            f"git {' '.join(args)} failed in {cwd or os.getcwd()}: "
            f"{(proc.stderr or proc.stdout).strip()}"
        )
    return proc.stdout


def try_git(args: list[str], cwd: str | None = None) -> tuple[bool, str]:
    proc = subprocess.run(
        ["git", *args], cwd=cwd, text=True, capture_output=True, check=False
    )
    return proc.returncode == 0, (proc.stdout if proc.returncode == 0 else proc.stderr)


def read_outgoing_refs(stream: str) -> list[tuple[str, str]]:
    """Return (local_ref, local_sha) for refs that actually move objects."""
    refs: list[tuple[str, str]] = []
    for line in stream.splitlines():
        parts = line.split()
        if len(parts) != 4:
            continue
        local_ref, local_sha, _remote_ref, _remote_sha = parts
        if set(local_sha) == {"0"}:
            continue  # ref deletion: nothing to build
        refs.append((local_ref, local_sha))
    return refs


def gitlinks_at(commit: str, cwd: str) -> list[Gitlink]:
    """Submodule (gitlink) entries recorded by a superproject commit."""
    ok, _ = try_git(["cat-file", "-e", f"{commit}^{{commit}}"], cwd)
    if not ok:
        raise Failure(f"cannot resolve pushed commit {commit} in {cwd}")
    links: list[Gitlink] = []
    for line in run_git(["ls-tree", "-r", commit], cwd).splitlines():
        meta, _, path = line.partition("\t")
        fields = meta.split()
        if len(fields) >= 3 and fields[0] == "160000":
            links.append(Gitlink(path=path, sha=fields[2]))
    return links


def current_branch(cwd: str) -> str | None:
    ok, out = try_git(["symbolic-ref", "--quiet", "--short", "HEAD"], cwd)
    return out.strip() if ok else None


def submodule_remote(cwd: str) -> str:
    branch = current_branch(cwd)
    if branch:
        ok, out = try_git(["config", f"branch.{branch}.remote"], cwd)
        if ok and out.strip():
            return out.strip()
    ok, out = try_git(["config", "remote.origin.url"], cwd)
    if not ok:
        raise Failure(
            f"submodule at {cwd} has no usable remote: "
            "set branch.<name>.remote or add an 'origin' remote"
        )
    return "origin"


def reachable_from_remote(cwd: str, remote: str, sha: str) -> bool:
    """True when sha is contained in some remote-tracking ref of `remote`."""
    ok, out = try_git(
        [
            "for-each-ref",
            "--contains",
            sha,
            f"--format=%(refname)",
            f"refs/remotes/{remote}/",
        ],
        cwd,
    )
    return ok and bool(out.strip())


def remote_of_containing_branch(cwd: str, sha: str) -> list[tuple[str, str]]:
    """Local (branch, upstream-ref-or-empty) pairs containing sha, current first."""
    out = run_git(
        [
            "for-each-ref",
            "--contains",
            sha,
            "--format=%(refname:short)\t%(upstream:remoteref)",
            "refs/heads/",
        ],
        cwd,
    )
    pairs: list[tuple[str, str]] = []
    for line in out.splitlines():
        if not line.strip():
            continue
        branch, _, upstream = line.partition("\t")
        pairs.append((branch.strip(), upstream.strip()))
    head = current_branch(cwd)
    pairs.sort(key=lambda item: item[0] != head)
    return pairs


def fetch_remote(cwd: str, remote: str) -> None:
    ok, err = try_git(["fetch", "--quiet", "--prune", remote], cwd)
    if not ok:
        raise Failure(
            f"cannot fetch submodule remote '{remote}' in {cwd}: {err.strip()}"
        )


def ensure_reachable(path: str, sha: str, *, dry_run: bool, push: bool) -> str | None:
    """Make `sha` reachable on the submodule remote. Returns a report line."""
    if not os.path.isdir(path):
        raise Failure(
            f"submodule '{path}' is not initialized (path is missing); run "
            "`git submodule update --init --recursive` before pushing"
        )
    ok, err = try_git(["rev-parse", "--git-dir"], path)
    if not ok:
        raise Failure(
            f"submodule '{path}' is not initialized ({err.strip()}); run "
            "`git submodule update --init --recursive` before pushing"
        )
    ok, _ = try_git(["cat-file", "-e", f"{sha}^{{commit}}"], path)
    if not ok:
        raise Failure(
            f"submodule '{path}' does not contain commit {sha}; run "
            "`git submodule update --init --recursive` (recorded in the commit "
            "you are pushing but missing locally)"
        )

    remote = submodule_remote(path)
    fetch_remote(path, remote)
    if reachable_from_remote(path, remote, sha):
        return None  # already published; nothing to do

    candidates = remote_of_containing_branch(path, sha)
    if not candidates:
        raise Failure(
            f"submodule '{path}': commit {sha} is not on any local branch, so it "
            f"cannot be pushed to '{remote}'. Create a branch there "
            "(git -C <sub> branch <name> <sha>) and push it, or amend the "
            "superproject commit to a published submodule commit."
        )

    branch, upstream = candidates[0]
    destination = upstream or f"refs/heads/{branch}"
    where = f"{path} ({branch} -> {remote} {destination})"
    if dry_run or not push:
        verb = "would push" if dry_run else "must push"
        return f"{verb} submodule '{where}' for commit {sha}"

    ok, err = try_git(["push", remote, f"refs/heads/{branch}:{destination}"], path)
    if not ok:
        raise Failure(f"submodule push failed for '{where}': {err.strip()}")
    fetch_remote(path, remote)
    if not reachable_from_remote(path, remote, sha):
        raise Failure(
            f"submodule push to '{where}' reported success but commit {sha} is "
            "still not reachable from the remote; refusing to push the "
            "superproject"
        )
    return f"pushed submodule '{where}' for commit {sha}"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--remote", default="origin")
    parser.add_argument("--url", default="")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-push", action="store_true")
    parser.add_argument("--stdin", default="-", help="pre-push stdin (default: -)")
    args = parser.parse_args(argv)

    if os.environ.get("SKIP_SUBMODULE_PUSH_CHECK") == "1":
        print(
            "pre-push: WARNING SKIP_SUBMODULE_PUSH_CHECK=1 is set; submodule "
            "commits were NOT verified. CI checkout will fail if the recorded "
            "submodule commit is not published.",
            file=sys.stderr,
        )
        return 0

    root = run_git(["rev-parse", "--show-toplevel"]).strip()
    stream = sys.stdin.read() if args.stdin == "-" else open(args.stdin).read()
    refs = read_outgoing_refs(stream)
    if not refs:
        return 0

    reports: list[str] = []
    seen: set[tuple[str, str]] = set()
    for local_ref, sha in refs:
        for link in gitlinks_at(sha, root):
            key = (link.path, link.sha)
            if key in seen:
                continue
            seen.add(key)
            sub_path = os.path.join(root, link.path)
            report = ensure_reachable(
                sub_path, link.sha, dry_run=args.dry_run, push=not args.no_push
            )
            if report:
                reports.append(f"{local_ref}: {report}")

    for report in reports:
        print(f"pre-push: {report}", file=sys.stderr)
    if reports and (args.dry_run or args.no_push):
        if args.no_push:
            raise Failure(
                "submodule commits are not published on their remotes "
                "(listed above); push them first"
            )
        return 0
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except Failure as failure:
        print(f"pre-push: BLOCKED {failure}", file=sys.stderr)
        sys.exit(1)
