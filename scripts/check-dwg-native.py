#!/usr/bin/env python3
"""External DWG corpus smoke check; real frames, not visual approval.

Uses the real hardware GPU by default: it never forces VK_ICD_FILENAMES, so the
Vulkan loader picks the real device (an inherited VK_ICD_FILENAMES is honored as
explicit user intent). Software Vulkan (Mesa lavapipe) is opt-in only via
``--software`` (or an explicit ``--icd`` path); software runs are labeled
SOFTWARE in the per-sample output.

Usage: python3 scripts/check-dwg-native.py ~/sources/cad-test-files OUT
       [--cli target/release/cad-cli-tools] [--font name=path ...]
       [--software | --icd /path/to/icd.json]
Corpus samples may live outside the repo or be committed fixtures; any committed
sample records its redistribution terms in fixtures/manifest.
"""
import argparse
import glob
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

SOFTWARE_ICD_TOKENS = ("lvp", "lavapipe", "llvmpipe", "swiftshader", "softpipe")


def is_software_icd(path) -> bool:
    return bool(path) and any(token in Path(path).name.lower() for token in SOFTWARE_ICD_TOKENS)


def find_software_icd() -> str | None:
    for pattern in ("*lvp*.json", "*lavapipe*.json"):
        matches = sorted(glob.glob(f"/usr/share/vulkan/icd.d/{pattern}"))
        if matches:
            return matches[0]
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--cli", default="target/release/cad-cli-tools")
    parser.add_argument("--font", action="append", default=[])
    parser.add_argument("--width", type=int, default=2160)
    parser.add_argument("--height", type=int, default=1520)
    parser.add_argument("--icd", default=None,
                        help="explicit Vulkan ICD path (default: none, real GPU). "
                             "A lavapipe/llvmpipe path is treated as software and labeled.")
    parser.add_argument("--software", action="store_true",
                        help="force Mesa lavapipe software Vulkan (labeled SOFTWARE); "
                             "the local software ICD is selected automatically")
    args = parser.parse_args()

    if args.software and not args.icd:
        args.icd = find_software_icd()
        if not args.icd:
            parser.error("--software given but no Mesa lavapipe ICD found under /usr/share/vulkan/icd.d "
                         "(install mesa-vulkan-drivers)")
    software = args.software or is_software_icd(args.icd)
    if not software and not args.icd and not os.environ.get("VK_ICD_FILENAMES") \
            and not glob.glob("/dev/dri/renderD*"):
        parser.error("no hardware GPU render node (/dev/dri/renderD*) and no VK_ICD_FILENAMES; "
                     "run on a machine with a GPU or pass --software to force Mesa lavapipe explicitly")

    samples = sorted(p for p in args.corpus.iterdir() if p.suffix.lower() == ".dwg")
    if not samples:
        parser.error("no DWG files in corpus")
    args.output.mkdir(parents=True, exist_ok=True)
    if software:
        print(f"check-dwg-native: SOFTWARE GPU mode (lavapipe); VK_ICD_FILENAMES={args.icd}; "
              f"frames are software-rendered, not a real GPU", file=sys.stderr)
    env = dict(os.environ)
    if args.icd:
        env["VK_ICD_FILENAMES"] = args.icd
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
                    if software:
                        assert document["adapter"]["device_type"] == "cpu"
                        assert document["adapter"]["driver"] == "llvmpipe"
                    assert document["pixels"]["non_background"] > 0, "blank frame"
                    assert png.read_bytes()[:8] == b"\x89PNG\r\n\x1a\n"
                    result["adapter"] = document["adapter"]
                    result["frame"] = document["frame"]
            result["graphics"] = "software (lavapipe)" if software else "hardware"
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
