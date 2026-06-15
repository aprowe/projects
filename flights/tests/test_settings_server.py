"""Tests for settings persistence and the dashboard state (no network)."""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), os.pardir)))

from flight_noise import settings as settings_mod  # noqa: E402
from flight_noise.config import default_location  # noqa: E402
from flight_noise.ducker import DuckConfig, VolumeDucker  # noqa: E402
from flight_noise.opensky import OpenSkyClient  # noqa: E402
from flight_noise.roku import RokuTV  # noqa: E402
from flight_noise.server import DashboardState  # noqa: E402


class TestSettings(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.NamedTemporaryFile(suffix=".json", delete=False)
        self.tmp.close()
        os.environ["FLIGHT_NOISE_CONFIG"] = self.tmp.name
        for k in ("OPENSKY_CLIENT_ID", "ROKU_IP"):
            os.environ.pop(k, None)

    def tearDown(self):
        os.unlink(self.tmp.name)
        os.environ.pop("FLIGHT_NOISE_CONFIG", None)
        os.environ.pop("ROKU_IP", None)

    def test_round_trip(self):
        settings_mod.save_settings({"roku_ip": "10.0.0.5", "address": "X St"})
        loaded = settings_mod.load_settings()
        self.assertEqual(loaded["roku_ip"], "10.0.0.5")
        self.assertEqual(loaded["address"], "X St")

    def test_env_overrides_file(self):
        settings_mod.save_settings({"roku_ip": "10.0.0.5"})
        os.environ["ROKU_IP"] = "192.168.0.9"
        self.assertEqual(settings_mod.load_settings()["roku_ip"], "192.168.0.9")

    def test_secret_file_permissions(self):
        path = settings_mod.save_settings({"opensky_client_secret": "shh"})
        mode = os.stat(path).st_mode & 0o777
        self.assertEqual(mode, 0o600)


SAMPLE = os.path.join(os.path.dirname(__file__), os.pardir, "data", "sample_states.json")


class TestDashboardState(unittest.TestCase):
    def _state(self, **kw):
        self.keys = []
        roku = RokuTV(ip="1.2.3.4", transport=self.keys.append)
        return DashboardState(
            location=default_location(),
            opensky=OpenSkyClient(),
            roku=roku,
            ducker=VolumeDucker(DuckConfig(max_step_per_tick=2)),
            demo=True, interval=1, **kw,
        )

    def test_poll_builds_state_with_geo_planes(self):
        st = self._state()
        st.poll_once()
        data = st.state_json()
        self.assertTrue(data["planes"])
        first = data["planes"][0]
        self.assertIn("lat", first)
        self.assertIn("lon", first)
        self.assertEqual(first["callsign"], "SWA1234")  # loudest sorts first
        self.assertEqual(len(data["history"]), 1)
        # Reverse-duck applied a positive volume nudge for the loud jet.
        self.assertGreater(data["roku"]["offset"], 0)
        self.assertTrue(any(k == "VolumeUp" for k in self.keys))

    def test_manual_volume_action(self):
        st = self._state()
        self.assertEqual(st.roku_action("volume_down"), {"ok": True})
        self.assertIn("VolumeDown", self.keys)

    def test_disabling_auto_duck_restores_baseline(self):
        st = self._state()
        st.poll_once()
        self.assertGreater(st.ducker.applied, 0)
        st.set_auto_duck(False)
        self.assertFalse(st.auto_duck)
        self.assertEqual(st.ducker.applied, 0)  # restored to baseline


if __name__ == "__main__":
    unittest.main()
