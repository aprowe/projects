"""Web dashboard: live map, volume chart, and Roku TV controls.

Runs a small stdlib HTTP server. A background thread polls flights on an
interval, runs the noise model + reverse-duck, and keeps a rolling history.
The browser polls ``/api/state`` for JSON and POSTs to ``/api/roku/*`` and
``/api/duck/*`` to control the TV.

    python3 -m flight_noise serve [--port 8000] [--demo] [--interval 8]
"""

from __future__ import annotations

import json
import mimetypes
import os
import threading
import time
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from .config import Location, default_location
from .ducker import DuckConfig, VolumeDucker
from .estimator import estimate_from_states_file, estimate_live
from .opensky import OpenSkyClient, OpenSkyError
from .roku import RokuError, RokuTV

WEB_DIR = os.path.join(os.path.dirname(__file__), "web")
SAMPLE_PATH = os.path.join(os.path.dirname(__file__), os.pardir, "data", "sample_states.json")


class DashboardState:
    """Thread-safe shared state polled by the background worker and HTTP API."""

    def __init__(self, location: Location, opensky: OpenSkyClient,
                 roku: RokuTV | None, ducker: VolumeDucker, *,
                 demo: bool = False, interval: float = 8.0, bbox: float = 0.25,
                 history_len: int = 240):
        self.location = location
        self.opensky = opensky
        self.roku = roku
        self.ducker = ducker
        self.demo = demo
        self.interval = interval
        self.bbox = bbox
        self.auto_duck = roku is not None
        self.lock = threading.Lock()
        self.snapshot = None
        self.error: str | None = None
        self.history: deque = deque(maxlen=history_len)
        self._stop = threading.Event()

    # -- polling -------------------------------------------------------------

    def poll_once(self) -> None:
        try:
            if self.demo:
                snap = estimate_from_states_file(self.location, os.path.abspath(SAMPLE_PATH))
            else:
                snap = estimate_live(self.location, self.opensky, bbox_half_deg=self.bbox)
            err = None
        except OpenSkyError as e:
            snap, err = None, str(e)

        offset = self.ducker.applied
        if snap is not None and self.roku is not None and self.auto_duck:
            loud = snap.loudest
            dba = loud.dba if (loud and loud.audible) else None
            delta = self.ducker.update(dba)
            try:
                self.roku.apply_delta(delta)
            except RokuError as e:
                err = (err + "; " if err else "") + f"roku: {e}"
            offset = self.ducker.applied

        with self.lock:
            if snap is not None:
                self.snapshot = snap
            self.error = err
            loud = snap.loudest if snap else None
            self.history.append({
                "t": int(time.time() * 1000),
                "loudest_dba": round(loud.dba, 1) if (loud and loud.audible) else None,
                "combined_dba": snap.combined_dba if snap else None,
                "volume_offset": offset,
            })

    def run(self) -> None:
        self.poll_once()
        while not self._stop.wait(self.interval):
            self.poll_once()

    def stop(self) -> None:
        self._stop.set()

    # -- serialization -------------------------------------------------------

    def state_json(self) -> dict:
        with self.lock:
            snap = self.snapshot
            history = list(self.history)
            error = self.error
        planes = []
        if snap:
            for e in snap.estimates:
                planes.append({
                    "callsign": e.callsign,
                    "icao24": e.icao24,
                    "lat": e.latitude,
                    "lon": e.longitude,
                    "heading": e.heading,
                    "dba": e.dba,
                    "altitude_ft": round(e.altitude_m * 3.28084),
                    "slant_ft": round(e.slant_distance_ft),
                    "audible": e.audible,
                    "description": e.description,
                })
        loud = snap.loudest if snap else None
        return {
            "location": {
                "address": self.location.address,
                "lat": self.location.lat,
                "lon": self.location.lon,
            },
            "planes": planes,
            "loudest": ({"callsign": loud.callsign, "dba": loud.dba,
                         "description": loud.description}
                        if (loud and loud.audible) else None),
            "combined_dba": snap.combined_dba if snap else None,
            "history": history,
            "error": error,
            "roku": {
                "configured": self.roku is not None,
                "ip": getattr(self.roku, "ip", None),
                "auto_duck": self.auto_duck,
                "offset": self.ducker.applied,
                "max_steps": self.ducker.cfg.max_boost_steps,
            },
            "demo": self.demo,
        }

    # -- TV controls (called from the HTTP handler) --------------------------

    def roku_action(self, action: str) -> dict:
        if self.roku is None:
            return {"ok": False, "error": "no Roku configured"}
        try:
            if action == "volume_up":
                self.roku.volume_up(1)
            elif action == "volume_down":
                self.roku.volume_down(1)
            elif action == "mute":
                self.roku.keypress("VolumeMute")
            else:
                return {"ok": False, "error": f"unknown action {action!r}"}
        except RokuError as e:
            return {"ok": False, "error": str(e)}
        return {"ok": True}

    def set_auto_duck(self, enabled: bool) -> dict:
        with self.lock:
            self.auto_duck = enabled and self.roku is not None
            if not self.auto_duck and self.roku is not None:
                # Hand volume back to the user at baseline.
                restore = self.ducker.reset()
                try:
                    self.roku.apply_delta(restore)
                except RokuError:
                    pass
        return {"ok": True, "auto_duck": self.auto_duck}


def make_handler(state: DashboardState):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):  # quiet by default
            pass

        def _send(self, code, body, content_type="application/json"):
            data = body.encode() if isinstance(body, str) else body
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            if self.path in ("/", "/index.html"):
                try:
                    with open(os.path.join(WEB_DIR, "index.html"), "rb") as f:
                        self._send(200, f.read(), "text/html; charset=utf-8")
                except OSError:
                    self._send(500, "dashboard html missing", "text/plain")
            elif self.path == "/api/state":
                self._send(200, json.dumps(state.state_json()))
            elif self.path.startswith("/static/"):
                self._serve_static()
            else:
                self._send(404, json.dumps({"error": "not found"}))

        def _serve_static(self):
            # Serve image assets the user drops into the web/ folder (e.g. a
            # real logo at web/logo.png -> /static/logo.png). Basename only.
            name = os.path.basename(self.path.split("?", 1)[0])
            fp = os.path.join(WEB_DIR, name)
            allowed = (".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp")
            if os.path.isfile(fp) and name.lower().endswith(allowed):
                ctype = mimetypes.guess_type(fp)[0] or "application/octet-stream"
                with open(fp, "rb") as f:
                    self._send(200, f.read(), ctype)
            else:
                self._send(404, json.dumps({"error": "not found"}))

        def do_POST(self):
            if self.path.startswith("/api/roku/"):
                result = state.roku_action(self.path.rsplit("/", 1)[-1])
            elif self.path == "/api/duck/on":
                result = state.set_auto_duck(True)
            elif self.path == "/api/duck/off":
                result = state.set_auto_duck(False)
            else:
                result = {"ok": False, "error": "not found"}
            self._send(200 if result.get("ok") else 400, json.dumps(result))

    return Handler


def serve(location: Location, *, port: int = 8000, demo: bool = False,
          interval: float = 8.0, bbox: float = 0.25,
          opensky: OpenSkyClient | None = None, roku: RokuTV | None = None,
          duck_config: DuckConfig | None = None, auto_duck: bool = True) -> int:
    state = DashboardState(
        location=location,
        opensky=opensky or OpenSkyClient(),
        roku=roku,
        ducker=VolumeDucker(duck_config or DuckConfig()),
        demo=demo, interval=interval, bbox=bbox,
    )
    state.auto_duck = auto_duck and roku is not None
    worker = threading.Thread(target=state.run, daemon=True)
    worker.start()

    httpd = ThreadingHTTPServer(("0.0.0.0", port), make_handler(state))
    print(f"Flight-noise dashboard: http://localhost:{port}")
    print(f"  monitoring: {location.address}")
    print(f"  mode: {'demo (sample data)' if demo else 'live'}; "
          f"roku: {'on' if roku else 'off'}; refresh: {interval}s")
    print("  Ctrl-C to stop.")
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        state.stop()
        if roku is not None:
            try:
                roku.apply_delta(state.ducker.reset())
            except RokuError:
                pass
        httpd.server_close()
    return 0
