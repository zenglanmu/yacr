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
  * the four release packaging jobs (``linux-release``, ``windows-release``,
    ``macos-release``, ``android-release``) are declared,
    call their real entry point, and keep the ``v*`` tag / manual-dispatch event
    gate so a routine push can never build a release artifact. When credentials
    exist they may **sign** the artifact they build with repository-held
    **self-signed** material (a Windows PFX, an Android keystore, ad-hoc macOS
    ``codesign``, a repository GPG key for the Flatpak OSTree commit); they still
    never publish. The retirement machinery remains
    for future use, but no job is retired today;
  * every **retired** job (none today) is still declared, hard-disabled with an
    unconditional ``if: false`` and a ``RETIRED`` marker, and its only step
    fails non-zero (``exit 1``) if it is ever run. It must NOT carry a mutable
    capability variable (``vars.…``) or a ``secrets.…`` publish path;
  * no required or retired job declares ``continue-on-error: true`` (job-level or
    step-level);
  * the browser pipeline entry points (``scripts/build-web.sh`` plus the shared
    ``scripts/lib/wasm-toolchain.sh``, ``scripts/web-smoke.cjs`` and
    ``scripts/deploy-cloudflare-pages.sh``) are real: the build re-reads the
    ``wasm-bindgen`` pin from ``Cargo.toml`` and compares it with the CLI, runs
    ``cargo build -p app-web --target wasm32-unknown-unknown --profile wasm --locked``
    under the size profile, shrinks the module with pinned ``wasm-opt -Oz`` and
    enforces the 25 MiB Cloudflare Pages ceiling, emits the ``--target web`` glue
    and writes the documented
    ``apps/app-web/dist`` directory it then prints; the smoke waits for the typed
    ``start: ok`` status and labels software rendering; the deploy entry point
    validates every bundle artifact before it hands the directory to wrangler and
    publishes to the project's own ``pages.dev`` hostname (it must not manage a
    custom domain). The ``web-build`` / ``web-smoke`` / ``web-deploy`` jobs are
    required jobs, and ``web-deploy`` must stay gated to ``v*`` tags or a manual
    dispatch.

Each guarantee is enforced structurally (and, when PyYAML is importable, with a
real parse as well) so a mutation that removes a fragment or a disable guard is
reported rather than silently accepted. The script is a guard rail, not a
replacement for a YAML parser or for actually running the workflow.
"""
from __future__ import annotations

import os
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
        # Tauri v2 denies core-plugin commands without a capability file; the
        # chrome only needs the `cad_event` listen/unlisten pair.
        "python3 scripts/check-tauri-capabilities.py",
        "check-fixture-manifest.py",
        "check-file-size.py",
        # The committed CJK fallback's bytes/catalogue entry/OFL text.
        "python3 scripts/check-font-assets.py",
        # Both Web bundles stage the same committed package and the JS preload
        # runs before the wasm session starts.
        "python3 scripts/test-web-font-package.py",
        # The command/alias docs gate: §2/§2b (docs/shortcuts.md) and the
        # 测量选择子 section (docs/autocad-2026-command-coverage.md) are re-checked
        # against the cad-ui-core tables, and its self-test proves the check can fail.
        "python3 scripts/check-command-docs.py",
        "python3 scripts/test-check-command-docs.py",
        "cargo check --workspace --all-targets --locked",
        # core-quality also compiles app-linux (clippy + workspace check), so it
        # needs the same Tauri/GTK prerequisites and the frontend dist — and it
        # runs the React tests and the retained platform-neutral node contract.
        "libwebkit2gtk-4.1-dev",
        "apps/desktop-ui",
        "npm test",
        "npm run build",
        "node --test scripts/test-web-pixels.mjs",
        # The Android host's static wiring contract (layout, Android-only `tauri`,
        # the two shell-only commands, the webgl2 refusal, the UI bundle path).
        "python3 scripts/test-android-host.py",
        "test-ui-migration-guards.py",
        "test-release-workflows.py",
        # The static landing page + manual R2/Pages deployment contract.
        "python3 scripts/test-landing-site.py",
    ),
    "windows-check": (
        "cargo check -p app-windows --all-targets --locked",
        "actions/upload-artifact",
    ),
    "macos-check": (
        "cargo check -p app-macos --all-targets --locked",
        "actions/setup-node@v4",
        "apps/desktop-ui",
        "npm run build",
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
        # The **real** Android host (a Tauri 2 WebView carrier around the wasm
        # `dom-canvas` session) is compiled for an Android target. `tauri::
        # generate_context!` embeds `frontendDist`, so the Android UI bundle must be
        # built first; dropping either half must be reported, not silently accepted.
        "apps/desktop-ui",
        "scripts/build-android-ui.sh",
        "cargo check --target aarch64-linux-android -p app-android --locked",
    ),
    # Release packaging jobs (re-enabled 2026-11-01): event-gated artifact
    # builders for the current React/Tauri hosts. They never run on a routine
    # push/PR and never publish; when credentials exist they sign the artifact
    # they build with repository self-signed material (Windows PFX / Android
    # Windows falls back to a `*-unsigned` artifact when its PFX is absent; the
    # Android release job fails closed without a keystore.
    "linux-release": (
        "refs/tags/v",
        "workflow_dispatch",
        "scripts/package-linux-release.sh",
        "flatpak-builder",
        "actions/upload-artifact",
        "*.flatpak",
        # Optional OSTree/GPG commit signing.
        "FLATPAK_GPG_KEY_ID",
        "--gpg-sign",
        ".flatpak.gpg",
    ),
    "windows-release": (
        "refs/tags/v",
        "workflow_dispatch",
        "scripts/package-windows-release.sh",
        "actions/setup-node@v4",
        "actions/upload-artifact",
        "*.zip",
        # Repository-PFX Authenticode signing of the staged executables, then
        # local verification and the shared zip/checksum finalizer.
        "WINDOWS_SELF_SIGNED_PFX_BASE64",
        "Import-PfxCertificate",
        "signtool",
        "yacr-dev-codesign.cer",
        "finalize-windows-release.sh",
        "Get-AuthenticodeSignature",
    ),
    "macos-release": (
        "refs/tags/v",
        "workflow_dispatch",
        "scripts/package-macos-release.sh",
        "actions/setup-node@v4",
        "aarch64-apple-darwin",
        "actions/upload-artifact",
        "*.tar.gz",
        # Ad-hoc codesign (or an explicit identity), verified after packaging.
        "MACOS_SIGN_IDENTITY",
        "codesign --verify --strict",
    ),
    "android-release": (
        "refs/tags/v",
        "workflow_dispatch",
        "scripts/build-android.sh",
        "--release",
        "ndk;29.0.13846066",
        "actions/upload-artifact",
        "*.apk",
        # Aligned APK signed with the repository keystore, then verified locally.
        "ANDROID_KEYSTORE_BASE64",
        "Fail if the release keystore is not configured",
        "apksigner",
        "SIGNING.txt",
        "app-universal-release.apk",
    ),
    "web-build": (
        # The real wasm CAD bundle (ADR 0024): pinned CLI install, script build,
        # artifact upload that web-smoke and web-deploy consume. The same job also
        # builds the shared React chrome bundle (`web-ui-dist`) that web-deploy
        # publishes: one wasm build, two bundles, no cross-use.
        "bash scripts/build-web.sh",
        "wasm32-unknown-unknown",
        "wasm-bindgen",
        "apps/app-web/dist",
        "actions/upload-artifact",
        "web-dist",
        "npm ci",
        "bash scripts/build-web-ui.sh",
        "apps/app-web/dist-ui",
        "web-ui-dist",
    ),
    "web-smoke": (
        # The bundle this run built must be served and driven in a browser; CI has
        # no GPU, so the job name and the smoke log both label software rendering.
        "actions/download-artifact",
        "web-dist",
        "scripts/serve-web.py",
        "scripts/web-smoke.cjs",
        "playwright",
    ),
    "web-deploy": (
        # Publishes the uploaded chrome artifact only; the deploy script validates
        # every bundle file (`BUNDLE_KIND=ui`) before wrangler sees it.
        "actions/download-artifact",
        "web-ui-dist",
        "apps/app-web/dist-ui",
        "BUNDLE_KIND",
        "scripts/deploy-cloudflare-pages.sh",
        "CLOUDFLARE_API_TOKEN",
        "SKIP_BUILD",
    ),
}

# Jobs that are capability- or event-gated: they must carry an explicit `if:`
# guard so that when the condition is absent GitHub reports SKIPPED, never a pass.
# `android-check` is capability-gated (no Android device/toolchain by default) and
# `web-deploy` is event-gated (only `v*` tags or a manual dispatch may publish).
GATED_JOBS: tuple[str, ...] = (
    "android-check",
    "web-deploy",
    "linux-release",
    "windows-release",
    "macos-release",
    "android-release",
)

# `if:` fragment each gated job must retain (the capability/event switch).
GATED_JOB_IF: dict[str, str] = {
    "android-check": "vars.ANDROID_CI_ENABLED",
    "web-deploy": "refs/tags/v",
    "linux-release": "refs/tags/v",
    "windows-release": "refs/tags/v",
    "macos-release": "refs/tags/v",
    "android-release": "refs/tags/v",
}

# No platform release job is retired any more (2026-11-01): the four release
# jobs are real artifact builders gated to a `v*` tag or a manual dispatch. The
# retirement machinery stays in the checker so a future backslide into an
# `if: false` stub is still caught; the tuple is intentionally empty.
RETIRED_JOBS: tuple[str, ...] = ()

RETIRED_MARKER = "RETIRED"
HARD_DISABLE_RE = re.compile(r"^    if:\s*false\s*(?:#.*)?$", re.MULTILINE)
MUTABLE_GATE_RE = re.compile(r"\bvars\.")
# A retired job must not reference repository credentials: a disabled publish job
# has no reason to touch a secret, and a stray `secrets.…` is the signature of a
# publish path that was not really retired.
SECRETS_RE = re.compile(r"\bsecrets\.")
TRUE_CONTINUE_ON_ERROR = re.compile(r"continue-on-error\s*:\s*true\b", re.IGNORECASE)

# --- browser bundle, smoke and deploy entry points ---------------------------
#
# The wasm CAD session in `apps/app-web` was revived (ADR 0024), so
# `scripts/build-web.sh` builds a real bundle, `scripts/web-smoke.cjs` drives it in
# a browser, and `scripts/deploy-cloudflare-pages.sh` publishes exactly the
# validated bundle. What this checker pins is the **script-level** contract so a
# silent regression back to a stub, a dropped wasm-bindgen pin check, a smoke that
# never asserts the status line, or a deploy that publishes an unvalidated
# directory fails.

BUILD_WEB_SCRIPT = "scripts/build-web.sh"
WASM_TOOLCHAIN_SCRIPT = "scripts/lib/wasm-toolchain.sh"
BUILD_WEB_OUT_DIR = "apps/app-web/dist"
WEB_SMOKE_SCRIPT = "scripts/web-smoke.cjs"
DEPLOY_SCRIPT = "scripts/deploy-cloudflare-pages.sh"

# Fragments `build-web.sh` must keep. The pin check and the cargo/wasm-bindgen
# invocations live in the shared toolchain library (both wasm entry points use
# them), so this script has to source it and call its steps in the documented
# order rather than carry a second copy.
BUILD_WEB_FRAGMENTS: tuple[str, ...] = (
    "set -euo pipefail",
    WASM_TOOLCHAIN_SCRIPT,
    "yacr_init_wasm_toolchain",
    "yacr_build_app_web_wasm",
    "yacr_bindgen_web",
    BUILD_WEB_OUT_DIR,
)

# Fragments the shared toolchain library must keep: the exact wasm-bindgen and
# wasm-opt pin comparisons, the wasm32 size-profile build, the `--target web`
# post-processing and the Cloudflare Pages per-file ceiling.
WASM_TOOLCHAIN_FRAGMENTS: tuple[str, ...] = (
    "wasm-bindgen",
    "Cargo.toml",
    "--version",
    "wasm32-unknown-unknown",
    "build -p app-web",
    "--profile",
    "--locked",
    "wasm-opt",
    "-Oz",
    "26214400",
    "--target web",
    "--no-typescript",
    "app_web_bg.wasm",
)

# The declared bundle directory, e.g. `readonly OUT_DIR="$REPO_ROOT/apps/app-web/dist"`.
BUILD_WEB_OUT_DIR_RE = re.compile(
    r'^readonly OUT_DIR="\$REPO_ROOT/(?P<dir>[^"]+)"\s*$', re.MULTILINE
)
# ... and the run report must print that same variable, not a hard-coded copy.
BUILD_WEB_REPORT_RE = re.compile(r"""printf 'artifacts in %s\\n' "\$OUT_DIR\"""")

# Fragments the smoke must keep: it has to load the built bundle, wait for the
# typed `start: ok` status, read the document status through the page handle, and
# label software rendering instead of implying GPU evidence.
WEB_SMOKE_FRAGMENTS: tuple[str, ...] = (
    "playwright",
    "#cad-status",
    "start: ok",
    "__YACR_CANVAS__",
    "synthetic-four-lines.dwg",
    "webgl2",
    "software",
    "process.exitCode",
)

# Fragments the deploy entry point must keep. The artifact list is spelled out so
# a missing/partial bundle is refused before wrangler runs; the credential names
# and the wrangler invocation are the publish contract itself. The reported URL
# is the project's own `pages.dev` hostname: the deploy entry point must not grow
# a custom-domain attach/detach path again.
DEPLOY_FRAGMENTS: tuple[str, ...] = (
    "set -euo pipefail",
    "CLOUDFLARE_API_TOKEN",
    "CLOUDFLARE_ACCOUNT_ID",
    "SKIP_BUILD",
    # Fonts are not in either bundle: the deploy assembles the full CDN package
    # at the site root and the page loads it lazily after a document opens.
    "SKIP_FONTS",
    "scripts/fetch-fonts.sh",
    "BUNDLE_KIND",
    "index.html",
    "main.js",
    "worker.js",
    "style.css",
    "pkg/app_web.js",
    "pkg/app_web_bg.wasm",
    "canvas/app_web.js",
    "canvas/app_web_bg.wasm",
    "canvas/bootstrap.js",
    # The Tianzheng vendor plugin is part of the shared chrome bundle; a site
    # without it would silently lose vendor-derived geometry.
    "plugins/tch.js",
    BUILD_WEB_OUT_DIR,
    "wrangler",
    "pages deploy",
    "$PROJECT.pages.dev",
)

# A custom domain is Cloudflare-side configuration. The repository deployment
# path is the project's `pages.dev` hostname only, so reintroducing a domain
# attach switch (or its API call) is a contract error rather than a nice-to-have.
CUSTOM_DOMAIN_RE = re.compile(r"CF_PAGES_DOMAIN|/projects/\$PROJECT/domains")


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


def build_web_errors(text: str) -> list[str]:
    """Contract errors for the local bundle script (`scripts/build-web.sh`)."""
    errors: list[str] = []
    if not text.strip():
        return [f"{BUILD_WEB_SCRIPT}: entry point is empty"]
    if "NOT IMPLEMENTED" in text:
        errors.append(
            f"{BUILD_WEB_SCRIPT}: is a retired stub again ('NOT IMPLEMENTED'); the "
            "local browser bundle build must stay runnable"
        )
    for fragment in BUILD_WEB_FRAGMENTS:
        if fragment not in text:
            errors.append(
                f"{BUILD_WEB_SCRIPT}: missing required fragment {fragment!r}"
            )
    declared = BUILD_WEB_OUT_DIR_RE.search(text)
    if declared is None:
        errors.append(
            f"{BUILD_WEB_SCRIPT}: no `readonly OUT_DIR=\"$REPO_ROOT/{BUILD_WEB_OUT_DIR}\"` "
            "declaration for the bundle directory"
        )
    elif declared.group("dir") != BUILD_WEB_OUT_DIR:
        errors.append(
            f"{BUILD_WEB_SCRIPT}: writes {declared.group('dir')!r} but the documented "
            f"bundle directory is {BUILD_WEB_OUT_DIR!r}"
        )
    if BUILD_WEB_REPORT_RE.search(text) is None:
        errors.append(
            f"{BUILD_WEB_SCRIPT}: does not print the directory it wrote (expected "
            "`printf 'artifacts in %s\\n' \"$OUT_DIR\"`)"
        )
    return errors


def wasm_toolchain_errors(text: str) -> list[str]:
    """Contract errors for the shared wasm toolchain (`scripts/lib/...`)."""
    errors: list[str] = []
    if not text.strip():
        return [f"{WASM_TOOLCHAIN_SCRIPT}: is empty"]
    for fragment in WASM_TOOLCHAIN_FRAGMENTS:
        if fragment not in text:
            errors.append(
                f"{WASM_TOOLCHAIN_SCRIPT}: missing required fragment {fragment!r} "
                "(wasm-bindgen pin check / wasm32 build / web glue)"
            )
    return errors


def web_smoke_errors(text: str) -> list[str]:
    """Contract errors for the headless browser smoke (`scripts/web-smoke.cjs`)."""
    errors: list[str] = []
    if not text.strip():
        return [f"{WEB_SMOKE_SCRIPT}: is empty"]
    if "NOT IMPLEMENTED" in text:
        errors.append(
            f"{WEB_SMOKE_SCRIPT}: claims NOT IMPLEMENTED; the smoke must drive the bundle"
        )
    for fragment in WEB_SMOKE_FRAGMENTS:
        if fragment not in text:
            errors.append(f"{WEB_SMOKE_SCRIPT}: missing required fragment {fragment!r}")
    if "process.exit(0)" in text:
        errors.append(
            f"{WEB_SMOKE_SCRIPT}: exits 0 unconditionally; a failing smoke must fail the job"
        )
    return errors


def deploy_script_errors(text: str) -> list[str]:
    """Contract errors for the Cloudflare Pages publish entry point."""
    errors: list[str] = []
    if not text.strip():
        return [f"{DEPLOY_SCRIPT}: is empty"]
    if "NOT IMPLEMENTED" in text:
        errors.append(
            f"{DEPLOY_SCRIPT}: is a retired stub again ('NOT IMPLEMENTED'); the publish "
            "entry point must stay runnable"
        )
    for fragment in DEPLOY_FRAGMENTS:
        if fragment not in text:
            errors.append(f"{DEPLOY_SCRIPT}: missing required fragment {fragment!r}")
    if CUSTOM_DOMAIN_RE.search(text):
        errors.append(
            f"{DEPLOY_SCRIPT}: manages a custom domain; the deployment target is "
            "the project's pages.dev hostname only"
        )
    # The artifact guard must run before the upload: a `pages deploy` invocation
    # that appears before any `[ -s "$DIST/…" ]` check would publish a partial
    # bundle.
    guard = text.find('[ -s "$DIST/')
    upload = text.find("pages deploy")
    if guard == -1:
        errors.append(
            f"{DEPLOY_SCRIPT}: no `[ -s \"$DIST/…\" ]` artifact guard; a partial bundle "
            "could be published"
        )
    elif upload != -1 and guard > upload:
        errors.append(
            f"{DEPLOY_SCRIPT}: the artifact guard runs after the pages upload"
        )
    return errors


def _read_script(rel: str, require_exec: bool = True) -> tuple[str | None, list[str]]:
    path = ROOT / rel
    if not path.is_file():
        return None, [f"{rel}: missing"]
    problems: list[str] = []
    if not os.access(path, os.R_OK):
        problems.append(f"{rel}: is not readable")
    if require_exec and not os.access(path, os.X_OK):
        problems.append(f"{rel}: is not executable (chmod +x required)")
    try:
        return path.read_text(encoding="utf-8"), problems
    except OSError as exc:
        return None, problems + [f"{rel}: cannot read ({exc})"]


def entry_point_pass() -> tuple[list[str], list[str]]:
    """Check the bundle build, the browser smoke and the publish entry point."""
    errors: list[str] = []
    ok: list[str] = []

    text, problems = _read_script(BUILD_WEB_SCRIPT)
    errors.extend(problems)
    if text is not None:
        found = build_web_errors(text)
        errors.extend(found)
        if not found and not problems:
            ok.append(
                f"local entry point '{BUILD_WEB_SCRIPT}' builds the browser bundle "
                f"into {BUILD_WEB_OUT_DIR}"
            )

    text, problems = _read_script(WASM_TOOLCHAIN_SCRIPT, require_exec=False)
    errors.extend(problems)
    if text is not None:
        found = wasm_toolchain_errors(text)
        errors.extend(found)
        if not found and not problems:
            ok.append(
                f"shared wasm toolchain '{WASM_TOOLCHAIN_SCRIPT}' keeps the wasm-bindgen "
                "pin check and the wasm32 size-profile build"
            )

    for rel, contract, label in (
        (WEB_SMOKE_SCRIPT, web_smoke_errors, "drives the built bundle in a browser"),
        (DEPLOY_SCRIPT, deploy_script_errors, "publishes only a validated bundle"),
    ):
        text, problems = _read_script(rel)
        errors.extend(problems)
        if text is None:
            continue
        found = contract(text)
        errors.extend(found)
        if not found and not problems:
            ok.append(f"entry point '{rel}' {label}")

    return ok, errors


def submodule_checkout_errors(files: list[pathlib.Path]) -> list[str]:
    """Every checkout must fetch the `plugins/tch` submodule.

    The Tianzheng decoder lives in its own repository and is mounted at
    ``plugins/tch``. A job that checks out the superproject without submodules
    compiles/tests against an empty directory (or, worse, a stale build output),
    so every ``actions/checkout`` step must set ``submodules``.
    """
    errors: list[str] = []
    for path in files:
        text = path.read_text(encoding="utf-8")
        lines = text.splitlines()
        for index, line in enumerate(lines):
            if "uses: actions/checkout@" not in line:
                continue
            block = "\n".join(lines[index : index + 6])
            if not re.search(r"^\s+submodules:\s*(?:recursive|true)\s*$", block, re.MULTILINE):
                errors.append(
                    f"{path.relative_to(ROOT)}:{index + 1}: actions/checkout does not "
                    "set submodules; plugins/tch would be empty"
                )
    return errors


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
    errors.extend(submodule_checkout_errors(files))
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

    entry_ok, entry_errors = entry_point_pass()
    errors.extend(entry_errors)
    for line in entry_ok:
        print(f"  [ok]   {line}")
    # Entry-point failures are not workflow job failures, but they are still hard
    # errors: a silent stub or a drifted shared toolchain breaks the documented
    # local build.
    for error in entry_errors:
        print(f"  [FAIL] {error}")

    if errors:
        print(f"\n{len(errors)} problem(s):")
        for error in errors:
            print(f"  - {error}")
        return 1

    retired_note = (
        "retired jobs hard-disabled"
        if RETIRED_JOBS
        else "no retired jobs (release jobs are event-gated artifact builders)"
    )
    print(
        f"\nOK: required jobs present with commands intact, {retired_note}, "
        "browser build/smoke/deploy entry points real (deploy tag/dispatch gated), "
        "no continue-on-error: true"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
