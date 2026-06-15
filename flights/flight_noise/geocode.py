"""Address -> latitude/longitude using OpenStreetMap's free Nominatim service.

Falls back to the bundled default coordinates for the project's home address if
the network call fails (e.g. blocked, rate-limited, or offline).
"""

from __future__ import annotations

import json
import urllib.parse
import urllib.request

from .config import DEFAULT_ADDRESS, DEFAULT_LAT, DEFAULT_LON, Location

NOMINATIM_URL = "https://nominatim.openstreetmap.org/search"
USER_AGENT = "flight-noise-estimator/1.0 (contact: example@example.com)"


def geocode(address: str) -> Location:
    """Resolve ``address`` to a :class:`Location`.

    On any failure for the default address we return the hardcoded fallback so
    the tool still works offline; for other addresses we re-raise.
    """
    params = urllib.parse.urlencode({"q": address, "format": "json", "limit": 1})
    req = urllib.request.Request(
        f"{NOMINATIM_URL}?{params}", headers={"User-Agent": USER_AGENT}
    )
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            results = json.load(resp)
        if not results:
            raise ValueError(f"No geocoding result for: {address!r}")
        top = results[0]
        return Location(address=top.get("display_name", address),
                        lat=float(top["lat"]), lon=float(top["lon"]))
    except Exception:
        if address.strip().lower() == DEFAULT_ADDRESS.lower():
            return Location(DEFAULT_ADDRESS, DEFAULT_LAT, DEFAULT_LON)
        raise
