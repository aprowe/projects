"""Load and save user settings (Roku IP, OpenSky keys, target address).

Settings live in a JSON file (``~/.config/flight-noise/config.json`` by default,
override with ``FLIGHT_NOISE_CONFIG``). Environment variables take precedence
over the file, so existing ``OPENSKY_CLIENT_ID`` / ``ROKU_IP`` etc. still work.
"""

from __future__ import annotations

import json
import os

ENV_OVERRIDES = {
    "opensky_client_id": "OPENSKY_CLIENT_ID",
    "opensky_client_secret": "OPENSKY_CLIENT_SECRET",
    "roku_ip": "ROKU_IP",
}

DEFAULTS = {
    "address": "5071 Lotus Street, San Diego, CA 92107",
    "lat": None,
    "lon": None,
    "elevation": None,
    "opensky_client_id": None,
    "opensky_client_secret": None,
    "roku_ip": None,
}


def config_path() -> str:
    if os.getenv("FLIGHT_NOISE_CONFIG"):
        return os.path.expanduser(os.environ["FLIGHT_NOISE_CONFIG"])
    base = os.getenv("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return os.path.join(base, "flight-noise", "config.json")


def load_settings() -> dict:
    """Return merged settings: defaults <- config file <- environment."""
    settings = dict(DEFAULTS)
    path = config_path()
    if os.path.exists(path):
        try:
            with open(path) as f:
                settings.update({k: v for k, v in json.load(f).items() if v is not None})
        except (OSError, ValueError):
            pass
    for key, env in ENV_OVERRIDES.items():
        if os.getenv(env):
            settings[key] = os.environ[env]
    return settings


def save_settings(settings: dict) -> str:
    """Persist settings to the config file, returning the path written."""
    path = config_path()
    os.makedirs(os.path.dirname(path), exist_ok=True)
    # Only store known keys; drop Nones to keep the file tidy.
    to_store = {k: settings.get(k) for k in DEFAULTS if settings.get(k) is not None}
    with open(path, "w") as f:
        json.dump(to_store, f, indent=2)
    try:
        os.chmod(path, 0o600)  # the file holds an API secret
    except OSError:
        pass
    return path
