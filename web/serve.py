#!/usr/bin/env python3
"""Serve the web/ build with no caching whatsoever.

The stock `python -m http.server` sends no Cache-Control header, so
Firefox applies heuristic freshness and can keep replaying a stale
wasm — a build so old it still drew the blue frost rim — through
page reloads. This server says `no-store` on every response: every
reload fetches the actual files on disk, always.

    python3 web/serve.py 8081
"""
import functools
import http.server
import pathlib
import sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8081
ROOT = pathlib.Path(__file__).resolve().parent


class NoStore(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store, max-age=0")
        super().end_headers()


if __name__ == "__main__":
    handler = functools.partial(NoStore, directory=str(ROOT))
    print(f"serving {ROOT} at http://localhost:{PORT} (no-store)")
    http.server.ThreadingHTTPServer(("localhost", PORT), handler).serve_forever()
