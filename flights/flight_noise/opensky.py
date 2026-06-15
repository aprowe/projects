"""Client for the OpenSky Network flight-tracker API.

OpenSky is a free, community ADS-B network. In 2025 it moved from anonymous /
HTTP-basic access to OAuth2 *client credentials*. This client supports both:

  * OAuth2 (recommended): set OPENSKY_CLIENT_ID and OPENSKY_CLIENT_SECRET.
    Create an API client at https://opensky-network.org/ -> Account.
  * Anonymous: no credentials. Heavily rate-limited and increasingly returns
    HTTP 403; provided only as a best-effort fallback.

Only the Python standard library is used, so no `pip install` is required.
"""

from __future__ import annotations

import json
import os
import time
import urllib.parse
import urllib.request
from dataclasses import dataclass

API_BASE = "https://opensky-network.org/api"
TOKEN_URL = (
    "https://auth.opensky-network.org/auth/realms/"
    "opensky-network/protocol/openid-connect/token"
)
USER_AGENT = "flight-noise-estimator/1.0 (+https://opensky-network.org)"


@dataclass
class Aircraft:
    """A single aircraft state vector, normalised from the OpenSky response."""

    icao24: str
    callsign: str
    origin_country: str
    longitude: float
    latitude: float
    altitude_m: float | None      # geometric altitude MSL (falls back to baro)
    on_ground: bool
    velocity_mps: float | None    # ground speed
    true_track: float | None      # heading, degrees
    vertical_rate_mps: float | None
    category: int                 # ADS-B emitter category (size class)

    @classmethod
    def from_state(cls, s: list) -> "Aircraft":
        """Build from an OpenSky state-vector array (see API docs for indices)."""

        def at(i):
            return s[i] if i < len(s) else None

        geo_alt = at(13)
        baro_alt = at(7)
        return cls(
            icao24=(at(0) or "").strip(),
            callsign=(at(1) or "").strip(),
            origin_country=at(2) or "",
            longitude=at(5),
            latitude=at(6),
            altitude_m=geo_alt if geo_alt is not None else baro_alt,
            on_ground=bool(at(8)),
            velocity_mps=at(9),
            true_track=at(10),
            vertical_rate_mps=at(11),
            category=int(at(17)) if at(17) is not None else 0,
        )

    @property
    def is_positioned(self) -> bool:
        return self.latitude is not None and self.longitude is not None


class OpenSkyError(RuntimeError):
    pass


class OpenSkyClient:
    def __init__(self, client_id: str | None = None, client_secret: str | None = None):
        self.client_id = client_id or os.getenv("OPENSKY_CLIENT_ID")
        self.client_secret = client_secret or os.getenv("OPENSKY_CLIENT_SECRET")
        self._token: str | None = None
        self._token_expiry: float = 0.0

    @property
    def authenticated(self) -> bool:
        return bool(self.client_id and self.client_secret)

    def _get_token(self) -> str:
        if self._token and time.time() < self._token_expiry - 30:
            return self._token
        data = urllib.parse.urlencode(
            {
                "grant_type": "client_credentials",
                "client_id": self.client_id,
                "client_secret": self.client_secret,
            }
        ).encode()
        req = urllib.request.Request(
            TOKEN_URL,
            data=data,
            headers={
                "Content-Type": "application/x-www-form-urlencoded",
                "User-Agent": USER_AGENT,
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=20) as resp:
                payload = json.load(resp)
        except urllib.error.HTTPError as e:
            raise OpenSkyError(f"OAuth token request failed: {e}") from e
        self._token = payload["access_token"]
        self._token_expiry = time.time() + float(payload.get("expires_in", 1800))
        return self._token

    def states_in_bbox(
        self, lat_min: float, lon_min: float, lat_max: float, lon_max: float
    ) -> list[Aircraft]:
        """Fetch all aircraft state vectors within a lat/lon bounding box."""
        params = {
            "lamin": lat_min,
            "lomin": lon_min,
            "lamax": lat_max,
            "lomax": lon_max,
            "extended": 1,  # include the emitter-category field
        }
        url = f"{API_BASE}/states/all?" + urllib.parse.urlencode(params)
        headers = {"User-Agent": USER_AGENT}
        if self.authenticated:
            headers["Authorization"] = f"Bearer {self._get_token()}"

        req = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=25) as resp:
                payload = json.load(resp)
        except urllib.error.HTTPError as e:
            hint = (
                " (anonymous access is often blocked — set OPENSKY_CLIENT_ID and "
                "OPENSKY_CLIENT_SECRET)"
                if not self.authenticated
                else ""
            )
            raise OpenSkyError(f"states request failed: {e}{hint}") from e

        states = payload.get("states") or []
        aircraft = [Aircraft.from_state(s) for s in states]
        return [a for a in aircraft if a.is_positioned and not a.on_ground]
