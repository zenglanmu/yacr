#!/usr/bin/env python3
"""Static + mutation contracts for scripts/package-linux-release.sh.

The packaging script is a heavy, host-specific release build (it links the Slint
GUI, relocates shared libraries and optionally renders), so it is not run in CI.
This guard rail instead asserts the script keeps its packaging guarantees:

  * it builds and stages *both* the headless CLI and the GUI host;
  * it relocates the GUI's non-base shared libraries into ``lib/`` and links the
    binary with a relocatable ``$ORIGIN/../lib`` RPATH;
  * it refuses to package a bundle whose GUI cannot load or whose RPATH is not
    honoured;
  * it produces the tarball.

Each guarantee is deleted in turn and the checker must report it, so the contract
cannot silently degrade into an empty success. This checks the script text only;
it is not evidence that a package was built or that its GUI renders.
"""
from __future__ import annotations

import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/package-linux-release.sh"

# (human label, fragment that must appear in the script).
REQUIRED_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("headless CLI build", "cargo build --release --offline --locked -p cad-cli-tools"),
    ("GUI host build", "cargo rustc -p app-linux --bin yacr-linux --release --offline --locked"),
    ("relocatable RPATH link flag", "link-arg=-Wl,--disable-new-dtags,-rpath,"),
    ("RPATH points at lib/", "GUI_RPATH='$ORIGIN/../lib'"),
    ("stage CLI binary", 'cp "$BIN" "$STAGE/bin/cad-cli-tools"'),
    ("stage GUI binary", 'cp "$GUI" "$STAGE/bin/yacr-linux"'),
    ("relocate GUI libraries", 'copy_gui_libs "$GUI" "$STAGE/lib"'),
    ("refuse unresolved libraries", "grep -q 'not found'"),
    ("verify RPATH is honoured", 'grep -Fq "$STAGE"'),
    ("verify GUI loads", "--headless requires --output NEW_DIRECTORY"),
    ("optional GUI smoke gate", "YACR_LINUX_SMOKE"),
    ("tarball output", 'tar -C "$DIST" -czf "$TARBALL" "$NAME"'),
)


def check_script(text: str) -> list[str]:
    """Return one problem string per missing packaging guarantee."""
    return [
        f"missing {label}: {fragment!r}"
        for label, fragment in REQUIRED_FRAGMENTS
        if fragment not in text
    ]


class PackageLinuxReleaseContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.text = SCRIPT.read_text(encoding="utf-8")

    def test_real_script_satisfies_every_guarantee(self) -> None:
        self.assertEqual(check_script(self.text), [])

    def test_each_guarantee_is_required(self) -> None:
        for label, fragment in REQUIRED_FRAGMENTS:
            with self.subTest(guarantee=label):
                mutated = self.text.replace(fragment, "")
                self.assertNotEqual(
                    check_script(mutated), [], f"mutation not detected: {label}"
                )

    def test_gui_binary_cannot_be_dropped_silently(self) -> None:
        mutated = self.text.replace('cp "$GUI" "$STAGE/bin/yacr-linux"', "")
        problems = check_script(mutated)
        self.assertTrue(problems)
        self.assertTrue(any("GUI" in problem for problem in problems))

    def test_library_relocation_cannot_be_dropped_silently(self) -> None:
        mutated = self.text.replace(
            'if [ "$BUNDLE_LIBS" = "1" ]; then\n'
            "  echo \"==> bundling yacr-linux shared libraries\"\n"
            '  copy_gui_libs "$GUI" "$STAGE/lib"\n',
            "",
        )
        self.assertNotEqual(check_script(mutated), [])


if __name__ == "__main__":
    unittest.main()
