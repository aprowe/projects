"""Baseline scheduler: greedy downlinks + "image whenever the sun is up".

Usage:
    python examples/greedy_scheduler.py --inprocess          # no server needed
    python examples/greedy_scheduler.py --url http://localhost:8080

It is intentionally naive -- a starting point to beat. Things it ignores:
uplink latency (activities before the plan reaches the satellite are skipped),
storage/power limits, request priorities, clouds, antenna fairness.
"""

from __future__ import annotations

import argparse
from collections import defaultdict
from datetime import datetime, timedelta

from planetsim import PlanBuilder, SimClient
from planetsim import models as m


def assign_downlinks(contacts: list[m.ContactWindow], antennas: dict[str, int], pad_s: float = 5.0):
    """Interval scheduling: take contacts in AOS order if the station has a free antenna
    and the satellite is not already booked. Returns {sat_id: [(station, start, end)]}."""
    station_busy: dict[str, list[tuple[datetime, datetime]]] = defaultdict(list)
    sat_busy: dict[str, list[tuple[datetime, datetime]]] = defaultdict(list)
    out: dict[str, list[tuple[str, datetime, datetime]]] = defaultdict(list)
    pad = timedelta(seconds=pad_s)

    def overlaps(lst, s, e):
        return sum(1 for a, b in lst if a < e and b > s)

    for c in sorted(contacts, key=lambda c: (c.aos, -c.max_elevation_deg)):
        s, e = c.aos + pad, c.los - pad
        if (e - s).total_seconds() < 120:
            continue
        if overlaps(sat_busy[c.satellite_id], s - timedelta(seconds=60), e + timedelta(seconds=60)):
            continue
        if overlaps(station_busy[c.station_id], s, e) >= antennas[c.station_id]:
            continue
        station_busy[c.station_id].append((s, e))
        sat_busy[c.satellite_id].append((s, e))
        out[c.satellite_id].append((c.station_id, s, e))
    return out


def daylight_intervals(eph: m.Ephemeris, min_sun_deg: float):
    out, cur = [], None
    for p in eph.points:
        if p.sun_elevation_deg >= min_sun_deg:
            cur = [p.time, p.time] if cur is None else [cur[0], p.time]
        elif cur is not None:
            out.append(tuple(cur))
            cur = None
    if cur is not None:
        out.append(tuple(cur))
    return [(s, e) for s, e in out if e > s]


def subtract(intervals, blocks, margin: timedelta):
    res = []
    for s, e in intervals:
        pieces = [(s, e)]
        for bs, be in blocks:
            bs, be = bs - margin, be + margin
            nxt = []
            for ps, pe in pieces:
                if be <= ps or bs >= pe:
                    nxt.append((ps, pe))
                    continue
                if ps < bs:
                    nxt.append((ps, bs))
                if be < pe:
                    nxt.append((be, pe))
            pieces = nxt
        res.extend(p for p in pieces if (p[1] - p[0]).total_seconds() >= 30)
    return res


def build_plans(sim: SimClient, start: datetime, horizon: timedelta) -> list[m.PlanSubmission]:
    end = start + horizon
    stations = {g.id: g.antennas for g in sim.ground_stations()}
    sats = sim.satellites()
    downlinks = assign_downlinks(sim.contacts(start, end), stations)
    plans = []
    for sat in sats:
        b = PlanBuilder(sat.id, start, end, label="greedy")
        dl = downlinks.get(sat.id, [])
        for st, s, e in dl:
            b.downlink(st, s, e)
        eph = sim.ephemeris(sat.id, start, end, step_s=30)
        light = daylight_intervals(eph, sat.spec.min_sun_elevation_deg)
        for s, e in subtract(light, [(s, e) for _, s, e in dl], timedelta(seconds=sat.spec.transition_time_s + 1)):
            b.image(max(s, start), min(e, end))
        plans.append(b.build())
    return plans


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://localhost:8080")
    ap.add_argument("--inprocess", action="store_true", help="use an in-process mock instead of --url")
    ap.add_argument("--hours", type=float, default=6.0, help="planning horizon per cycle")
    ap.add_argument("--cycles", type=int, default=4)
    ap.add_argument("--satellites", type=int, default=None)
    a = ap.parse_args()

    if a.inprocess:
        from planetsim.mock import inprocess_client

        sim = inprocess_client(seed=1, num_satellites=a.satellites, random_requests=300)
    else:
        sim = SimClient(a.url)
        sim.reset(seed=1, num_satellites=a.satellites, random_requests=300)
    sim.pause()

    horizon = timedelta(hours=a.hours)
    for cycle in range(a.cycles):
        now = sim.now()
        res = sim.submit_plans(build_plans(sim, now, horizon))
        print(f"[{now:%Y-%m-%d %H:%M}] cycle {cycle}: accepted {res.accepted}, rejected {res.rejected}")
        for item in res.items:
            if item.validation.violations:
                print("   ", item.index, [v.code for v in item.validation.violations][:5])
        sim.step(horizon)
        mt = sim.metrics()
        print(f"    images={mt.images_captured} down={mt.images_downlinked} "
              f"onboard={mt.data_onboard_mb / 1e3:.0f} GB coverage={mt.coverage_fraction:.1%} "
              f"latency={mt.mean_latency_s} req_fulfilled={mt.requests.fulfilled}/{mt.requests.total} "
              f"util={mt.station_utilization:.1%}")


if __name__ == "__main__":
    main()
