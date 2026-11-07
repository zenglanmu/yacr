#!/usr/bin/env python3
"""Behavior contracts for ``scripts/pre-push-submodules.py``.

Each case builds a throwaway superproject + submodule pair inside a temporary
directory (bare "remote" repositories on the local filesystem), so no network
and no real repository mutation is involved. Stdlib only.

Covered: a submodule commit that is already published (no-op), an unpublished
submodule commit (blocked by ``--no-push``, reported by ``--dry-run``, pushed by
the default mode and then idempotent), an uninitialized submodule (blocked), a
detached submodule commit on no branch (blocked), and ref deletion lines being
ignored.
"""
from __future__ import annotations

import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
HOOK = ROOT / "scripts" / "pre-push-submodules.py"


def git(*args: str, cwd: pathlib.Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
        env={
            **os.environ,
            "GIT_AUTHOR_NAME": "test",
            "GIT_AUTHOR_EMAIL": "test@example.invalid",
            "GIT_COMMITTER_NAME": "test",
            "GIT_COMMITTER_EMAIL": "test@example.invalid",
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_SYSTEM": os.devnull,
            "GIT_TERMINAL_PROMPT": "0",
        },
    )


def git_ok(*args: str, cwd: pathlib.Path) -> str:
    proc = git(*args, cwd=cwd)
    if proc.returncode != 0:
        raise AssertionError(f"git {' '.join(args)} failed: {proc.stderr}")
    return proc.stdout.strip()


class PrePushSubmoduleContracts(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.base = pathlib.Path(self._tmp.name)
        self.base.mkdir(parents=True, exist_ok=True)
        self.sub_remote = self.base / "sub-remote.git"
        self.super_remote = self.base / "super-remote.git"
        git_ok("init", "--bare", "-b", "main", str(self.sub_remote), cwd=self.base)
        git_ok("init", "--bare", "-b", "main", str(self.super_remote), cwd=self.base)

        seed = self.base / "sub-seed"
        git_ok("init", "-b", "main", str(seed), cwd=self.base)
        (seed / "a.txt").write_text("one\n", encoding="utf-8")
        git_ok("add", "a.txt", cwd=seed)
        git_ok("commit", "-m", "init", cwd=seed)
        git_ok("remote", "add", "origin", str(self.sub_remote), cwd=seed)
        git_ok("push", "-u", "origin", "main", cwd=seed)

        self.super = self.base / "super"
        git_ok("init", "-b", "main", str(self.super), cwd=self.base)
        (self.super / "README.md").write_text("super\n", encoding="utf-8")
        git_ok("add", "README.md", cwd=self.super)
        git_ok("commit", "-m", "init", cwd=self.super)
        git_ok(
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            str(self.sub_remote),
            "plugins/tch",
            cwd=self.super,
        )
        git_ok("commit", "-m", "add submodule", cwd=self.super)
        git_ok("remote", "add", "origin", str(self.super_remote), cwd=self.super)
        self.sub = self.super / "plugins" / "tch"

    # -- helpers ---------------------------------------------------------
    def hook(self, *args: str, sha: str | None = None) -> subprocess.CompletedProcess:
        sha = sha or git_ok("rev-parse", "HEAD", cwd=self.super)
        line = f"refs/heads/main {sha} refs/heads/main {'1' * 40}\n"
        return subprocess.run(
            [sys.executable, str(HOOK), "--stdin", "-", *args],
            cwd=self.super,
            input=line,
            capture_output=True,
            text=True,
            check=False,
            env={**os.environ, "GIT_TERMINAL_PROMPT": "0"},
        )

    def commit_unpublished_submodule_change(self) -> str:
        (self.sub / "a.txt").write_text("two\n", encoding="utf-8")
        git_ok("commit", "-am", "sub work", cwd=self.sub)
        git_ok("add", "plugins/tch", cwd=self.super)
        git_ok("commit", "-m", "bump submodule", cwd=self.super)
        return git_ok("rev-parse", "HEAD", cwd=self.sub)

    # -- contracts -------------------------------------------------------
    def test_published_submodule_commit_is_a_noop(self) -> None:
        proc = self.hook("--no-push")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertNotIn("pre-push:", proc.stderr)

    def test_unpublished_commit_blocks_without_push(self) -> None:
        sub_sha = self.commit_unpublished_submodule_change()
        proc = self.hook("--no-push")
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn("BLOCKED", proc.stderr)
        self.assertIn(sub_sha, proc.stderr)

    def test_dry_run_reports_without_pushing(self) -> None:
        sub_sha = self.commit_unpublished_submodule_change()
        proc = self.hook("--dry-run")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("would push", proc.stderr)
        self.assertNotIn(
            sub_sha, git_ok("ls-remote", str(self.sub_remote), "main", cwd=self.base)
        )

    def test_default_mode_pushes_then_becomes_noop(self) -> None:
        sub_sha = self.commit_unpublished_submodule_change()
        proc = self.hook()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("pushed submodule", proc.stderr)
        self.assertIn(
            sub_sha, git_ok("ls-remote", str(self.sub_remote), "main", cwd=self.base)
        )
        again = self.hook()
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertNotIn("pushed submodule", again.stderr)

    def test_uninitialized_submodule_blocks(self) -> None:
        shutil.rmtree(self.super / "plugins" / "tch")
        proc = self.hook("--no-push")
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn("not initialized", proc.stderr)

    def test_detached_submodule_commit_blocks(self) -> None:
        sub_sha = self.commit_unpublished_submodule_change()
        git_ok("checkout", "--detach", sub_sha, cwd=self.sub)
        git_ok("branch", "-D", "main", cwd=self.sub)
        proc = self.hook()
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn("not on any local branch", proc.stderr)

    def test_ref_deletion_is_ignored(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(HOOK)],
            cwd=self.super,
            input=f"(delete) {'0' * 40} refs/heads/gone {'1' * 40}\n",
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
