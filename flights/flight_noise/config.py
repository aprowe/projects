"""Default configuration: the target address and search geometry."""

from __future__ import annotations

from dataclasses import dataclass

# Default target: 5071 Lotus Street, San Diego, CA 92107 (Point Loma).
# Point Loma sits directly under the departure path of San Diego International
# (KSAN) runway 27, which makes it a great real-world test case.
# Coordinates are used as a fallback when geocoding is unavailable.
DEFAULT_ADDRESS = "5071 Lotus Street, San Diego, CA 92107"
DEFAULT_LAT = 32.7448
DEFAULT_LON = -117.2417
DEFAULT_GROUND_ELEVATION_M = 30.0  # approx terrain elevation at the address (MSL)

# Half-size of the bounding box (in degrees) to scan around the address.
# ~0.25 deg latitude ~= 28 km, generous enough to catch climbing departures.
DEFAULT_BBOX_HALF_DEG = 0.25


@dataclass
class Location:
    address: str
    lat: float
    lon: float
    ground_elevation_m: float = DEFAULT_GROUND_ELEVATION_M

    def bbox(self, half_deg: float = DEFAULT_BBOX_HALF_DEG):
        """Return (lat_min, lon_min, lat_max, lon_max) around this location."""
        return (
            self.lat - half_deg,
            self.lon - half_deg,
            self.lat + half_deg,
            self.lon + half_deg,
        )


def default_location() -> Location:
    return Location(DEFAULT_ADDRESS, DEFAULT_LAT, DEFAULT_LON)
