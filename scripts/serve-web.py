#!/usr/bin/env python3
"""Static server for the yacr web build with correct wasm MIME and COOP/COEP.

Spec v2.0 §9.2: the browser build must be static-deployable; `application/wasm`
and a secure context are required. COOP/COEP are only needed for the optional
shared-memory path, so they are opt-in via --cross-origin-isolation.

Usage: scripts/serve-web.py [--directory web-dist] [--port 8090] [--coi]
"""
from __future__ import annotations

import argparse
import functools
import http.server
import mimetypes
import pathlib

mimetypes.add_type("application/wasm", ".wasm")
mimetypes.add_type("application/javascript", ".js")
mimetypes.add_type("text/css", ".css")


class Handler(http.server.SimpleHTTPRequestHandler):
    coi = False

    def end_headers(self) -> None:  # noqa: D102
        if self.coi:
            self.send_header("Cross-Origin-Opener-Policy", "same-origin")
            self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, fmt: str, *args) -> None:  # noqa: D102
        print(f"[serve-web] {self.address_string()} {fmt % args}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", default="web-dist")
    parser.add_argument("--port", type=int, default=8090)
    parser.add_argument("--coi", action="store_true", help="enable COOP/COEP headers")
    args = parser.parse_args()
    directory = pathlib.Path(args.directory).resolve()
    if not (directory / "index.html").is_file():
        raise SystemExit(f"{directory} has no index.html; run scripts/build-web.sh first")
    Handler.coi = args.coi
    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), functools.partial(Handler, directory=str(directory)))
    print(f"[serve-web] http://127.0.0.1:{args.port}/ serving {directory}")
    server.serve_forever()


if __name__ == "__main__":
    main()