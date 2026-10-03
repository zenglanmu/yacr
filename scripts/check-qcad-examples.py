#!/usr/bin/env python3
"""Smoke/coverage harness for the committed QCAD example DXF corpus.

For every `fixtures/dxf/qcad-examples/*.dxf` it runs the real CLI through
`scan`, `proxy-report`, `build-representation` and (unless `--no-render`)
`render`, then records the entity-type inventory, completeness reasons, adapter
and non-empty frame. The corpus files have no upstream PNG except the flange
sample; with `--export-references` and the optional Python `ezdxf` +
`matplotlib` packages it exports a reference PNG per drawing into the output
directory for human/coarse comparison.

Stdlib only for its own work; ezdxf is an optional test-time tool and is
reported NOT RUN when absent. Non-zero exit on any hard failure. This is a
parse/render smoke, not a compatibility or fidelity claim.

Usage:
  python3 scripts/check-qcad-examples.py --out /tmp/opencode/yacr-qcad-corpus \
      --font txt=/home/me/fonts/txt.shx --no-render
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import time
from pathlib import Path

DEFAULT_ICD = "/usr/share/vulkan/icd.d/lvp_icd.json"


def run_json(cli: str, operation: str, sample: Path, env: dict, extra: list) -> dict:
    proc = subprocess.run(
        [cli, operation, str(sample), *extra],
        env=env,
        capture_output=True,
        text=True,
        timeout=900,
    )
    if proc.returncode:
        raise RuntimeError(f"{operation} exited {proc.returncode}: {proc.stderr.strip()[:500]}")
    return json.loads(proc.stdout)


def export_reference(sample: Path, out_png: Path) -> str:
    """Return 'ok', or a NOT RUN reason when ezdxf/matplotlib are unavailable."""
    try:
        import matplotlib

        matplotlib.use("Agg")
        import matplotlib.pyplot as plt  # noqa: PLC0415
        import ezdxf  # noqa: PLC0415
        from ezdxf.addons.drawing import Frontend, RenderContext  # noqa: PLC0415
        from ezdxf.addons.drawing.matplotlib import MatplotlibBackend  # noqa: PLC0415
    except Exception as error:  # noqa: BLE001 - optional test dependency
        return f"NOT RUN: ezdxf/matplotlib unavailable ({error})"
    doc = ezdxf.readfile(sample)
    figure = plt.figure()
    axis = figure.add_axes((0, 0, 1, 1))
    axis.set_axis_off()
    backend = MatplotlibBackend(axis)
    Frontend(RenderContext(doc), backend).draw_layout(doc.modelspace(), finalize=True)
    figure.savefig(out_png, dpi=150, facecolor="white")
    plt.close(figure)
    return "ok"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--corpus", type=Path, default=Path("fixtures/dxf/qcad-examples"))
    parser.add_argument("--out", type=Path, default=Path("/tmp/opencode/yacr-qcad-corpus"))
    parser.add_argument("--cli", default="target/release/cad-cli-tools")
    parser.add_argument("--font", action="append", default=[])
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=960)
    parser.add_argument("--icd", default=DEFAULT_ICD)
    parser.add_argument("--any-adapter", action="store_true")
    parser.add_argument("--no-render", action="store_true")
    parser.add_argument("--export-references", action="store_true")
    args = parser.parse_args()

    samples = sorted(args.corpus.glob("*.dxf"))
    if not samples:
        parser.error(f"no DXF files in {args.corpus}")
    args.out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    if args.icd:
        env["VK_ICD_FILENAMES"] = args.icd
    font_args = [item for font in args.font for item in ("--font", font)]

    report = []
    failed = 0
    for sample in samples:
        record = {"file": sample.name, "input_sha256": __import__("hashlib").sha256(sample.read_bytes()).hexdigest()}
        try:
            started = time.monotonic()
            scan = run_json(args.cli, "scan", sample, env, [])
            record["entities"] = scan["entities"]
            record["model_entities"] = scan["model_entities"]
            record["units"] = scan["units"]
            record["completeness"] = scan["completeness"]

            proxy = run_json(args.cli, "proxy-report", sample, env, [])
            record["entity_types"] = {
                e["type"]: e["render"] for e in proxy.get("entity_types", [])
            }

            build = run_json(args.cli, "build-representation", sample, env, font_args)
            if build["failures"]:
                raise RuntimeError(f"build failures: {build['failures']}")
            record["kinds"] = build["kind_counts"]
            record["primitives"] = build["primitives"]

            if not args.no_render:
                png = args.out / f"{sample.stem}.png"
                render = run_json(
                    args.cli,
                    "render",
                    sample,
                    env,
                    ["--png", str(png), "--width", str(args.width), "--height", str(args.height), *font_args],
                )
                adapter = render["adapter"]
                if not args.any_adapter:
                    assert adapter["backend"] == "vulkan", adapter
                    assert adapter["device_type"] == "cpu", adapter
                assert render["pixels"]["non_background"] > 0, "blank frame"
                record["pixels"] = render["pixels"]
                record["adapter"] = adapter

            if args.export_references:
                ref = args.out / f"{sample.stem}-ezdxf.png"
                record["reference_export"] = export_reference(sample, ref)

            record["seconds"] = round(time.monotonic() - started, 3)
            record["result"] = "passed"
        except Exception as error:  # noqa: BLE001 - record, do not fake success
            record["result"] = "failed"
            record["error"] = str(error)
            failed += 1
        report.append(record)
        print(f"{record['result']:>7}  {record['file']:<24} {record.get('completeness', {}).get('status', '?')}")

    (args.out / "summary.json").write_text(json.dumps(report, indent=2, ensure_ascii=False))
    unsupported = sorted(
        {
            f"{r['file']}:{kind}"
            for r in report
            if r.get("result") == "passed"
            for kind, status in r.get("entity_types", {}).items()
            if status == "unsupported"
        }
    )
    print(f"\n{len(report) - failed}/{len(report)} passed; unsupported entity types: {unsupported or 'none'}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
