"""End-to-end tests: SimClient -> FastAPI mock -> engine."""

from __future__ import annotations

import json
from datetime import timedelta
from pathlib import Path

import pytest

from planetsim import PlanBuilder, PlanRejected, SimAPIError
from planetsim import models as m
from planetsim.mock import inprocess_client

H = timedelta(hours=1)


@pytest.fixture()
def sim():
    c = inprocess_client(seed=3, num_satellites=8)
    yield c
    c.close()


def first_contact(sim, sat_id=None, after=None, min_dur=240):
    t0 = after or sim.now()
    cs = sim.contacts(t0, t0 + 12 * H, satellite_ids=[sat_id] if sat_id else None)
    return next(c for c in cs if c.duration_s >= min_dur and c.aos > t0 + timedelta(seconds=30))


def next_uplink_contact(sim, sat_id, min_dur=240):
    """Step the (paused) sim until ``sat_id``'s very next contact is a long one, and return it.

    Plans uplink at the first contact of any kind, so tests that reason about *when* a plan
    reaches the satellite must make sure no shorter pass comes first.
    """
    while True:
        t0 = sim.now()
        cs = sim.contacts(t0, t0 + 12 * H, satellite_ids=[sat_id])
        c = cs[0]
        if c.aos > t0 + timedelta(seconds=30) and c.duration_s >= min_dur:
            return c
        sim.step(until=c.los + timedelta(seconds=30))


def daylight_window(sim, sat_id, start, end, minutes=10):
    eph = sim.ephemeris(sat_id, start, end, step_s=30)
    run = []
    for p in eph.points:
        run = run + [p.time] if p.sun_elevation_deg > 20 else []
        if len(run) * 30 >= minutes * 60:
            return run[0], run[-1]
    raise AssertionError("no daylight window")


def test_status_scenario_and_snapshot(sim):
    st = sim.status()
    assert not st.running and st.sim_time == st.epoch
    sc = sim.scenario()
    assert sum(len(f.satellite_ids) for f in sc.flocks) == 8
    assert all(95 < f.inclination_deg < 99 for f in sc.flocks)  # sun-synchronous
    snap = sim.constellation()
    assert len(snap.satellites) == 8 and len(snap.ground_stations) == len(sc.ground_station_ids)
    s = snap.satellites[0]
    assert 470 < s.position.alt_km < 530
    assert s.spec.pointing == "nadir_only"


def test_step_and_sim_control(sim):
    st = sim.step(600)
    assert st.sim_time == st.epoch + timedelta(seconds=600)
    sim.resume(rate=10)
    with pytest.raises(SimAPIError) as ei:
        sim.step(10)
    assert ei.value.code == "SIM_RUNNING"
    sim.pause()


def test_contacts_are_sane(sim):
    t0 = sim.now()
    cs = sim.contacts(t0, t0 + 6 * H)
    assert cs
    for c in cs:
        assert c.aos <= c.tca <= c.los
        assert c.max_elevation_deg >= 10
        assert 0 < c.duration_s < 20 * 60


def test_validation_errors(sim):
    t0 = sim.now()
    sat = "SD-001"
    c = first_contact(sim, sat)
    # downlink outside any contact, overlapping imaging, and too-short transition
    bad = (PlanBuilder(sat, t0, t0 + 6 * H)
           .image(c.aos - timedelta(seconds=300), c.aos + timedelta(seconds=10))
           .downlink(c.station_id, c.aos, c.los)
           .downlink(c.station_id, c.los + 2 * H, c.los + 2 * H + timedelta(seconds=60))
           .build())
    v = sim.validate_plan(bad)
    codes = {x.code for x in v.violations}
    assert not v.valid
    assert {"OVERLAP", "NOT_IN_CONTACT"} <= codes
    with pytest.raises(PlanRejected) as ei:
        sim.submit_plan(bad)
    assert ei.value.status == 422 and not ei.value.validation.valid

    too_long = PlanBuilder(sat, t0, t0 + 100 * H).build()
    assert "OUT_OF_HORIZON" in {x.code for x in sim.validate_plan(too_long).violations}

    with pytest.raises(SimAPIError) as ei:
        sim.satellite("nope")
    assert ei.value.status == 404


def test_plan_lifecycle_uplink_on_contact(sim):
    sat = "SD-002"
    c = next_uplink_contact(sim, sat)
    t0 = sim.now()
    start = c.aos + timedelta(seconds=90)  # uplink needs 60 s of contact first
    img_s, img_e = daylight_window(sim, sat, c.los + timedelta(minutes=2), c.los + 3 * H)
    plan = sim.submit_plan(
        PlanBuilder(sat, t0, img_e + H).downlink(c.station_id, start, c.los - timedelta(seconds=5))
        .image(img_s, img_e).build()
    )
    assert plan.status == m.PlanStatus.queued
    assert sim.satellite(sat).queued_plan_ids == [plan.id]

    # nothing reaches the satellite before the contact
    sim.step(until=c.aos - timedelta(seconds=10))
    assert sim.plan(plan.id).status == m.PlanStatus.queued
    assert sim.onboard_schedule(sat).activities == []

    sim.step(until=start + timedelta(seconds=30))
    p = sim.plan(plan.id)
    assert p.status == m.PlanStatus.active
    assert p.uplink_station_id is not None
    assert sim.satellite(sat).mode == m.SatelliteMode.downlink

    sim.step(until=img_s + timedelta(minutes=3))
    s = sim.satellite(sat)
    assert s.mode == m.SatelliteMode.imaging
    assert s.resources.storage_used_mb > 0
    # telemetry is ground knowledge only: it lags the truth while out of contact
    tlm = sim.telemetry(sat)
    assert tlm.received_at is not None and tlm.received_at < s.as_of

    sim.step(until=img_e + H + timedelta(minutes=1))
    p = sim.plan(plan.id)
    assert p.status == m.PlanStatus.completed
    imaging = [a for a in p.activities if a.type == "imaging"][0]
    assert imaging.status in (m.ActivityStatus.completed, m.ActivityStatus.partial)
    assert imaging.result["data_mb"] > 0
    imgs = sim.images(satellite_id=sat)
    assert imgs.total >= 1 and imgs.items[0].size_mb > 0 and len(imgs.items[0].centerline) > 2

    types = {e.type for e in sim.iter_events(satellite_id=sat)}
    assert {m.EventType.plan_queued, m.EventType.plan_uplinked, m.EventType.plan_activated,
            m.EventType.activity_started, m.EventType.image_captured, m.EventType.plan_completed} <= types


def test_activities_before_uplink_are_skipped(sim):
    sat = "SD-003"
    c = next_uplink_contact(sim, sat)
    t0 = sim.now()
    early_s, early_e = daylight_window(sim, sat, t0, c.aos - timedelta(minutes=2), minutes=2) \
        if (c.aos - t0) > timedelta(minutes=20) else (None, None)
    if early_s is None:
        pytest.skip("first contact too soon for this check")
    plan = sim.submit_plan(PlanBuilder(sat, t0, c.los + 2 * H).image(early_s, early_e).build())
    sim.step(until=c.los + timedelta(minutes=1))
    act = sim.plan(plan.id).activities[0]
    assert act.status == m.ActivityStatus.skipped
    assert "uplinked_after_start" in act.result["reasons"]


def test_new_plan_supersedes_old(sim):
    sat = "SD-004"
    c1 = next_uplink_contact(sim, sat)
    t0 = sim.now()
    later = t0 + 6 * H
    a = sim.submit_plan(PlanBuilder(sat, t0, t0 + 12 * H).image(later, later + timedelta(minutes=5)).build())
    b = sim.submit_plan(PlanBuilder(sat, t0 + 5 * H, t0 + 12 * H).build())
    sim.step(until=c1.aos + timedelta(minutes=3))
    assert sim.plan(a.id).status == m.PlanStatus.superseded
    assert sim.plan(a.id).superseded_by == b.id
    assert sim.plan(b.id).status == m.PlanStatus.uplinked


def test_cancel_only_queued(sim):
    t0 = sim.now()
    p = sim.submit_plan(PlanBuilder("SD-005", t0, t0 + 6 * H).build())
    assert sim.cancel_plan(p.id).status == m.PlanStatus.cancelled
    with pytest.raises(SimAPIError) as ei:
        sim.cancel_plan(p.id)
    assert ei.value.status == 409


def test_antenna_conflict():
    sim = inprocess_client(seed=0)  # full constellation: plenty of simultaneous passes
    t0 = sim.now()
    cs = sim.contacts(t0, t0 + 3 * H, station_ids=["troll"])  # single-antenna station
    pair = None
    for i, a in enumerate(cs):
        for b in cs[i + 1:]:
            if b.satellite_id != a.satellite_id and b.aos < a.los - timedelta(seconds=90) and a.aos < b.los:
                pair = (a, b)
                break
        if pair:
            break
    assert pair, "expected overlapping passes over troll"
    a, b = pair
    s = max(a.aos, b.aos) + timedelta(seconds=5)
    e = min(a.los, b.los) - timedelta(seconds=5)
    sim.submit_plan(PlanBuilder(a.satellite_id, t0, t0 + 6 * H).downlink("troll", s, e).build())
    with pytest.raises(PlanRejected) as ei:
        sim.submit_plan(PlanBuilder(b.satellite_id, t0, t0 + 6 * H).downlink("troll", s, e).build())
    assert ei.value.status == 409 and ei.value.code == "ANTENNA_CONFLICT"
    st = sim.ground_station("troll")
    assert len(st.reservations) == 1

    # within one batch, later plans see earlier plans' reservations
    sim2 = inprocess_client(seed=0)
    both = [PlanBuilder(x.satellite_id, t0, t0 + 6 * H).downlink("troll", s, e).build() for x in (a, b)]
    res = sim2.submit_plans(both, atomic=True)
    assert res.accepted == 0
    assert "ANTENNA_CONFLICT" in {v.code for v in res.items[1].validation.violations}
    res = sim2.submit_plans(both)
    assert (res.accepted, res.rejected) == (1, 1)


def test_request_fulfilled_end_to_end():
    sim = inprocess_client(seed=5, num_satellites=12)
    t0 = sim.now()
    # pick a daylit point under a satellite's ground track, then make it a request
    sat = "SD-001"
    c_up = next_uplink_contact(sim, sat)
    t0 = sim.now()
    s, e = daylight_window(sim, sat, c_up.los + timedelta(minutes=2), c_up.los + 6 * H, minutes=4)
    mid = sim.ephemeris(sat, s + (e - s) / 2, s + (e - s) / 2 + timedelta(seconds=1), step_s=1).points[0]
    req = sim.create_request(m.ImagingRequestCreate(
        target=m.GeoPoint(lat_deg=round(mid.position.lat_deg, 4), lon_deg=round(mid.position.lon_deg, 4)),
        priority=3, window_end=t0 + 24 * H))
    opps = sim.opportunities(c_up.los, t0 + 8 * H, request_ids=[req.id], satellite_ids=[sat])
    assert opps and opps[0].request_id == req.id and abs(opps[0].cross_track_km) < 16.25
    o = opps[0]

    assert c_up.los < o.start, "test setup expects an uplink pass before the opportunity"
    c_down = first_contact(sim, sat, after=o.end + timedelta(minutes=2))
    plan = (PlanBuilder(sat, t0, c_down.los + H)
            .image(o.start - timedelta(seconds=30), o.end + timedelta(seconds=30), request_ids=[req.id])
            .downlink(c_down.station_id, c_down.aos + timedelta(seconds=5), c_down.los - timedelta(seconds=5))
            .build())
    sim.submit_plan(plan)
    sim.step(until=c_down.los + timedelta(minutes=1))
    r = sim.request(req.id)
    assert r.status == m.RequestStatus.fulfilled, r
    assert r.image_ids
    mt = sim.metrics()
    assert mt.requests.fulfilled == 1 and mt.requests.fulfilled_priority == 3
    assert mt.images_downlinked >= 1 and mt.mean_latency_s > 0


def test_events_pagination(sim):
    sim.step(2 * 3600)
    all_ev = list(sim.iter_events(limit=7))
    assert [e.seq for e in all_ev] == list(range(1, len(all_ev) + 1))
    only = sim.events(types=[m.EventType.contact_start]).events
    assert only and all(e.type == m.EventType.contact_start for e in only)


def test_products_endpoints(sim):
    sim.step(1800)
    grid = sim.coverage(resolution_deg=2.0)
    assert (grid.rows, grid.cols) == (90, 180) and len(grid.last_imaged_s) == 90
    fc = sim.cloud_forecast([(10, 20), (-30, 140)], sim.now() + 6 * H)
    assert len(fc) == 2 and all(0 <= f.cloud_fraction <= 1 and f.sigma > 0 for f in fc)
    near = sim.cloud_forecast([(10, 20)], sim.now())[0]
    assert near.sigma < fc[0].sigma
    sats = sim.satellites()
    assert all(s.resources.battery_wh > 0 for s in sats)


def test_batch_submit_non_atomic_and_atomic(sim):
    t0 = sim.now()
    good = PlanBuilder("SD-006", t0, t0 + 3 * H).build()
    bad = PlanBuilder("SD-007", t0, t0 + 300 * H).build()
    res = sim.submit_plans([good, bad])
    assert (res.accepted, res.rejected) == (1, 1)
    res = sim.submit_plans([good, bad], atomic=True)
    assert (res.accepted, res.rejected) == (0, 2)


def test_openapi_contract_is_current():
    """api/openapi.json must be regenerated when models/endpoints change."""
    from planetsim.mock.server import create_app

    committed = Path(__file__).resolve().parents[2] / "api" / "openapi.json"
    assert json.loads(committed.read_text()) == create_app().openapi(), \
        "run: python scripts/export_openapi.py"
