#!/usr/bin/env python3
"""Contract: Android builds ignore ambient proxies without changing the caller."""
import os
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent


class AndroidBuildProxyContracts(unittest.TestCase):
    def test_build_children_have_no_proxy_and_sourcing_preserves_parent(self):
        with tempfile.TemporaryDirectory() as directory:
            temp = pathlib.Path(directory)
            mock_home = temp / "home"
            bin_dir = mock_home / ".cargo" / "bin"
            bin_dir.mkdir(parents=True)
            trace = temp / "child-environment.txt"
            for command in ("rustup", "cargo", "cargo-apk"):
                executable = bin_dir / command
                if command == "cargo-apk":
                    body = "exit 0\n"
                elif command == "cargo":
                    body = (
                        'if [[ "$1" == apk && "$2" == build ]]; then\n'
                        '  printf "%s\\n" "${http_proxy-unset}" "${https_proxy-unset}" "${all_proxy-unset}" "${no_proxy-unset}" "${HTTP_PROXY-unset}" "${HTTPS_PROXY-unset}" "${ALL_PROXY-unset}" "${NO_PROXY-unset}" "${ANDROID_BUILD_HTTPS_PROXY-unset}" > "$PROXY_TRACE"\n'
                        '  exit 73\n'
                        "fi\nexit 0\n"
                    )
                else:
                    body = 'exit 0\n'
                executable.write_text("#!/usr/bin/env bash\n" + body)
                executable.chmod(0o755)

            proxy_values = {
                "http_proxy": "http://lower-http.invalid",
                "https_proxy": "http://lower-https.invalid",
                "all_proxy": "socks5://lower-all.invalid",
                "no_proxy": "lower-no.invalid",
                "HTTP_PROXY": "http://upper-http.invalid",
                "HTTPS_PROXY": "http://upper-https.invalid",
                "ALL_PROXY": "socks5://upper-all.invalid",
                "NO_PROXY": "upper-no.invalid",
            }
            override = "http://proxy-override.invalid:10809"
            caller_values = {**proxy_values, "ANDROID_BUILD_HTTPS_PROXY": override}
            env = os.environ.copy()
            env.update(caller_values)
            env.update(
                {
                    "PATH": f"{bin_dir}:{env['PATH']}",
                    "HOME": str(mock_home),
                    "ANDROID_HOME": str(ROOT / "mock-android-sdk"),
                    "JAVA_HOME": str(ROOT / "mock-jdk"),
                    "ANDROID_NDK_HOME": str(ROOT / "mock-ndk"),
                    "TARGET": "aarch64-linux-android",
                    "PROXY_TRACE": str(trace),
                    "CARGO_TARGET_DIR": str(temp / "target"),
                }
            )
            checks = " && ".join(
                f'test "${{{key}}}" = "{value}"' for key, value in caller_values.items()
            )
            command = (
                f"source {str(ROOT / 'scripts/build-android.sh')!r}; "
                "status=$?; "
                f"{checks} || exit 1; "
                "test \"$status\" -eq 73"
            )
            result = subprocess.run(
                ["bash", "-c", command], env=env, text=True, capture_output=True
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(
                trace.read_text().splitlines(),
                [
                    "unset",
                    override,
                    "unset",
                    "unset",
                    "unset",
                    "unset",
                    "unset",
                    "unset",
                    override,
                ],
            )


if __name__ == "__main__":
    unittest.main()
