"""Tests for the reverse-ducking controller and Roku transport plumbing."""

import os
import sys
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), os.pardir)))

from flight_noise.ducker import DuckConfig, VolumeDucker  # noqa: E402
from flight_noise.roku import RokuTV  # noqa: E402


class TestTargetMapping(unittest.TestCase):
    def setUp(self):
        self.d = VolumeDucker(DuckConfig(db_floor=60, db_ceiling=95, max_boost_steps=12))

    def test_quiet_means_no_boost(self):
        self.assertEqual(self.d.target_steps(None), 0)
        self.assertEqual(self.d.target_steps(40), 0)
        self.assertEqual(self.d.target_steps(60), 0)

    def test_ceiling_means_full_boost(self):
        self.assertEqual(self.d.target_steps(95), 12)
        self.assertEqual(self.d.target_steps(120), 12)

    def test_midpoint_is_proportional(self):
        # 77.5 dBA is halfway between floor and ceiling -> ~6 steps.
        self.assertEqual(self.d.target_steps(77.5), 6)

    def test_louder_never_targets_less(self):
        prev = -1
        for dba in range(55, 100):
            t = self.d.target_steps(dba)
            self.assertGreaterEqual(t, prev)
            prev = t


class TestSmoothing(unittest.TestCase):
    def test_ramps_up_gradually(self):
        d = VolumeDucker(DuckConfig(db_floor=60, db_ceiling=95,
                                    max_boost_steps=12, max_step_per_tick=2))
        # A loud plane (86 dBA -> target 9) should ramp, not jump.
        deltas = [d.update(86.0) for _ in range(6)]
        self.assertEqual(deltas, [2, 2, 2, 2, 1, 0])
        self.assertEqual(d.applied, 9)

    def test_ramps_back_down_when_quiet(self):
        d = VolumeDucker(DuckConfig(max_step_per_tick=2))
        for _ in range(8):
            d.update(95.0)  # ramp to max
        self.assertEqual(d.applied, 12)
        # Plane gone: should descend back toward 0.
        down = [d.update(None) for _ in range(8)]
        self.assertTrue(all(x <= 0 for x in down))
        self.assertEqual(d.applied, 0)

    def test_reset_returns_inverse_offset(self):
        d = VolumeDucker()
        d.applied = 7
        self.assertEqual(d.reset(), -7)
        self.assertEqual(d.applied, 0)


class TestRokuTransport(unittest.TestCase):
    def test_apply_delta_sends_correct_keypresses(self):
        sent = []
        tv = RokuTV(ip="1.2.3.4", transport=sent.append)
        tv.apply_delta(3, pause=0)
        tv.apply_delta(-2, pause=0)
        tv.apply_delta(0, pause=0)
        self.assertEqual(sent, ["VolumeUp"] * 3 + ["VolumeDown"] * 2)


if __name__ == "__main__":
    unittest.main()
