#!/usr/bin/env python3
"""Validate the layered GitHub Actions CI workflows (N02).

Standard library only. What it checks:

  * at least one workflow file exists under ``.github/workflows`` and is non-empty;
  * every required job is declared as a job key in some workflow
    (``core-quality``, ``wasm-check``, ``i18n-contracts``, ``shader-validation``,
     ``linux-app``, ``linux-release``, ``windows-check``, ``windows-release``,
     ``macos-check``, ``macos-release``, ``android-release``, ``web-build``,
     ``web-host-contracts``, ``web-deploy``, ``android-check``, ``android-apk``,
     ``web-smoke``; the capability-gated ones must still be *declared* so
     their absence is visible rather than silent);
  * no required job declares ``continue-on-error: true`` (job-level or step-level);
  * each required job still contains its expected command fragment, so the job
    keeps mirroring the real gate instead of drifting into an empty success.

PyYAML is used when importable for a real parse. Otherwise it falls back to a
structural text check and says so explicitly. The script is a guard rail, not a
replacement for a YAML parser or for actually running the workflow.
"""
from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKFLOW_DIR = ROOT / ".github" / "workflows"
WORKFLOW_SUFFIXES = (".yml", ".yaml")

# Job key -> command fragments that must appear inside that job.
#
# `shader-validation`, `web-build`, `web-host-contracts`, `web-deploy`,
# `android-check`, `android-apk` and `web-smoke` are all required to be
# *declared*. The gated jobs (`web-deploy`, `android-check`, `android-apk`,
# `web-smoke`) must keep their explicit `if:` capability guard, checked
# separately below, so a gate can never be mistaken for a silent
# skip-to-green.
REQUIRED_JOBS: dict[str, tuple[str, ...]] = {
    "linux-app": (
        "cargo check -p app-linux --all-targets --locked",
        "actions/upload-artifact",
        "linux-app.log",
    ),
    "linux-release": (
        "scripts/package-linux-release.sh",
        "actions/upload-artifact",
    ),
    "core-quality": (
        "cargo fmt --all -- --check",
        "cargo clippy",
        "check-architecture.py",
        "check-fixture-manifest.py",
        "cargo check --workspace --exclude app-android --exclude app-web --all-targets --locked",
    ),
    "windows-check": (
        "cargo check -p app-windows --all-targets --locked",
        "actions/upload-artifact",
    ),
    "windows-release": (
        "scripts/package-windows-release.sh",
        "actions/upload-artifact",
    ),
    "macos-check": (
        "cargo check -p app-macos --all-targets --locked",
        "actions/upload-artifact",
        "macos-check.log",
    ),
    "macos-release": (
        "scripts/package-macos-release.sh",
        "actions/upload-artifact",
    ),
    "wasm-check": (
        "cargo check --workspace --lib --target wasm32-unknown-unknown --locked",
        "cargo check -p app-web --target wasm32-unknown-unknown --locked",
    ),
    "i18n-contracts": (
        "python3 scripts/check-i18n.py",
    ),
    "shader-validation": (
        "cargo test -p cad-render-wgpu --test wgsl_validation --locked",
    ),
    "web-build": (
        "scripts/build-web.sh",
        "wasm-bindgen-cli",
        "actions/upload-artifact",
    ),
    "web-host-contracts": (
        "node --test scripts/test-web-host.mjs",
        "actions/setup-node",
    ),
    "web-deploy": (
        "scripts/deploy-cloudflare-pages.sh",
        "actions/download-artifact",
        "secrets.CLOUDFLARE_API_TOKEN",
    ),
    "android-check": (
        "cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked",
    ),
    "android-apk": (
        "scripts/build-android.sh",
        "dump badging",
        "actions/upload-artifact",
    ),
    "android-release": (
        "scripts/build-android.sh",
        "scripts/fetch-fonts.sh",
        "actions/upload-artifact",
    ),
    "web-smoke": (
        "scripts/check-web-ui.mjs",
        "scripts/serve-web.py",
    ),
}

# Jobs that are capability-gated: they must carry an explicit `if:` guard so
# that when the capability is absent GitHub reports SKIPPED, never a pass.
GATED_JOBS: tuple[str, ...] = (
    "web-deploy",
    "android-check",
    "android-apk",
    "web-smoke",
)

# `if:` fragment each gated job must retain (the capability switch).
GATED_JOB_IF: dict[str, str] = {
    "web-deploy": "vars.CF_PAGES_DEPLOY_ENABLED",
    "android-check": "vars.ANDROID_CI_ENABLED",
    "android-apk": "vars.ANDROID_CI_ENABLED",
    "web-smoke": "vars.WEB_SMOKE_ENABLED",
}

TRUE_CONTINUE_ON_ERROR = re.compile(r"continue-on-error\s*:\s*true\b", re.IGNORECASE)


def workflow_files() -> list[pathlib.Path]:
    if not WORKFLOW_DIR.is_dir():
        return []
    return sorted(
        path
        for path in WORKFLOW_DIR.iterdir()
        if path.is_file() and path.suffix in WORKFLOW_SUFFIXES
    )


def is_true(value: object) -> bool:
    if value is True:
        return True
    return isinstance(value, str) and value.strip().lower() == "true"


def job_block(text: str, key: str) -> str | None:
    """Return the text of a top-level job (2-space key) up to the next job."""
    match = re.search(rf"^  {re.escape(key)}\s*:\s*$", text, re.MULTILINE)
    if match is None:
        return None
    rest = text[match.end():]
    if rest.startswith("\n"):
        rest = rest[1:]
    end = len(rest)
    for nxt in re.finditer(r"^  [A-Za-z0-9_-]+\s*:", rest, re.MULTILINE):
        end = nxt.start()
        break
    return rest[:end]


def structural_pass(
    files: list[pathlib.Path],
) -> tuple[dict[str, list[str]], list[str]]:
    declared: dict[str, list[str]] = {}
    errors: list[str] = []
    for path in files:
        rel = str(path.relative_to(ROOT))
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as exc:
            errors.append(f"{rel}: cannot read ({exc})")
            continue
        if not text.strip():
            errors.append(f"{rel}: workflow file is empty")
            continue
        for key, fragments in REQUIRED_JOBS.items():
            block = job_block(text, key)
            if block is None:
                continue
            declared.setdefault(key, []).append(rel)
            if TRUE_CONTINUE_ON_ERROR.search(block):
                errors.append(
                    f"{rel}: required job '{key}' contains 'continue-on-error: true'"
                )
            if key == "linux-app" and re.search(r"^    if:", block, re.MULTILINE):
                errors.append(f"{rel}: primary linux-app must not be capability-gated")
            for fragment in fragments:
                if fragment not in block:
                    errors.append(
                        f"{rel}: required job '{key}' is missing command fragment "
                        f"{fragment!r}"
                    )
            if key in GATED_JOB_IF and GATED_JOB_IF[key] not in block:
                errors.append(
                    f"{rel}: gated job '{key}' is missing its capability guard "
                    f"{GATED_JOB_IF[key]!r} (a gate must SKIP, never pass silently)"
                )
    return declared, errors


def yaml_pass(
    files: list[pathlib.Path],
) -> tuple[dict[str, list[str]], list[str]]:
    """Real parse when PyYAML is available; returns its own errors."""
    import yaml  # type: ignore  # local import: optional dependency

    declared: dict[str, list[str]] = {}
    errors: list[str] = []
    for path in files:
        rel = str(path.relative_to(ROOT))
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as exc:
            errors.append(f"{rel}: cannot read ({exc})")
            continue
        if not text.strip():
            errors.append(f"{rel}: workflow file is empty")
            continue
        try:
            data = yaml.safe_load(text)
        except yaml.YAMLError as exc:
            errors.append(f"{rel}: YAML parse error: {exc}")
            continue
        jobs = (data or {}).get("jobs")
        if not isinstance(jobs, dict):
            errors.append(f"{rel}: no 'jobs' mapping found")
            continue
        for key in REQUIRED_JOBS:
            job = jobs.get(key)
            if job is None:
                continue
            declared.setdefault(key, []).append(rel)
            if not isinstance(job, dict):
                errors.append(f"{rel}: required job '{key}' is not a mapping")
                continue
            if is_true(job.get("continue-on-error")):
                errors.append(
                    f"{rel}: required job '{key}' sets 'continue-on-error: true'"
                )
            if key in GATED_JOB_IF:
                condition = job.get("if")
                if not isinstance(condition, str) or GATED_JOB_IF[key] not in condition:
                    errors.append(
                        f"{rel}: gated job '{key}' is missing its capability guard "
                        f"{GATED_JOB_IF[key]!r} (a gate must SKIP, never pass silently)"
                    )
            steps = job.get("steps") or []
            if key == "linux-app" and "if" in job:
                errors.append(f"{rel}: primary linux-app must not be capability-gated")
            if not isinstance(steps, list):
                errors.append(f"{rel}: required job '{key}' has non-list 'steps'")
                continue
            for index, step in enumerate(steps, start=1):
                if isinstance(step, dict) and is_true(step.get("continue-on-error")):
                    label = step.get("name") or step.get("uses") or f"step {index}"
                    errors.append(
                        f"{rel}: required job '{key}' step '{label}' sets "
                        "'continue-on-error: true'"
                    )
    return declared, errors


def load_yaml() -> object | None:
    try:
        import yaml  # type: ignore
    except ImportError:
        return None
    return yaml


def main() -> int:
    files = workflow_files()
    if not files:
        print(f"FAIL: no workflow files (*.yml/*.yaml) under {WORKFLOW_DIR}")
        return 1
    for path in files:
        if not path.read_text(encoding="utf-8").strip():
            print(f"FAIL: empty workflow file {path.relative_to(ROOT)}")
            return 1

    yaml_module = load_yaml()
    declared, errors = structural_pass(files)
    if yaml_module is not None:
        mode = "PyYAML parse + structural pass"
        yaml_declared, yaml_errors = yaml_pass(files)
        errors.extend(yaml_errors)
        for key, locations in yaml_declared.items():
            declared.setdefault(key, [])
            for location in locations:
                if location not in declared[key]:
                    declared[key].append(location)
    else:
        mode = "structural text check (PyYAML unavailable)"

    print(f"workflow check mode: {mode}")
    print(f"workflow files: {len(files)}")
    for key in REQUIRED_JOBS:
        locations = declared.get(key, [])
        if locations:
            tag = " (gated)" if key in GATED_JOBS else ""
            print(f"  [ok]   required job '{key}'{tag} declared in {', '.join(sorted(set(locations)))}")
        else:
            print(f"  [FAIL] required job '{key}' is not declared")
            errors.append(f"required job '{key}' is not declared in any workflow")

    if errors:
        print(f"\n{len(errors)} problem(s):")
        for error in errors:
            print(f"  - {error}")
        return 1

    print("\nOK: required jobs present, commands intact, no continue-on-error: true")
    return 0


if __name__ == "__main__":
    sys.exit(main())
