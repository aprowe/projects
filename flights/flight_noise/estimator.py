"""Tie the flight feed and acoustic model together.

Given a :class:`Location`, fetch the aircraft currently overhead and rank them
by the noise they produce at that location.
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass

from .acoustics import NoiseEstimate, estimate_aircraft_noise
from .config import Location
from .opensky import Aircraft, OpenSkyClient


@dataclass
class Snapshot:
    """Estimates for all positioned aircraft at one instant, loudest first."""

    location: Location
    estimates: list[NoiseEstimate]

    @property
    def loudest(self) -> NoiseEstimate | None:
        return self.estimates[0] if self.estimates else None

    @property
    def combined_dba(self) -> float | None:
        """Energy-summed level of all audible aircraft (incoherent addition)."""
        audible = [e.dba for e in self.estimates if e.audible]
        if not audible:
            return None
        total = sum(10 ** (d / 10.0) for d in audible)
        return round(10 * math.log10(total), 1)


def estimate_from_aircraft(location: Location, aircraft: list[Aircraft]) -> Snapshot:
    """Pure function: turn a list of aircraft into a ranked Snapshot."""
    estimates = [
        estimate_aircraft_noise(
            ac, location.lat, location.lon, location.ground_elevation_m
        )
        for ac in aircraft
        if ac.is_positioned
    ]
    estimates.sort(key=lambda e: e.dba, reverse=True)
    return Snapshot(location=location, estimates=estimates)


def estimate_live(location: Location, client: OpenSkyClient | None = None,
                  bbox_half_deg: float = 0.25) -> Snapshot:
    """Fetch live aircraft around ``location`` and estimate noise."""
    client = client or OpenSkyClient()
    aircraft = client.states_in_bbox(*location.bbox(bbox_half_deg))
    return estimate_from_aircraft(location, aircraft)


def estimate_from_states_file(location: Location, path: str) -> Snapshot:
    """Estimate from a saved OpenSky `/states/all` JSON response (demo/offline)."""
    with open(path) as f:
        payload = json.load(f)
    aircraft = [Aircraft.from_state(s) for s in (payload.get("states") or [])]
    aircraft = [a for a in aircraft if a.is_positioned and not a.on_ground]
    return estimate_from_aircraft(location, aircraft)
