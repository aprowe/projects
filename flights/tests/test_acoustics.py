"""Unit tests for the acoustic model (pure, no network)."""

import math
import os
import sys
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), os.pardir)))

from flight_noise.acoustics import (  # noqa: E402
    describe_level,
    estimate_aircraft_noise,
    haversine_m,
)
from flight_noise.opensky import Aircraft  # noqa: E402


def make_aircraft(**kw):
    base = dict(
        icao24="test01", callsign="TST1", origin_country="US",
        longitude=-117.2417, latitude=32.7448, altitude_m=1000.0,
        on_ground=False, velocity_mps=100.0, true_track=270.0,
        vertical_rate_mps=0.0, category=4,
    )
    base.update(kw)
    return Aircraft(**base)


class TestHaversine(unittest.TestCase):
    def test_zero_distance(self):
        self.assertAlmostEqual(haversine_m(32.74, -117.24, 32.74, -117.24), 0.0, places=3)

    def test_known_distance(self):
        # ~1 deg of latitude is ~111 km.
        d = haversine_m(32.0, -117.0, 33.0, -117.0)
        self.assertAlmostEqual(d, 111_195, delta=500)


class TestAcousticModel(unittest.TestCase):
    LAT, LON = 32.7448, -117.2417

    def test_large_jet_overhead_is_loud(self):
        # 737-class climbing at ~305 m slant directly overhead -> calibrated ~88 dBA.
        ac = make_aircraft(altitude_m=305.0, vertical_rate_mps=12.0, category=4,
                           latitude=self.LAT, longitude=self.LON)
        est = estimate_aircraft_noise(ac, self.LAT, self.LON, listener_elevation_m=0.0)
        self.assertGreater(est.dba, 85)
        self.assertLess(est.dba, 100)
        self.assertTrue(est.audible)

    def test_farther_is_quieter(self):
        near = make_aircraft(altitude_m=1000.0, latitude=self.LAT, longitude=self.LON)
        far = make_aircraft(altitude_m=1000.0, latitude=self.LAT + 0.2, longitude=self.LON)
        near_est = estimate_aircraft_noise(near, self.LAT, self.LON)
        far_est = estimate_aircraft_noise(far, self.LAT, self.LON)
        self.assertGreater(near_est.dba, far_est.dba)

    def test_doubling_distance_drops_about_6db(self):
        # Spreading loss is ~6 dB per doubling. Use short ranges directly
        # overhead so atmospheric absorption (5 dB/km) stays negligible.
        a1 = make_aircraft(altitude_m=150.0, vertical_rate_mps=0.0,
                           velocity_mps=0.0, latitude=self.LAT, longitude=self.LON)
        a2 = make_aircraft(altitude_m=300.0, vertical_rate_mps=0.0,
                           velocity_mps=0.0, latitude=self.LAT, longitude=self.LON)
        d1 = estimate_aircraft_noise(a1, self.LAT, self.LON).dba
        d2 = estimate_aircraft_noise(a2, self.LAT, self.LON).dba
        # ~6 dB from spreading plus <1 dB of extra atmospheric absorption.
        self.assertAlmostEqual(d1 - d2, 6.0, delta=1.5)

    def test_heavier_is_louder_than_light(self):
        heavy = make_aircraft(category=6, latitude=self.LAT, longitude=self.LON)
        light = make_aircraft(category=2, latitude=self.LAT, longitude=self.LON)
        self.assertGreater(
            estimate_aircraft_noise(heavy, self.LAT, self.LON).dba,
            estimate_aircraft_noise(light, self.LAT, self.LON).dba,
        )

    def test_high_cruise_is_quiet(self):
        ac = make_aircraft(altitude_m=11000.0, category=6,
                           latitude=self.LAT + 0.1, longitude=self.LON + 0.1)
        est = estimate_aircraft_noise(ac, self.LAT, self.LON)
        self.assertLess(est.dba, 55)

    def test_climbing_louder_than_level(self):
        climb = make_aircraft(altitude_m=600.0, vertical_rate_mps=12.0,
                             latitude=self.LAT, longitude=self.LON)
        level = make_aircraft(altitude_m=600.0, vertical_rate_mps=0.0,
                             latitude=self.LAT, longitude=self.LON)
        self.assertGreater(
            estimate_aircraft_noise(climb, self.LAT, self.LON).dba,
            estimate_aircraft_noise(level, self.LAT, self.LON).dba,
        )


class TestDescribe(unittest.TestCase):
    def test_levels(self):
        self.assertIn("inaudible", describe_level(30))
        self.assertIn("very loud", describe_level(90))
        self.assertIn("extremely", describe_level(100))


if __name__ == "__main__":
    unittest.main()
