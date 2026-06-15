"""Interactive setup: discover the Roku TV and capture API keys.

Run with ``python3 -m flight_noise setup``. Walks through Roku discovery,
OpenSky credentials, and the target address, then writes them to the config
file so the estimator and dashboard can pick them up automatically.
"""

from __future__ import annotations

import getpass

from .roku import RokuTV
from .settings import config_path, load_settings, save_settings


def _ask(prompt: str, default: str | None = None) -> str:
    suffix = f" [{default}]" if default else ""
    try:
        ans = input(f"{prompt}{suffix}: ").strip()
    except EOFError:
        ans = ""
    return ans or (default or "")


def _yes(prompt: str, default: bool = True) -> bool:
    d = "Y/n" if default else "y/N"
    ans = _ask(f"{prompt} ({d})").lower()
    if not ans:
        return default
    return ans.startswith("y")


def _setup_roku(settings: dict) -> None:
    print("\n--- Roku TV ---")
    current = settings.get("roku_ip")
    if current and not _yes(f"Roku IP is set to {current}. Reconfigure?", default=False):
        return

    ip = None
    if _yes("Scan the network for your Roku now?", default=True):
        print("Searching (a few seconds)...")
        ip = RokuTV.discover()
        if ip:
            print(f"  Found a Roku at {ip}")
            if not _yes(f"Use {ip}?", default=True):
                ip = None
        else:
            print("  No Roku found via discovery.")

    if not ip:
        ip = _ask("Enter Roku IP address (blank to skip)") or None

    if ip:
        tv = RokuTV(ip=ip)
        print("  Testing connection...")
        if tv.is_reachable():
            print("  Connected.")
            if _yes("Send a quick volume up/down test?", default=False):
                try:
                    tv.volume_up(1)
                    tv.volume_down(1)
                    print("  Sent VolumeUp + VolumeDown.")
                except Exception as e:  # noqa: BLE001
                    print(f"  Test failed: {e}")
        else:
            print("  Warning: could not reach the TV; saving the IP anyway.")
    settings["roku_ip"] = ip


def _setup_opensky(settings: dict) -> None:
    print("\n--- OpenSky Network API (live flight data) ---")
    print("Create a client at https://opensky-network.org/ -> Account -> API Client.")
    has = settings.get("opensky_client_id")
    if has and not _yes(f"Client ID already set ({has}). Replace it?", default=False):
        return
    cid = _ask("OpenSky client id (blank to skip / use demo mode only)")
    if not cid:
        return
    secret = getpass.getpass("OpenSky client secret (hidden): ").strip()
    settings["opensky_client_id"] = cid
    settings["opensky_client_secret"] = secret or settings.get("opensky_client_secret")


def _setup_address(settings: dict) -> None:
    print("\n--- Target address ---")
    settings["address"] = _ask("Address to monitor", settings.get("address"))
    if _yes("Pin exact coordinates (skip geocoding)?", default=False):
        lat = _ask("Latitude", str(settings.get("lat") or ""))
        lon = _ask("Longitude", str(settings.get("lon") or ""))
        try:
            settings["lat"] = float(lat)
            settings["lon"] = float(lon)
        except ValueError:
            print("  Invalid numbers; leaving coordinates to geocoding.")
    elev = _ask("Ground elevation in metres MSL (optional)",
                str(settings.get("elevation") or ""))
    try:
        settings["elevation"] = float(elev) if elev else settings.get("elevation")
    except ValueError:
        pass


def run_wizard() -> int:
    print("=" * 60)
    print(" Flight Noise Estimator — setup")
    print("=" * 60)
    settings = load_settings()

    _setup_roku(settings)
    _setup_opensky(settings)
    _setup_address(settings)

    path = save_settings(settings)
    print(f"\nSaved configuration to {path}")
    print("Secrets are stored with 0600 permissions. You're ready:")
    print("  python3 -m flight_noise            # one-shot estimate")
    print("  python3 -m flight_noise serve      # web dashboard")
    return 0


if __name__ == "__main__":
    raise SystemExit(run_wizard())
