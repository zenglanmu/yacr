#!/usr/bin/env python3
"""Capture a single frame from the GNOME/Mutter ScreenCast API via GStreamer.

This is the same-UID user flow used for the native Wayland backend, where the
host owns a ``wl_subsurface`` and an XGetImage window grab is not available: the
invoking user talks to ``org.gnome.Mutter.ScreenCast`` on their own session bus
(no portal impersonation, no permission-state change, no privilege escalation).

Flow: CreateSession -> RecordMonitor -> subscribe ``PipeWireStreamAdded`` on the
returned stream object -> Session.Start -> wait for the PipeWire node ->
``gst-launch-1.0`` grabs one frame -> always Session.Stop().

Exit code is 0 only when a non-empty PNG is produced; every failure path is
non-zero with an explicit English message and never writes a placeholder PNG.
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys

SC_DEST = "org.gnome.Mutter.ScreenCast"
SC_ROOT = "/org/gnome/Mutter/ScreenCast"
DISPLAY_CONFIG = "org.gnome.Mutter.DisplayConfig"
DISPLAY_CONFIG_PATH = "/org/gnome/Mutter/DisplayConfig"
DEFAULT_CONNECTOR = "eDP-1"


def resolve_connector(bus, dbus):
    """Primary monitor connector from DisplayConfig, else the first, else default."""
    try:
        obj = bus.get_object(DISPLAY_CONFIG, DISPLAY_CONFIG_PATH)
        config = dbus.Interface(obj, DISPLAY_CONFIG)
        state = config.GetCurrentState()
        monitors = state[1]
        fallback = None
        for monitor in monitors:
            connector = str(monitor[0])
            if fallback is None:
                fallback = connector
            props = monitor[4]
            if bool(props.get("primary", False)):
                return connector
        if fallback is not None:
            return fallback
    except Exception:
        pass
    return DEFAULT_CONNECTOR


def build_pipeline_argv(node, out):
    return [
        "gst-launch-1.0", "-e",
        "pipewiresrc", "path={}".format(node),
        "num-buffers=1", "do-timestamp=true",
        "keepalive-time=1000", "resend-last=true",
        "!", "video/x-raw",
        "!", "videoconvert",
        "!", "pngenc",
        "!", "filesink", "location={}".format(out),
    ]


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Capture one frame from Mutter ScreenCast to a PNG.",
    )
    parser.add_argument("--out", required=True, help="Absolute output PNG path.")
    parser.add_argument(
        "--connector",
        default=None,
        help="Monitor connector (default: primary monitor, else {}).".format(
            DEFAULT_CONNECTOR
        ),
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=10.0,
        help="Seconds to wait for the PipeWire node (default: 10).",
    )
    parser.add_argument(
        "--gst-timeout",
        type=float,
        default=30.0,
        help="Seconds to wait for the GStreamer grab (default: 30).",
    )
    args = parser.parse_args(argv)

    out = os.path.abspath(args.out)

    try:
        import dbus
        import dbus.mainloop.glib
        from gi.repository import GLib
    except Exception as error:  # pragma: no cover - environment dependent
        print(
            "error: python dbus/GLib bindings unavailable: {}".format(error),
            file=sys.stderr,
        )
        return 3

    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)

    try:
        bus = dbus.SessionBus()
    except Exception as error:
        print(
            "error: cannot connect to the session bus: {}".format(error),
            file=sys.stderr,
        )
        return 4

    connector = args.connector or resolve_connector(bus, dbus)

    session = None
    state = {"node": None}
    try:
        root_obj = bus.get_object(SC_DEST, SC_ROOT)
        screencast = dbus.Interface(root_obj, SC_DEST)
        session_path = screencast.CreateSession({})
        session = dbus.Interface(
            bus.get_object(SC_DEST, session_path), SC_DEST + ".Session"
        )

        props = {"cursor-mode": dbus.UInt32(1), "is-recording": dbus.Boolean(True)}
        stream_path = session.RecordMonitor(connector, props)
        stream = dbus.Interface(
            bus.get_object(SC_DEST, stream_path), SC_DEST + ".Stream"
        )

        loop = GLib.MainLoop()

        def on_added(node_id):
            state["node"] = int(node_id)
            loop.quit()

        # Subscribe BEFORE Start so the signal is never missed while the session
        # introspection (and this connection) stay alive.
        stream.connect_to_signal("PipeWireStreamAdded", on_added)

        def on_timeout():
            loop.quit()
            return False

        GLib.timeout_add(int(max(1.0, args.timeout) * 1000), on_timeout)
        session.Start()
        loop.run()

        node = state["node"]
        if node is None:
            print(
                "error: no PipeWireStreamAdded signal within {:.0f}s".format(
                    args.timeout
                ),
                file=sys.stderr,
            )
            return 5

        try:
            result = subprocess.run(
                build_pipeline_argv(node, out),
                capture_output=True,
                text=True,
                timeout=args.gst_timeout,
            )
        except subprocess.TimeoutExpired:
            print(
                "error: gst-launch-1.0 timed out after {:.0f}s".format(
                    args.gst_timeout
                ),
                file=sys.stderr,
            )
            return 6

        if result.returncode != 0:
            detail = (result.stderr or "").strip()
            print(
                "error: gst-launch-1.0 failed (exit {}): {}".format(
                    result.returncode, detail
                ),
                file=sys.stderr,
            )
            return 7

        if not os.path.exists(out) or os.path.getsize(out) == 0:
            print(
                "error: no frame was written to {}".format(out),
                file=sys.stderr,
            )
            return 8

        print("wrote {} (connector={}, node={})".format(out, connector, node))
        return 0
    except Exception as error:
        print(
            "error: screencast capture failed: {}".format(error),
            file=sys.stderr,
        )
        return 9
    finally:
        if session is not None:
            try:
                session.Stop()
            except Exception:
                pass
        try:
            bus.close()
        except Exception:
            pass


if __name__ == "__main__":
    sys.exit(main())
