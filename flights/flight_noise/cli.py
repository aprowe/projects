"""Command-line interface for the flight-noise estimator."""

from __future__ import annotations

import argparse
import os
import sys
import time

from .config import DEFAULT_ADDRESS, Location, default_location
from .ducker import DuckConfig, VolumeDucker
from .estimator import (
    Snapshot,
    estimate_from_states_file,
    estimate_live,
)
from .geocode import geocode
from .opensky import OpenSkyClient, OpenSkyError
from .roku import RokuError, RokuTV
from .settings import load_settings

SAMPLE_PATH = os.path.join(os.path.dirname(__file__), os.pardir, "data", "sample_states.json")


def _format_snapshot(snap: Snapshot, top: int, verbose: bool) -> str:
    lines = []
    loc = snap.location
    lines.append(f"Listener: {loc.address}")
    lines.append(f"          ({loc.lat:.5f}, {loc.lon:.5f}), ground ~{loc.ground_elevation_m:.0f} m MSL")
    lines.append("")

    if not snap.estimates:
        lines.append("No aircraft currently in range.")
        return "\n".join(lines)

    audible = [e for e in snap.estimates if e.audible]
    if snap.combined_dba is not None:
        lines.append(f"Combined audible level (all aircraft): {snap.combined_dba} dBA "
                     f"({len(audible)} audible of {len(snap.estimates)} tracked)")
    else:
        lines.append(f"{len(snap.estimates)} aircraft tracked; none above ambient ({45} dBA).")
    lines.append("")

    header = f"{'CALLSIGN':<10} {'dBA':>6} {'ALT(ft)':>8} {'SLANT(ft)':>10}  PERCEIVED"
    lines.append(header)
    lines.append("-" * len(header))
    for e in snap.estimates[:top]:
        alt_ft = e.altitude_m * 3.28084
        lines.append(
            f"{e.callsign:<10} {e.dba:>6.1f} {alt_ft:>8.0f} {e.slant_distance_ft:>10.0f}  {e.description}"
        )
        if verbose:
            comp = e.components
            terms = "  ".join(f"{k}={v:+.1f}" for k, v in comp.items())
            lines.append(f"           {terms}")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="flight-noise",
        description="Estimate how loud overhead aircraft are at a given address.",
    )
    p.add_argument("-a", "--address", default=None,
                   help=f"Street address to monitor (default: saved setting or "
                        f"{DEFAULT_ADDRESS!r}).")
    p.add_argument("--lat", type=float, help="Latitude (skips geocoding).")
    p.add_argument("--lon", type=float, help="Longitude (skips geocoding).")
    p.add_argument("--elevation", type=float, default=None,
                   help="Ground elevation at the address in metres MSL.")
    p.add_argument("--bbox", type=float, default=0.25,
                   help="Half-size of the search box in degrees (default 0.25).")
    p.add_argument("-n", "--top", type=int, default=10,
                   help="Show at most this many aircraft (default 10).")
    p.add_argument("-v", "--verbose", action="store_true",
                   help="Show the per-aircraft dB breakdown.")
    p.add_argument("--demo", action="store_true",
                   help="Use bundled sample data instead of the live API (offline).")
    p.add_argument("--watch", type=float, metavar="SECONDS", default=None,
                   help="Refresh continuously every SECONDS.")

    g = p.add_argument_group("Roku reverse-ducking")
    g.add_argument("--roku", action="store_true",
                   help="Boost a Roku TV's volume while a plane is loud, then "
                        "smoothly restore it (best with --watch).")
    g.add_argument("--roku-ip", default=None,
                   help="Roku TV IP address (else ROKU_IP env, else SSDP discovery).")
    g.add_argument("--roku-dry-run", action="store_true",
                   help="Print volume keypresses instead of sending them.")
    g.add_argument("--duck-floor", type=float, default=DuckConfig.db_floor,
                   help=f"dBA at/below which no boost is applied "
                        f"(default {DuckConfig.db_floor:.0f}).")
    g.add_argument("--duck-ceiling", type=float, default=DuckConfig.db_ceiling,
                   help=f"dBA at/above which full boost is applied "
                        f"(default {DuckConfig.db_ceiling:.0f}).")
    g.add_argument("--duck-max-steps", type=int, default=DuckConfig.max_boost_steps,
                   help=f"Max volume steps above baseline "
                        f"(default {DuckConfig.max_boost_steps}).")
    g.add_argument("--duck-rate", type=int, default=DuckConfig.max_step_per_tick,
                   help=f"Max volume steps changed per refresh "
                        f"(default {DuckConfig.max_step_per_tick}).")
    return p


def resolve_location(args, settings) -> Location:
    """Build a Location from CLI args, falling back to saved settings."""
    address = args.address or settings.get("address") or DEFAULT_ADDRESS
    lat = args.lat if args.lat is not None else settings.get("lat")
    lon = args.lon if args.lon is not None else settings.get("lon")
    elevation = args.elevation if args.elevation is not None else settings.get("elevation")

    if lat is not None and lon is not None:
        loc = default_location()
        loc.address, loc.lat, loc.lon = address, float(lat), float(lon)
    else:
        loc = geocode(address)
    if elevation is not None:
        loc.ground_elevation_m = float(elevation)
    return loc


def opensky_client(settings) -> OpenSkyClient:
    return OpenSkyClient(
        client_id=settings.get("opensky_client_id"),
        client_secret=settings.get("opensky_client_secret"),
    )


def _setup_roku(args, settings):
    """Build (RokuTV, VolumeDucker) if --roku is set, else (None, None)."""
    if not args.roku:
        return None, None

    transport = None
    if args.roku_dry_run:
        transport = lambda key: print(f"    [roku dry-run] keypress {key}")  # noqa: E731

    ip = args.roku_ip or settings.get("roku_ip")
    if not ip and not args.roku_dry_run:
        print("Discovering Roku via SSDP...", file=sys.stderr)
        ip = RokuTV.discover()
        if not ip:
            print("error: no Roku found; set --roku-ip or ROKU_IP.", file=sys.stderr)
            raise SystemExit(2)
        print(f"Found Roku at {ip}", file=sys.stderr)

    roku = RokuTV(ip=ip or "0.0.0.0", transport=transport)
    cfg = DuckConfig(
        db_floor=args.duck_floor,
        db_ceiling=args.duck_ceiling,
        max_boost_steps=args.duck_max_steps,
        max_step_per_tick=args.duck_rate,
    )
    return roku, VolumeDucker(cfg)


def _apply_duck(roku, ducker, snap) -> None:
    """Nudge the Roku volume toward the level the loudest plane calls for."""
    loud = snap.loudest
    dba = loud.dba if (loud and loud.audible) else None
    delta = ducker.update(dba)
    try:
        roku.apply_delta(delta)
    except RokuError as e:
        print(f"  Roku: control failed: {e}", file=sys.stderr)
        return
    arrow = "+" if delta > 0 else ("" if delta == 0 else "-")
    target = ducker.target_steps(dba)
    src = f"{dba:.1f} dBA ({loud.callsign})" if dba is not None else "quiet"
    print(f"  Roku reverse-duck: {arrow}{abs(delta)} step(s) -> "
          f"offset +{ducker.applied}/{ducker.cfg.max_boost_steps} "
          f"(target +{target}, source {src})")


def _run_once(args, location, settings, roku=None, ducker=None) -> int:
    try:
        if args.demo:
            snap = estimate_from_states_file(location, os.path.abspath(SAMPLE_PATH))
        else:
            client = opensky_client(settings)
            if not client.authenticated:
                print("warning: no OPENSKY_CLIENT_ID/SECRET set; trying anonymous "
                      "access (often blocked). Use --demo for an offline example.",
                      file=sys.stderr)
            snap = estimate_live(location, client, bbox_half_deg=args.bbox)
    except OpenSkyError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    print(_format_snapshot(snap, args.top, args.verbose))
    if roku is not None and ducker is not None:
        print()
        _apply_duck(roku, ducker, snap)
    return 0


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    settings = load_settings()
    location = resolve_location(args, settings)
    roku, ducker = _setup_roku(args, settings)

    if args.watch:
        try:
            while True:
                print("\033[2J\033[H", end="")  # clear screen
                print(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}]  refreshing every {args.watch}s "
                      f"(Ctrl-C to quit)\n")
                _run_once(args, location, settings, roku, ducker)
                time.sleep(args.watch)
        except KeyboardInterrupt:
            # Restore the TV to its baseline volume before exiting.
            if roku is not None and ducker is not None:
                restore = ducker.reset()
                if restore:
                    print(f"\nRestoring Roku volume ({restore:+d} steps)...")
                    try:
                        roku.apply_delta(restore)
                    except RokuError:
                        pass
            return 0
    return _run_once(args, location, settings, roku, ducker)


def serve_main(argv=None) -> int:
    """Entry point for `python -m flight_noise serve` (the web dashboard)."""
    from . import server  # local import keeps the base CLI lightweight

    p = argparse.ArgumentParser(prog="flight-noise serve",
                                description="Launch the flight-noise web dashboard.")
    p.add_argument("-a", "--address", default=None)
    p.add_argument("--lat", type=float)
    p.add_argument("--lon", type=float)
    p.add_argument("--elevation", type=float)
    p.add_argument("-p", "--port", type=int, default=8000)
    p.add_argument("--interval", type=float, default=8.0,
                   help="Seconds between flight refreshes (default 8).")
    p.add_argument("--bbox", type=float, default=0.25)
    p.add_argument("--demo", action="store_true",
                   help="Serve bundled sample data instead of live flights.")
    p.add_argument("--roku-ip", default=None)
    p.add_argument("--roku-dry-run", action="store_true",
                   help="Log volume keypresses instead of sending them.")
    p.add_argument("--no-duck", action="store_true",
                   help="Start with auto reverse-ducking off (controls still work).")
    p.add_argument("--duck-floor", type=float, default=DuckConfig.db_floor)
    p.add_argument("--duck-ceiling", type=float, default=DuckConfig.db_ceiling)
    p.add_argument("--duck-max-steps", type=int, default=DuckConfig.max_boost_steps)
    p.add_argument("--duck-rate", type=int, default=DuckConfig.max_step_per_tick)
    args = p.parse_args(argv)

    settings = load_settings()
    location = resolve_location(args, settings)

    # Build a Roku client if we have any way to reach one, so the dashboard's
    # controls and auto-duck are available.
    roku = None
    ip = args.roku_ip or settings.get("roku_ip")
    if ip or args.roku_dry_run:
        transport = None
        if args.roku_dry_run:
            transport = lambda key: print(f"[roku dry-run] keypress {key}")  # noqa: E731
        roku = RokuTV(ip=ip or "0.0.0.0", transport=transport)

    duck_cfg = DuckConfig(
        db_floor=args.duck_floor, db_ceiling=args.duck_ceiling,
        max_boost_steps=args.duck_max_steps, max_step_per_tick=args.duck_rate,
    )

    return server.serve(
        location, port=args.port, demo=args.demo, interval=args.interval,
        bbox=args.bbox, opensky=opensky_client(settings), roku=roku,
        duck_config=duck_cfg, auto_duck=not args.no_duck,
    )


def dispatch(argv=None) -> int:
    """Top-level dispatcher: `setup` / `serve` subcommands, else the estimator."""
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] == "setup":
        from .setup_wizard import run_wizard
        return run_wizard()
    if argv and argv[0] == "serve":
        return serve_main(argv[1:])
    return main(argv)


if __name__ == "__main__":
    raise SystemExit(dispatch())
