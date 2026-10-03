#!/usr/bin/env python3
"""Summarize a `scripts/verify-ui.sh` run into an honest, layered evidence package.

Reads the per-layer status TSV written by the shell entry, scans the logs for
panics, validates that screenshots are real PNGs, and writes `verify-ui.json`,
`environment.json` and `panic.txt`. It never upgrades a failed/absent layer into
a pass, and it always records that a real GPU / real device was not exercised.
"""
import json
import os
import pathlib
import platform
import re
import shutil
import subprocess
import sys

REQUIRED = ["ui-unit", "ui-offscreen", "scenario", "host-contracts", "app-build", "app-smoke"]


def versions() -> dict:
    out = {}
    for tool in ("rustc", "cargo"):
        exe = shutil.which(tool)
        if not exe:
            out[tool] = None
            continue
        try:
            out[tool] = subprocess.run(
                [exe, "--version"], capture_output=True, text=True, timeout=30
            ).stdout.strip()
        except Exception as error:  # pragma: no cover - environment dependent
            out[tool] = f"unavailable: {error}"
    return out


def read_layers(status_file: pathlib.Path) -> list:
    layers = []
    if not status_file.exists():
        return layers
    for line in status_file.read_text().splitlines():
        parts = line.split("\t")
        if len(parts) < 4:
            continue
        name, status, code, elapsed = parts[0], parts[1], parts[2], parts[3]
        log = parts[4] if len(parts) >= 5 else ""
        layers.append(
            {
                "name": name,
                "status": status,
                "exit": int(code),
                "elapsedSeconds": int(elapsed),
                "log": pathlib.Path(log).name if log else None,
            }
        )
    return layers


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: verify-ui-summary.py OUTPUT_DIR FAILED_FLAG", file=sys.stderr)
        return 2
    output = pathlib.Path(sys.argv[1])
    failed = sys.argv[2] != "0"
    layers = read_layers(output / ".layers.tsv")
    by_name = {layer["name"]: layer for layer in layers}

    # Panic scan: a panic anywhere is a real failure, even if a wrapper exited 0.
    panic_pattern = re.compile(r"panicked at|thread '[^']*' panicked")
    panic_lines: list = []
    for log in sorted((output / "logs").glob("*.log")):
        for line in log.read_text(errors="replace").splitlines():
            if panic_pattern.search(line):
                panic_lines.append(f"{log.name}: {line}")
    if panic_lines:
        (output / "panic.txt").write_text("\n".join(panic_lines) + "\n")

    # Screenshot sanity: real PNG magic, non-empty.
    screenshots = sorted((output / "screenshots").glob("*.png"))
    bad_screenshots = [
        p.name
        for p in screenshots
        if p.stat().st_size < 64 or not p.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
    ]

    missing_required = [
        name
        for name in REQUIRED
        if by_name.get(name, {}).get("status") != "passed"
    ]
    environment = {
        "schemaVersion": 1,
        "host": "app-linux",
        "entry": "verify-ui",
        "virtualDisplay": "slint-offscreen-platform",
        "displayServer": {
            "DISPLAY": None,
            "WAYLAND_DISPLAY": None,
            "xvfbAvailable": shutil.which("Xvfb") is not None,
            "xWaylandAvailable": shutil.which("Xwayland") is not None,
        },
        "softwareVulkanIcd": None,
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
        },
        "versions": versions(),
        "realDevice": "not-run",
    }
    environment["displayServer"]["DISPLAY"] = os.environ.get("DISPLAY") or None
    environment["displayServer"]["WAYLAND_DISPLAY"] = os.environ.get("WAYLAND_DISPLAY") or None
    environment["softwareVulkanIcd"] = os.environ.get("VK_ICD_FILENAMES") or None
    app_report = output / "app" / "report.json"
    if app_report.exists():
        try:
            report = json.loads(app_report.read_text())
            environment["adapterBackend"] = report.get("backend")
            environment["softwareGpu"] = report.get("softwareGpu")
            environment["cadFrames"] = report.get("cadFrames")
        except json.JSONDecodeError:
            environment["adapterBackend"] = "unparseable app report"
    (output / "environment.json").write_text(json.dumps(environment, indent=2) + "\n")

    status = "failed" if (failed or missing_required or panic_lines or bad_screenshots) else "passed"
    summary = {
        "schemaVersion": 1,
        "host": "app-linux",
        "entry": "verify-ui",
        "status": status,
        "layers": {
            "core": "cargo test cad-ui-slint --lib",
            "virtualGraphics": "slint-offscreen + lavapipe (software Vulkan)",
            "realDevice": "not-run",
        },
        "missingOrFailedRequiredLayers": missing_required,
        "panics": panic_lines,
        "screenshotCount": len(screenshots),
        "badScreenshots": bad_screenshots,
        "steps": layers,
        "note": (
            "Software Vulkan on the Slint offscreen platform. This is not a real "
            "GPU, window server, real drawing, or device result."
        ),
    }
    (output / "verify-ui.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({k: summary[k] for k in (
        "status", "missingOrFailedRequiredLayers", "screenshotCount")}))
    if status != "passed":
        for layer in layers:
            if layer["status"] != "passed" and layer["status"] != "skipped":
                print(f"  FAILED {layer['name']}: {layer['status']} (see {layer['log']})")
        for name in missing_required:
            print(f"  MISSING required layer: {name}")
        for bad in bad_screenshots:
            print(f"  BAD screenshot: {bad}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
