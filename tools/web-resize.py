#!/usr/bin/env python3
"""Ask a live web page whether its canvas can still see its own window.

This check is about layout and not about attributes, which is the whole reason
it exists. A canvas's `style.width` is a wish; `getBoundingClientRect()` is
what the page actually is. One regression pinned the canvas with CSS
`min-width`/`max-width` — the web backend's translation of
`set_min_inner_size` — after which every resize wrote a fresh width, the
attribute changed, and any check that read the attribute reported success
while the page sat immovably at the size it booted at. So this tool moves a
box and then reads the box the layout produced plus the drawing buffer behind
it, and calls it a pass only when those agree with the request.

It cannot resize a real browser window: Firefox's BiDi has no
`browsingContext.resize`, and headless Firefox ignores `--window-size`. The
forced box stands in for the gesture, and drives the path that matters — the
canvas's box moves, winit's device-pixel observer reports it, the surface
reconfigures, the bitmap is recreated — without needing a hand on a frame.
The bitmap is deliberately left alone: only the engine can bring it to the new
size, so its arrival is the proof that the renderer noticed rather than that
the page looks tidy.

    .workspace/venv/bin/python tools/web-resize.py --url http://localhost:8081/

Exits 0 when the canvas followed its box, 1 when it did not.
"""

import argparse
import base64
import importlib.util
import json
import pathlib
import subprocess
import sys
import time


def companion():
    """The screenshot tool, for its profile and socket helpers."""
    path = pathlib.Path(__file__).with_name("firefox-shot.py")
    spec = importlib.util.spec_from_file_location("firefox_shot", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# The four numbers, plus the clamps: a canvas that will not move is nearly
# always a canvas that may not, and `min-width` printed next to `rect` says so
# without anyone having to open an inspector.
GEOMETRY = """JSON.stringify((()=>{const c=document.querySelector("canvas");
if(!c) return {error:"no canvas on the page"};
const r=c.getBoundingClientRect(), cs=getComputedStyle(c);
return {rect:[Math.round(r.width),Math.round(r.height)],
        client:[c.clientWidth,c.clientHeight],
        buffer:[c.width,c.height],
        viewport:[innerWidth,innerHeight],
        dpr:devicePixelRatio,
        clamp:[cs.minWidth,cs.maxWidth,cs.minHeight,cs.maxHeight],
        style:[c.style.width,c.style.height]};})())"""


class Session:
    """The few BiDi calls this check needs, with the request numbering."""

    def __init__(self, socket):
        self.socket = socket
        self.next_id = 0
        # No session until there is one: `session.new` is the one call that
        # cannot carry a session id, and BiDi refuses the message rather than
        # ignoring the field.
        self.session = None
        self.session = self.call("session.new", {"capabilities": {"alwaysMatch": {}}})["sessionId"]
        tree = self.call("browsingContext.getTree", {})
        # The root context, not a created tab: a tab Firefox opens for us
        # never paints, and a white capture proves nothing about the page.
        self.context = next(c["context"] for c in tree["contexts"] if c.get("parent") is None)

    def call(self, method, params):
        self.next_id += 1
        message = {"id": self.next_id, "method": method, "params": params}
        if self.session is not None:
            message["sessionId"] = self.session
        self.socket.send(json.dumps(message))
        while True:
            reply = json.loads(self.socket.recv())
            if reply.get("id") == message["id"]:
                if "error" in reply:
                    raise SystemExit(f"{method}: {reply['error']}")
                return reply.get("result", {})

    def evaluate(self, expression):
        """Whatever the expression produced, or nothing when it produced none.

        An expression that mutates the page has no value, and BiDi says so
        with a `type: undefined` rather than a `value`, so the callers that
        want geometry parse it themselves.
        """
        result = self.call(
            "script.evaluate",
            {"expression": expression, "target": {"context": self.context},
             "awaitPromise": True, "resultOwnership": "none"},
        )
        return result["result"].get("value")

    def geometry(self):
        return json.loads(self.evaluate(GEOMETRY))

    def navigate(self, url):
        self.call("browsingContext.navigate", {"context": self.context, "url": url, "wait": "complete"})

    def shot(self, path):
        data = self.call("browsingContext.captureScreenshot",
                         {"context": self.context, "format": {"type": "png"}})
        path.write_bytes(base64.b64decode(data["data"]))


def show(label, geometry):
    print(f"  {label:12} rect={geometry['rect'][0]}x{geometry['rect'][1]}  "
          f"buffer={geometry['buffer'][0]}x{geometry['buffer'][1]}  "
          f"viewport={geometry['viewport'][0]}x{geometry['viewport'][1]}  "
          f"style={'/'.join(geometry['style'])}  clamp={'/'.join(geometry['clamp'])}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--url", default="http://localhost:8081/")
    parser.add_argument("--wait", type=float, default=20.0,
                        help="seconds to let the page warm (wasm uploads slowly)")
    parser.add_argument("--box", default="620x440", help="the box to force, WxH")
    parser.add_argument("--settle", type=float, default=3.0,
                        help="seconds to give the renderer to notice")
    parser.add_argument("--profile", type=pathlib.Path, default=pathlib.Path(".workspace/ff-resize"))
    parser.add_argument("--shot", type=pathlib.Path, default=None, help="optional screenshot after settling")
    parser.add_argument("--clamp", action="store_true",
                        help="pin the canvas to its boot size first, to check that this tool "
                             "notices: that is the regression it was written for")
    args = parser.parse_args()

    width, height = (int(part) for part in args.box.lower().split("x", 1))
    tools = companion()
    profile = tools.profile_with_webgpu(args.profile)
    port = tools.free_port()
    browser = subprocess.Popen(
        ["firefox", "--headless", "--no-remote", "--profile", str(profile),
         "--remote-debugging-port", str(port), "about:blank"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    try:
        with tools.wait_for_socket(port, browser) as socket:
            page = Session(socket)
            page.navigate(args.url)
            time.sleep(args.wait)
            before = page.geometry()
            if "error" in before:
                raise SystemExit(before["error"])
            show("as booted", before)

            if args.clamp:
                # The bug re-enacted from outside, with the same four
                # properties winit writes for `set_min_inner_size`. A check
                # that has never failed is a check nobody has read yet.
                page.evaluate("""(()=>{const c=document.querySelector("canvas");
                    const r=c.getBoundingClientRect();
                    for (const [name, value] of [["min-width", r.width], ["max-width", r.width],
                                                 ["min-height", r.height], ["max-height", r.height]])
                        c.style.setProperty(name, `${value}px`);})()""")
            page.evaluate(f"""(()=>{{const c=document.querySelector("canvas");
                c.style.width="{width}px"; c.style.height="{height}px";}})()""")
            time.sleep(args.settle)
            after = page.geometry()
            show(f"forced {width}x{height}", after)
            if args.shot:
                page.shot(args.shot)
    finally:
        browser.terminate()
        browser.wait(10)

    scale = after["dpr"]
    want = [round(width * scale), round(height * scale)]
    rect_ok = after["rect"] == [width, height]
    buffer_ok = after["buffer"] == want
    if rect_ok and buffer_ok:
        print(f"  the canvas followed its box, and the renderer redrew at {'x'.join(map(str, want))}")
        return 0
    if not rect_ok:
        print(f"  the box did not move: the style says {'/'.join(after['style'])} "
              f"and the layout says {after['rect'][0]}x{after['rect'][1]}")
        if after["clamp"] != ["none", "none", "none", "none"]:
            print(f"  and it is clamped: min/max are {after['clamp']} — a canvas pinned in "
                  f"both directions cannot follow a window however its width is written")
    else:
        print(f"  the box moved but the drawing buffer stayed {after['buffer'][0]}x"
              f"{after['buffer'][1]}, expected {'x'.join(map(str, want))}: the renderer "
              f"never heard, so look at the observer and the surface reconfigure")
    return 1


if __name__ == "__main__":
    sys.exit(main())
