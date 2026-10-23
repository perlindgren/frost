#!/usr/bin/env python3
"""Photograph a page in headless Firefox N seconds *after* it loads.

`firefox --screenshot` is not that. It fires the instant the document finishes
loading, and a wasm app is still uploading its atlas at that moment, so what
comes back is the loading card — dark, a line of text in the middle, and
indistinguishable from a black screen unless you measure it. That frame sent me
looking for a compositor bug that did not exist. So: drive the browser over
WebDriver BiDi, wait, then capture.

    tools/firefox-shot.py http://localhost:8081/ hill.png --wait 35

The profile gets two prefs unless they are already there, because Firefox on
this distro ships WebGPU switched off and refuses this driver's Vulkan device:
`dom.webgpu.enabled` and `gfx.webgpu.ignore-blocklist`. Without them the page
renders nothing and the tool would be photographing a config error.

Requires a websocket client (`.workspace/venv/bin/pip install websockets`);
run it with whichever interpreter has one.
"""

import argparse
import base64
import json
import os
import pathlib
import socket
import subprocess
import sys
import time

try:
    import websockets.sync.client as ws
except ImportError:
    sys.exit("needs the `websockets` package — pip install websockets")

PREFS = {
    "dom.webgpu.enabled": True,
    "gfx.webgpu.ignore-blocklist": True,
    "datareporting.policy.dataSubmissionEnabled": False,
    "toolkit.telemetry.reportingpolicy.firstRun": False,
}


def profile_with_webgpu(path: pathlib.Path) -> pathlib.Path:
    """Make (or reuse) a profile whose prefs actually enable WebGPU."""
    path.mkdir(parents=True, exist_ok=True)
    user_js = path / "user.js"
    have = user_js.read_text() if user_js.exists() else ""
    missing = [f'user_pref("{k}", {str(v).lower()});' for k, v in PREFS.items() if f'"{k}"' not in have]
    if missing:
        with user_js.open("a") as f:
            f.write("\n".join(missing) + "\n")
    return path


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_socket(port: int, proc: subprocess.Popen, timeout: float = 60.0):
    """Open the BiDi socket once Firefox's remote agent is listening.

    Firefox answers BiDi at `ws://127.0.0.1:PORT/session` and serves no
    `/json/version` for discovery, unlike Chrome — probing the HTTP path there
    returns 404 and looks like a dead browser. So connect, and retry.
    """
    url = f"ws://127.0.0.1:{port}/session"
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        if proc.poll() is not None:
            sys.exit(f"firefox exited early with {proc.returncode}")
        try:
            return ws.connect(url, max_size=64 << 20)
        except Exception as e:  # connection refused until the agent starts
            last = e
            time.sleep(0.25)
    sys.exit(f"no BiDi socket on port {port} after {timeout}s: {last}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("url")
    ap.add_argument("out")
    ap.add_argument("--wait", type=float, default=20.0, help="seconds after load before capturing")
    ap.add_argument("--size", default="1280,800", help="window size, WxH")
    ap.add_argument("--profile", default=None, help="profile dir; gets the WebGPU prefs added")
    ap.add_argument("--freeze", action="store_true",
                    help="starve requestAnimationFrame first, so the held frame is what gets shot")
    args = ap.parse_args()

    prof = profile_with_webgpu(pathlib.Path(args.profile) if args.profile
                               else pathlib.Path(".workspace/ff-shot-profile"))
    port = free_port()
    proc = subprocess.Popen(
        ["firefox", "--headless", "--no-remote", "--profile", str(prof),
         "--window-size", args.size, "--remote-debugging-port", str(port), "about:blank"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    next_id = 1

    def send(conn, method, params=None, session=None):
        nonlocal next_id
        msg = {"id": next_id, "method": method, "params": params or {}}
        if session:
            msg["sessionId"] = session
        next_id += 1
        conn.send(json.dumps(msg))
        while True:  # skip events: they arrive unsolicited and interleave
            reply = json.loads(conn.recv())
            if reply.get("id") == msg["id"]:
                if "error" in reply:
                    sys.exit(f"{method}: {reply['error']}")
                return reply.get("result", {})

    try:
        with wait_for_socket(port, proc) as conn:
            session = send(conn, "session.new", {"capabilities": {"alwaysMatch": {}}})["sessionId"]
            # Navigate the tab Firefox itself opened rather than creating one:
            # a created tab stays inactive in headless, never paints, and the
            # capture comes back a clean white rectangle that reads like a
            # blank page and is nothing of the kind.
            tree = send(conn, "browsingContext.getTree", {}, session)["contexts"]
            # Firefox 157 omits `type` on the root entry, so select the root by
            # having no parent rather than by being called a tab.
            roots = [c for c in tree if c.get("parent") is None]
            if not roots:
                sys.exit(f"no context in the tree: {tree}")
            context = roots[0]["context"]
            send(conn, "browsingContext.navigate",
                 {"context": context, "url": args.url, "wait": "complete"}, session)
            time.sleep(args.wait)
            if args.freeze:
                # winit drives the wasm event loop from rAF; replacing it with a
                # no-op means nothing schedules the next frame, and a canvas keeps
                # whatever it last presented. The frame is then a specimen rather
                # than a rolling number, which is all anyone wants from an overlay.
                send(conn, "script.evaluate", {"expression": "window.requestAnimationFrame = () => 0;",
                                               "target": {"context": context}, "awaitPromise": True,
                                               "resultOwnership": "none"}, session)
                time.sleep(1.0)
            send(conn, "browsingContext.activate", {"context": context}, session)
            time.sleep(0.5)
            shot = send(conn, "browsingContext.captureScreenshot",
                        {"context": context, "format": {"type": "png"}}, session)
        pathlib.Path(args.out).write_bytes(base64.b64decode(shot["data"]))
        print(f"{args.out}: {os.path.getsize(args.out)} bytes after {args.wait:g}s, {args.size} window")
    finally:
        proc.terminate()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    main()
