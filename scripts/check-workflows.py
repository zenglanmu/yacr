#!/usr/bin/env python3
"""Validate the layered GitHub Actions CI workflows.

Standard library only (PyYAML is used only as an optional extra parse). What it
checks:

  * at least one workflow file exists under ``.github/workflows`` and is
    non-empty;
  * every **required** job is declared as a job key in some workflow and still
    contains its expected command fragment, so the job keeps mirroring the real
    gate instead of drifting into an empty success. The required jobs are the
    genuine compile/check gates that survive the UI replacement:
    ``core-quality``, ``wasm-check``, ``i18n-contracts``, ``shader-validation``,
    ``linux-app``, ``windows-check``, ``macos-check``, ``android-check``;
  * every **retired** platform build/deploy/release job
    (``web-build``, ``web-deploy``, ``web-smoke``, ``android-apk``,
    ``android-release``, ``linux-release``, ``windows-release``,
    ``macos-release``) is still *declared* so its absence stays visible, but is
    hard-disabled with an unconditional ``if: false`` and a ``RETIRED`` marker,
    and its only step fails non-zero (``exit 1``) if it is ever run. It must NOT
    carry a mutable capability variable (``vars.…``): a repository variable that
    could accidentally re-enable a stub publish is exactly the foot-gun this
    check removes. The one still capability-gated job is the ``android-check``
    compile gate, which cannot publish anything;
  * no required or retired job declares ``continue-on-error: true`` (job-level or
    step-level).

Each guarantee is enforced structurally (and, when PyYAML is importable, with a
real parse as well) so a mutation that removes a fragment or a disable guard is
reported rather than silently accepted. The script is a guard rail, not a
replacement for a YAML parser or for actually running the workflow.
"""
from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKFLOW_DIR = ROOT / ".github" / "workflows"
WORKFLOW_SUFFIXES = (".yml", ".yaml")

# Required (genuine) job key -> command fragments that must appear inside it.
REQUIRED_JOBS: dict[str, tuple[str, ...]] = {
    "linux-app": (
        "cargo check -p app-linux --all-targets --locked",
        # Tauri/GTK native prerequisites plus the React test + build the native
        # gate must run before `tauri::generate_context!` embeds `ui/dist`.
        "libwebkit2gtk-4.1-dev",
        "apps/desktop-ui",
        "npm test",
        "npm run build",
        "actions/upload-artifact",
        "linux-app.log",
    ),
    "core-quality": (
        "cargo fmt --all -- --check",
        "cargo clippy",
        "check-architecture.py",
        "check-fixture-manifest.py",
        "check-file-size.py",
        "cargo check --workspace --all-targets --locked",
        # core-quality also compiles app-linux (clippy + workspace check), so it
        # needs the same Tauri/GTK prerequisites and the frontend dist — and it
        # runs the React tests and the retained platform-neutral node contract.
        "libwebkit2gtk-4.1-dev",
        "apps/desktop-ui",
        "npm test",
        "npm run build",
        "node --test scripts/test-web-pixels.mjs",
        "test-ui-migration-guards.py",
    ),
    "windows-check": (
        "cargo check -p app-windows --all-targets --locked",
        "actions/upload-artifact",
    ),
    "macos-check": (
        "cargo check -p app-macos --all-targets --locked",
        "actions/upload-artifact",
        "macos-check.log",
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
    "android-check": (
        # The retired Slint UI is dropped from the Android compile gate; only the
        # explicit NOT IMPLEMENTED `app-android` placeholder is compiled.
        "cargo check --target aarch64-linux-android -p app-android --locked",
    ),
}

# Jobs that are capability-gated: they must carry an explicit `if:` guard so
# that when the capability is absent GitHub reports SKIPPED, never a pass. Only
# the `android-check` compile gate remains gated (it cannot publish anything).
GATED_JOBS: tuple[str, ...] = ("android-check",)

# `if:` fragment each gated job must retain (the capability switch).
GATED_JOB_IF: dict[str, str] = {
    "android-check": "vars.ANDROID_CI_ENABLED",
}

# Retired platform build/deploy/release jobs. They are kept declared (so the
# gap stays visible) but must be hard-disabled: an unconditional `if: false` and
# a `RETIRED` marker, never a mutable `vars.…` switch.
RETIRED_JOBS: tuple[str, ...] = (
    "web-build",
    "web-deploy",
    "web-smoke",
    "android-apk",
    "android-release",
    "linux-release",
    "windows-release",
    "macos-release",
)

RETIRED_MARKER = "RETIRED"
HARD_DISABLE_RE = re.compile(r"^    if:\s*false\s*(?:#.*)?$", re.MULTILINE)
MUTABLE_GATE_RE = re.compile(r"\bvars\.")
# A retired job must not reference repository credentials: a disabled publish job
# has no reason to touch a secret, and a stray `secrets.…` is the signature of a
# publish path that was not really retired.
SECRETS_RE = re.compile(r"\bsecrets\.")
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
) -> tuple[dict[str, list[str]], dict[str, list[str]], list[str]]:
    declared: dict[str, list[str]] = {}
    retired_declared: dict[str, list[str]] = {}
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
        for key in RETIRED_JOBS:
            block = job_block(text, key)
            if block is None:
                continue
            retired_declared.setdefault(key, []).append(rel)
            if not HARD_DISABLE_RE.search(block):
                errors.append(
                    f"{rel}: retired job '{key}' must be hard-disabled with an "
                    "unconditional 'if: false'"
                )
            if RETIRED_MARKER not in block:
                errors.append(
                    f"{rel}: retired job '{key}' is missing the 'RETIRED' marker "
                    "that documents its status"
                )
            # Belt-and-braces: even if the disable is ever removed, the retired
            # job's only step must fail non-zero, never pass as an empty success.
            if "exit 1" not in block:
                errors.append(
                    f"{rel}: retired job '{key}' must fail non-zero ('exit 1') if "
                    "it is ever run"
                )
            if MUTABLE_GATE_RE.search(block):
                errors.append(
                    f"{rel}: retired job '{key}' must not carry a mutable 'vars.…' "
                    "capability switch (it could re-enable a stub publish)"
                )
            if SECRETS_RE.search(block):
                errors.append(
                    f"{rel}: retired job '{key}' must not reference repository "
                    "secrets (a publish path was not fully retired)"
                )
            if TRUE_CONTINUE_ON_ERROR.search(block):
                errors.append(
                    f"{rel}: retired job '{key}' contains 'continue-on-error: true'"
                )
    return declared, retired_declared, errors


def yaml_pass(
    files: list[pathlib.Path],
) -> tuple[dict[str, list[str]], dict[str, list[str]], list[str]]:
    """Real parse when PyYAML is available; returns its own errors."""
    import yaml  # type: ignore  # local import: optional dependency

    declared: dict[str, list[str]] = {}
    retired_declared: dict[str, list[str]] = {}
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
        for key in RETIRED_JOBS:
            job = jobs.get(key)
            if job is None:
                continue
            retired_declared.setdefault(key, []).append(rel)
            if not isinstance(job, dict):
                errors.append(f"{rel}: retired job '{key}' is not a mapping")
                continue
            # `if: false` parses to the boolean False; any string condition (in
            # particular a `vars.…` switch) is a failure.
            if job.get("if") is not False:
                errors.append(
                    f"{rel}: retired job '{key}' must be hard-disabled with "
                    "'if: false' (got " + repr(job.get("if")) + ")"
                )
            if is_true(job.get("continue-on-error")):
                errors.append(
                    f"{rel}: retired job '{key}' sets 'continue-on-error: true'"
                )
    return declared, retired_declared, errors


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
    declared, retired_declared, errors = structural_pass(files)
    if yaml_module is not None:
        mode = "PyYAML parse + structural pass"
        yaml_declared, yaml_retired, yaml_errors = yaml_pass(files)
        errors.extend(yaml_errors)
        for bucket, extra in ((declared, yaml_declared), (retired_declared, yaml_retired)):
            for key, locations in extra.items():
                bucket.setdefault(key, [])
                for location in locations:
                    if location not in bucket[key]:
                        bucket[key].append(location)
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
    for key in RETIRED_JOBS:
        locations = retired_declared.get(key, [])
        if locations:
            print(
                f"  [ok]   retired job '{key}' declared (hard-disabled) in "
                f"{', '.join(sorted(set(locations)))}"
            )
        else:
            print(f"  [FAIL] retired job '{key}' is not declared")
            errors.append(
                f"retired job '{key}' is not declared in any workflow "
                "(a silent removal hides the capability gap)"
            )

    if errors:
        print(f"\n{len(errors)} problem(s):")
        for error in errors:
            print(f"  - {error}")
        return 1

    print(
        "\nOK: required jobs present with commands intact, retired jobs hard-disabled, "
        "no continue-on-error: true"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
