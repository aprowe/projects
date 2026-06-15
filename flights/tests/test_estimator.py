"""Tests for the estimator/snapshot layer using the bundled sample data."""

import os
import sys
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), os.pardir)))

from flight_noise.config import default_location  # noqa: E402
from flight_noise.estimator import estimate_from_states_file  # noqa: E402

SAMPLE = os.path.join(os.path.dirname(__file__), os.pardir, "data", "sample_states.json")


class TestSnapshotFromSample(unittest.TestCase):
    def setUp(self):
        self.snap = estimate_from_states_file(default_location(), os.path.abspath(SAMPLE))

    def test_has_estimates(self):
        self.assertTrue(self.snap.estimates)

    def test_sorted_loudest_first(self):
        dbas = [e.dba for e in self.snap.estimates]
        self.assertEqual(dbas, sorted(dbas, reverse=True))

    def test_loudest_is_the_low_departing_jet(self):
        # SWA1234 is the 737 climbing out directly overhead -> should be loudest.
        self.assertEqual(self.snap.loudest.callsign, "SWA1234")
        self.assertGreater(self.snap.loudest.dba, 80)

    def test_combined_level_at_least_loudest(self):
        self.assertIsNotNone(self.snap.combined_dba)
        self.assertGreaterEqual(self.snap.combined_dba, self.snap.loudest.dba)


if __name__ == "__main__":
    unittest.main()
