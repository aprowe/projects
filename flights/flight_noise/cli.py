"""Command-line interface for the flight-noise estimator."""

from __future__ import annotations

import argparse
import os
import sys
import time

from .config import DEFAULT_ADDRESS, default_location
from .estimator import (
    Snapshot,
    estimate_from_states_file,
    estimate_live,
)
from .geocode import geocode
from .opensky import OpenSkyClient, OpenSkyError

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
    p.add_argument("-a", "--address", default=DEFAULT_ADDRESS,
                   help=f"Street address to monitor (default: {DEFAULT_ADDRESS!r}).")
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
    return p


def _resolve_location(args):
    if args.lat is not None and args.lon is not None:
        loc = default_location()
        loc.address = args.address
        loc.lat, loc.lon = args.lat, args.lon
    else:
        loc = geocode(args.address)
    if args.elevation is not None:
        loc.ground_elevation_m = args.elevation
    return loc


def _run_once(args, location) -> int:
    try:
        if args.demo:
            snap = estimate_from_states_file(location, os.path.abspath(SAMPLE_PATH))
        else:
            client = OpenSkyClient()
            if not client.authenticated:
                print("warning: no OPENSKY_CLIENT_ID/SECRET set; trying anonymous "
                      "access (often blocked). Use --demo for an offline example.",
                      file=sys.stderr)
            snap = estimate_live(location, client, bbox_half_deg=args.bbox)
    except OpenSkyError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    print(_format_snapshot(snap, args.top, args.verbose))
    return 0


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    location = _resolve_location(args)

    if args.watch:
        try:
            while True:
                print("\033[2J\033[H", end="")  # clear screen
                print(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}]  refreshing every {args.watch}s "
                      f"(Ctrl-C to quit)\n")
                _run_once(args, location)
                time.sleep(args.watch)
        except KeyboardInterrupt:
            return 0
    return _run_once(args, location)


if __name__ == "__main__":
    raise SystemExit(main())
