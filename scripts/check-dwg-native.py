#!/usr/bin/env python3
"""External DWG corpus smoke check; real lavapipe frames, not visual approval.

Usage: python3 scripts/check-dwg-native.py ~/sources/cad-test-files OUT
       [--cli target/release/cad-cli-tools] [--font name=path ...]
Samples/reference images stay outside the repo (no redistribution implied).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--cli", default="target/release/cad-cli-tools")
    parser.add_argument("--font", action="append", default=[])
    parser.add_argument("--width", type=int, default=2160)
    parser.add_argument("--height", type=int, default=1520)
    parser.add_argument("--icd", default="/usr/share/vulkan/icd.d/lvp_icd.json")
    args = parser.parse_args()
    samples = sorted(p for p in args.corpus.iterdir() if p.suffix.lower() == ".dwg")
    if not samples:
        parser.error("no DWG files in corpus")
    args.output.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, VK_ICD_FILENAMES=args.icd)
    results = []
    for sample in samples:
        result = {"sample": sample.name, "sha256": hashlib.sha256(sample.read_bytes()).hexdigest()}
        results.append(result)
        try:
            for operation in ["scan", "build-representation", "render"]:
                command = [args.cli, operation, str(sample)]
                if operation != "scan":
                    for font in args.font:
                        command.extend(["--font", font])
                png = args.output / f"{sample.stem}.png"
                if operation == "render":
                    command.extend(["--png", str(png), "--width", str(args.width),
                                    "--height", str(args.height)])
                started = time.monotonic()
                process = subprocess.run(command, env=env, capture_output=True, text=True, timeout=660)
                (args.output / f"{sample.stem}-{operation}.stderr.log").write_text(process.stderr)
                if process.returncode:
                    raise RuntimeError(f"{operation} exited {process.returncode}: {process.stderr}")
                document = json.loads(process.stdout)
                (args.output / f"{sample.stem}-{operation}.json").write_text(process.stdout)
                result[operation + "_seconds"] = round(time.monotonic() - started, 3)
                if operation == "scan":
                    assert document["model_entities"] > 0, "no model-space entities"
                    result["entities"] = document["entities"]
                    result["import_completeness"] = document["completeness"]
                elif operation == "build-representation":
                    assert not document["failures"], "representation failures"
                    result["kind_counts"] = document["kind_counts"]
                else:
                    assert document["adapter"]["backend"] == "vulkan"
                    assert document["adapter"]["device_type"] == "cpu"
                    assert document["adapter"]["driver"] == "llvmpipe"
                    assert document["pixels"]["non_background"] > 0, "blank frame"
                    assert png.read_bytes()[:8] == b"\x89PNG\r\n\x1a\n"
                    result["adapter"] = document["adapter"]
                    result["frame"] = document["frame"]
            result["smoke"] = "passed"
            result["visual_acceptance"] = "manual reference review required; smoke is not fidelity"
        except Exception as error:
            result["smoke"] = "failed"
            result["error"] = str(error)
        print(json.dumps(result, ensure_ascii=False))
    (args.output / "summary.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
    return int(any(r["smoke"] != "passed" for r in results))


if __name__ == "__main__":
    raise SystemExit(main())
