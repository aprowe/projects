"""Mock simulation engine: just enough behaviour to exercise the API end-to-end.

Time-stepped (``step_s``, default 10 s). Each step:

1. propagate all satellites, compute station visibility and sunlight;
2. contacts: telemetry downlink + uplink of queued plans;
3. execute onboard activities (imaging fills storage, downlink drains it FIFO);
4. update power, safe mode, plan / request lifecycles.

The clock is lazy: when running, the engine catches up to
``rate * wall_elapsed`` on every API call instead of using a thread.
"""

from __future__ import annotations

import bisect
import copy
import math
import threading
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any

import numpy as np

from .. import models as m
from . import physics as ph
from .scenario import SCENARIOS, ScenarioDef

DEFAULT_EPOCH = datetime(2026, 1, 1, tzinfo=timezone.utc)


class ApiError(Exception):
    def __init__(self, status: int, code: str, message: str, details: Any = None):
        super().__init__(message)
        self.status, self.code, self.message, self.details = status, code, message, details


# --------------------------------------------------------------------------- #
# runtime records
# --------------------------------------------------------------------------- #


@dataclass
class ActRec:
    id: str
    plan_id: str
    sat: int
    type: str
    start: float
    end: float
    station: int | None = None
    request_ids: list[str] = field(default_factory=list)
    status: m.ActivityStatus = m.ActivityStatus.pending
    data_mb: float = 0.0
    reasons: set[str] = field(default_factory=set)
    image_ids: list[str] = field(default_factory=list)


@dataclass
class PlanRec:
    id: str
    sat: int
    start: float
    end: float
    label: str | None
    acts: list[ActRec]
    submitted_at: float
    status: m.PlanStatus = m.PlanStatus.queued
    uplinked_at: float | None = None
    uplink_station: int | None = None
    superseded_by: str | None = None
    warnings: list[m.Violation] = field(default_factory=list)


@dataclass
class ImageRec:
    id: str
    sat: int
    plan_id: str | None
    act_id: str | None
    start: float
    end: float
    size_mb: float = 0.0
    downlinked_mb: float = 0.0
    status: m.ImageStatus = m.ImageStatus.onboard
    downlinked_at: float | None = None
    station: int | None = None
    cloud_sum: float = 0.0
    cloud_n: int = 0
    centerline: list[tuple[float, float]] = field(default_factory=list)
    request_ids: list[str] = field(default_factory=list)


@dataclass
class ReqRec:
    id: str
    spec: m.ImagingRequestCreate
    created_at: float
    win_start: float
    win_end: float
    unit: np.ndarray
    status: m.RequestStatus = m.RequestStatus.open
    image_ids: list[str] = field(default_factory=list)
    captured_at: float | None = None
    fulfilled_at: float | None = None


@dataclass
class SatRec:
    id: str
    name: str
    flock: str
    spec: m.SpacecraftSpec
    storage_mb: float = 0.0
    battery_wh: float = 0.0
    health: m.HealthStatus = m.HealthStatus.nominal
    mode: m.SatelliteMode = m.SatelliteMode.idle
    onboard: list[ActRec] = field(default_factory=list)  # sorted by start
    queue: list[str] = field(default_factory=list)  # plan ids
    onboard_images: list[str] = field(default_factory=list)  # FIFO
    open_image: str | None = None
    last_contact: float | None = None
    telemetry: tuple | None = None  # (t, station, health, mode, resources, plan_id, completed_ids)
    current_act: ActRec | None = None
    storage_full_flag: bool = False
    battery_low_flag: bool = False
    safe_until: float = 0.0
    completed_act_ids: list[str] = field(default_factory=list)


# --------------------------------------------------------------------------- #
# engine
# --------------------------------------------------------------------------- #


class Engine:
    def __init__(self, reset: m.SimReset | None = None):
        self.lock = threading.RLock()
        self.reset(reset or m.SimReset())

    # ------------------------------------------------------------------ #
    # setup
    # ------------------------------------------------------------------ #

    def reset(self, cfg: m.SimReset) -> None:
        with self.lock:
            if cfg.scenario not in SCENARIOS:
                raise ApiError(404, "UNKNOWN_SCENARIO", f"unknown scenario {cfg.scenario!r}", list(SCENARIOS))
            sc: ScenarioDef = SCENARIOS[cfg.scenario]
            self.scn, self.cfg = sc, cfg
            self.epoch = cfg.epoch or DEFAULT_EPOCH
            self.epoch_unix = self.epoch.timestamp()
            self.rng = np.random.default_rng(cfg.seed)
            self.step_s = sc.step_s
            self.t = 0.0
            self.running, self.rate = False, 60.0
            self._wall = time.monotonic()
            self._next_id = 1
            self._seq = 0
            # uncertainty hooks (off for the live sim; set per Monte Carlo rollout sample)
            self.cloud_mode = m.CloudModel.truth
            self.cloud_issue_t = 0.0
            self.cloud_err: tuple[np.ndarray, ...] | None = None
            self.mc_rng: np.random.Generator | None = None
            self.pass_fail_p = self.uplink_fail_p = self.anomaly_rate = 0.0
            self.anomaly_recovery_s = 0.0
            self.failed_pairs: set[tuple[int, int]] = set()
            self.uplink_failed_pairs: set[tuple[int, int]] = set()
            self.sim_mask: np.ndarray | None = None  # simulate only these satellites

            # satellites
            a, inc, raan0, u0 = [], [], [], []
            self.sats: list[SatRec] = []
            self.flocks: list[m.FlockInfo] = []
            ra_sun = ph.sun_ra_rad(self.epoch_unix)
            for plane in sc.planes:
                ids = []
                ak = ph.RE + plane.altitude_km
                i = ph.sso_inclination_rad(ak)
                ltan = (plane.ltdn_hours + 12.0) % 24.0
                raan = ra_sun + np.deg2rad((ltan - 12.0) * 15.0)
                phase0 = self.rng.uniform(0, 2 * np.pi)
                for k in range(plane.count):
                    sid = f"SD-{len(self.sats) + 1:03d}"
                    ids.append(sid)
                    self.sats.append(SatRec(sid, f"{plane.name}-{k + 1:02d}", plane.name, sc.spec,
                                            battery_wh=sc.spec.battery_capacity_wh * 0.8))
                    a.append(ak)
                    inc.append(i)
                    raan0.append(raan)
                    u0.append(phase0 + 2 * np.pi * k / plane.count + self.rng.normal(0, 0.002))
                self.flocks.append(m.FlockInfo(name=plane.name, satellite_ids=ids, altitude_km=plane.altitude_km,
                                               inclination_deg=float(np.rad2deg(i)), ltdn_hours=plane.ltdn_hours))
            if cfg.num_satellites is not None:
                keep = cfg.num_satellites
                self.sats = self.sats[:keep]
                a, inc, raan0, u0 = a[:keep], inc[:keep], raan0[:keep], u0[:keep]
                kept = {s.id for s in self.sats}
                for f in self.flocks:
                    f.satellite_ids = [s for s in f.satellite_ids if s in kept]
                self.flocks = [f for f in self.flocks if f.satellite_ids]
            self.orbits = ph.Orbits(a, inc, raan0, u0)
            self.sat_index = {s.id: i for i, s in enumerate(self.sats)}

            # stations
            self.stations = sc.stations
            self.st_index = {s.id: i for i, s in enumerate(self.stations)}
            self.st_ecef = ph.ecef_from_geodetic(
                np.array([s.lat for s in self.stations]), np.array([s.lon for s in self.stations]),
                np.array([s.alt_m / 1000 for s in self.stations]))
            self.st_mask = np.array([s.min_el for s in self.stations])
            self.contact_elapsed = np.zeros((len(self.sats), len(self.stations)))
            self.antenna_busy_s = 0.0
            # reservations[station] -> sorted list of (start, end, ActRec)
            self.reservations: list[list[ActRec]] = [[] for _ in self.stations]

            self.plans: dict[str, PlanRec] = {}
            self.live_plans: set[str] = set()
            self.images: dict[str, ImageRec] = {}
            self.image_order: list[str] = []
            self.requests: dict[str, ReqRec] = {}
            self.events: list[m.Event] = []
            self.cov_res = 0.5
            self.coverage = np.full((int(180 / self.cov_res), int(360 / self.cov_res)), np.nan)
            self._prev_ground: np.ndarray | None = None

            self._emit(m.EventType.sim_reset, data={"scenario": sc.name, "seed": cfg.seed})
            for _ in range(cfg.random_requests):
                lat = float(np.rad2deg(np.arcsin(self.rng.uniform(-0.94, 0.94))))  # ~ +-70 deg, area-uniform
                lon = float(self.rng.uniform(-180, 180))
                self.create_request(m.ImagingRequestCreate(
                    target=m.GeoPoint(lat_deg=round(lat, 4), lon_deg=round(lon, 4)),
                    priority=float(self.rng.choice([1, 1, 1, 2, 5])),
                    window_end=self.to_dt(self.rng.uniform(24, 72) * 3600),
                    max_cloud_fraction=float(self.rng.choice([1.0, 0.5, 0.3])),
                ))
            self._sample(0.0)

    # ------------------------------------------------------------------ #
    # time helpers
    # ------------------------------------------------------------------ #

    def to_dt(self, t: float) -> datetime:
        return self.epoch + timedelta(seconds=float(t))

    def to_s(self, dt: datetime) -> float:
        return (dt - self.epoch).total_seconds()

    def _id(self, prefix: str) -> str:
        self._next_id += 1
        return f"{prefix}-{self._next_id - 1:06d}"

    def _emit(self, typ: m.EventType, *, t: float | None = None, **kw: Any) -> None:
        self._seq += 1
        self.events.append(m.Event(seq=self._seq, time=self.to_dt(self.t if t is None else t), type=typ, **kw))

    # ------------------------------------------------------------------ #
    # forking (for rollouts)
    # ------------------------------------------------------------------ #

    def __getstate__(self) -> dict:
        state = self.__dict__.copy()
        del state["lock"]
        return state

    def __setstate__(self, state: dict) -> None:
        self.__dict__.update(state)
        self.lock = threading.RLock()

    def fork(self) -> Engine:
        """Independent, paused copy of the current state.

        History that cannot affect the future (events, finished plans, downlinked
        images) is left out to keep the copy cheap.
        """
        with self.lock:
            saved = self.events, self.images, self.image_order, self.plans
            try:
                self.events = []
                self.images = {k: v for k, v in saved[1].items() if v.status == m.ImageStatus.onboard}
                self.image_order = [k for k in saved[2] if k in self.images]
                live = (m.PlanStatus.queued, m.PlanStatus.uplinked, m.PlanStatus.active)
                self.plans = {k: p for k, p in saved[3].items() if p.status in live}
                clone = copy.deepcopy(self)
            finally:
                self.events, self.images, self.image_order, self.plans = saved
        clone.running = False
        return clone

    def configure_uncertainty(self, mc: m.MonteCarloConfig | None, cloud_mode: m.CloudModel, seed: int | None) -> None:
        """Switch this (forked) engine to rollout mode."""
        self.cloud_mode, self.cloud_issue_t = cloud_mode, self.t
        rng = np.random.default_rng(seed)
        if cloud_mode == m.CloudModel.sampled:
            # smooth random forecast-error field: a few random plane waves in (lat, lon, t)
            k = 6
            self.cloud_err = (rng.integers(1, 6, k), rng.integers(1, 6, k), rng.normal(0, 0.1, k),
                              rng.uniform(0, 2 * np.pi, k), rng.normal(0, 1, k) / np.sqrt(k / 2))
        if mc is not None:
            self.mc_rng = rng
            self.pass_fail_p, self.uplink_fail_p = mc.pass_failure_prob, mc.uplink_failure_prob
            self.anomaly_rate, self.anomaly_recovery_s = mc.anomaly_rate_per_sat_day, mc.anomaly_recovery_s

    def sync(self) -> None:
        """Catch the sim clock up with wall time when running."""
        with self.lock:
            now = time.monotonic()
            if self.running:
                self.advance_to(self.t + (now - self._wall) * self.rate)
            self._wall = now

    # ------------------------------------------------------------------ #
    # geometry sampling
    # ------------------------------------------------------------------ #

    def _sample(self, t: float) -> None:
        unix = self.epoch_unix + t
        self.r_eci, self.v_eci = self.orbits.eci(t)
        g = ph.gmst_rad(unix)
        self.r_ecef = ph.rot_z(self.r_eci, g)
        self.lat, self.lon, self.alt = ph.geodetic_from_ecef(self.r_ecef)
        self.sun_eci = ph.sun_eci_unit(unix)
        self.sun_ecef = ph.rot_z(self.sun_eci, g)
        self.sunlit = ph.in_sunlight(self.r_eci, self.sun_eci)
        self.ground_unit = ph.unit(self.r_ecef)
        self.sun_el = ph.sun_elevation_deg(self.r_ecef, self.sun_ecef)
        # (nsat, nst)
        self.elev = ph.elevation_deg(self.r_ecef[:, None, :], self.st_ecef[None, :, :])
        self.in_view = self.elev >= self.st_mask[None, :]

    def cloud_truth(self, lat, lon, t):
        """Smooth, slowly-moving synthetic cloud field in [0, 1]."""
        lat, lon = np.deg2rad(lat), np.deg2rad(lon)
        h = t / 3600.0
        s = (0.55 + 0.25 * np.sin(3 * lon + 0.05 * h + self.cfg.seed) * np.cos(2 * lat)
             + 0.15 * np.sin(5 * lat - 0.08 * h) + 0.1 * np.cos(7 * lon + 4 * lat + 0.11 * h))
        return np.clip(s, 0, 1)

    def _forecast_sigma(self, t, issued):
        lead_h = np.maximum(0.0, (np.asarray(t) - issued) / 3600.0)
        return np.minimum(0.4, 0.03 + 0.15 * lead_h / 24.0)

    def _forecast_mean(self, lat, lon, t, issued):
        """Forecast issued at ``issued``: truth plus a deterministic error scaled by lead time."""
        lat, lon, t = np.asarray(lat, float), np.asarray(lon, float), np.asarray(t, float)
        h = np.sin(lat * 12.9898 + lon * 78.233 + t * 1e-4 + self.cfg.seed) * 43758.5453
        noise = (h - np.floor(h) - 0.5) * 2 * self._forecast_sigma(t, issued)
        return np.clip(self.cloud_truth(lat, lon, t) + noise, 0, 1)

    def cloud_actual(self, lat, lon, t):
        """Cloud fraction that imaging actually sees (truth live; model-dependent in rollouts)."""
        if self.cloud_mode == m.CloudModel.truth:
            return self.cloud_truth(lat, lon, t)
        mean = self._forecast_mean(lat, lon, t, self.cloud_issue_t)
        if self.cloud_mode == m.CloudModel.forecast or self.cloud_err is None:
            return mean
        fa, fo, ft, ph0, amp = self.cloud_err
        la, lo = np.deg2rad(np.asarray(lat, float))[..., None], np.deg2rad(np.asarray(lon, float))[..., None]
        err = np.sum(amp * np.sin(fa * la + fo * lo + ft * t / 3600.0 + ph0), axis=-1)
        return np.clip(mean + err * self._forecast_sigma(t, self.cloud_issue_t), 0, 1)

    # ------------------------------------------------------------------ #
    # main loop
    # ------------------------------------------------------------------ #

    def advance_to(self, t_target: float) -> None:
        with self.lock:
            while self.t + 1e-9 < t_target:
                dt = min(self.step_s, t_target - self.t)
                self._step(dt)

    def _step(self, dt: float) -> None:
        t0, t1 = self.t, self.t + dt
        prev_in_view = self.in_view.copy()
        prev_ground = self.ground_unit.copy()
        prev_lonlat = (self.lat.copy(), self.lon.copy())

        self._expire_queued(t0)
        self._contacts(t0, dt, prev_in_view)

        self._imaging_now: list[tuple[int, str]] = []
        for i, sat in enumerate(self.sats):
            if self.sim_mask is None or self.sim_mask[i]:
                self._run_satellite(i, sat, t0, t1, prev_lonlat)

        self.t = t1
        self._sample(t1)
        self._imaging_geometry(prev_ground, t1)
        new_contacts = self.in_view & ~prev_in_view
        lost_contacts = prev_in_view & ~self.in_view
        for i, j in zip(*np.nonzero(new_contacts)):
            failed = self.mc_rng is not None and self.pass_fail_p > 0 and self.mc_rng.random() < self.pass_fail_p
            if failed:
                self.failed_pairs.add((i, j))
            self._emit(m.EventType.contact_start, satellite_id=self.sats[i].id, station_id=self.stations[j].id,
                       data={"failed": True} if failed else {})
        for i, j in zip(*np.nonzero(lost_contacts)):
            self._emit(m.EventType.contact_end, satellite_id=self.sats[i].id, station_id=self.stations[j].id)
            self.contact_elapsed[i, j] = 0.0
            self.failed_pairs.discard((i, j))
            self.uplink_failed_pairs.discard((i, j))
        self._update_plans(t1)
        self._update_requests(t1)

    # ------------------------------------------------------------------ #
    # contacts: telemetry + uplink
    # ------------------------------------------------------------------ #

    def _antennas_busy(self, j: int, t: float) -> set[int]:
        return {a.sat for a in self.reservations[j] if a.start <= t < a.end
                and a.status in (m.ActivityStatus.scheduled, m.ActivityStatus.executing)}

    def _contacts(self, t: float, dt: float, in_view: np.ndarray) -> None:
        rules = self.scn.rules
        self.contact_elapsed[in_view] += dt
        for i, j in zip(*np.nonzero(in_view)):
            if (self.sim_mask is not None and not self.sim_mask[i]) or (i, j) in self.failed_pairs:
                continue
            sat, st = self.sats[i], self.stations[j]
            busy = self._antennas_busy(j, t)
            own_downlink = i in busy
            if not own_downlink and len(busy) >= st.antennas:
                continue  # no free antenna to talk to this satellite
            sat.last_contact = t
            if rules.telemetry_on_contact:
                sat.telemetry = (t, j, sat.health, sat.mode, self._resources_tuple(sat),
                                 sat.current_act.plan_id if sat.current_act else None,
                                 tuple(sat.completed_act_ids[-50:]))
                if self.contact_elapsed[i, j] <= dt:
                    self._emit(m.EventType.telemetry_received, satellite_id=sat.id, station_id=st.id)
            if sat.queue and (own_downlink or rules.opportunistic_uplink) \
                    and self.contact_elapsed[i, j] >= rules.uplink_duration_s \
                    and (i, j) not in self.uplink_failed_pairs:
                if self.mc_rng is not None and self.uplink_fail_p > 0 and self.mc_rng.random() < self.uplink_fail_p:
                    self.uplink_failed_pairs.add((i, j))  # this pass is wasted; retry at the next one
                    continue
                for pid in list(sat.queue):
                    self._uplink(self.plans[pid], t + dt, j)
                sat.queue.clear()

    def _resources_tuple(self, sat: SatRec) -> tuple:
        return (sat.storage_mb, sat.battery_wh, len(sat.onboard_images))

    def _uplink(self, plan: PlanRec, t: float, station: int) -> None:
        sat = self.sats[plan.sat]
        plan.status, plan.uplinked_at, plan.uplink_station = m.PlanStatus.uplinked, t, station
        self.live_plans.add(plan.id)
        # supersede onboard activities that start at/after the new plan's start
        keep, dropped = [], []
        for a in sat.onboard:
            (dropped if a.start >= plan.start and a.status == m.ActivityStatus.scheduled else keep).append(a)
        for a in dropped:
            a.status = m.ActivityStatus.superseded
            self._release(a)
        touched = {a.plan_id for a in dropped}
        for a in plan.acts:
            if a.start < t:
                a.status = m.ActivityStatus.skipped
                a.reasons.add("uplinked_after_start")
                self._release(a)
            else:
                a.status = m.ActivityStatus.scheduled
                keep.append(a)
        keep.sort(key=lambda a: a.start)
        sat.onboard = keep
        self._emit(m.EventType.plan_uplinked, satellite_id=sat.id, plan_id=plan.id,
                   station_id=self.stations[station].id)
        for pid in touched:
            old = self.plans[pid]
            if not any(a.status in (m.ActivityStatus.scheduled, m.ActivityStatus.executing) for a in old.acts):
                old.status, old.superseded_by = m.PlanStatus.superseded, plan.id
                self._emit(m.EventType.plan_superseded, satellite_id=sat.id, plan_id=old.id, data={"by": plan.id})

    def _release(self, a: ActRec) -> None:
        if a.station is not None and a in self.reservations[a.station]:
            self.reservations[a.station].remove(a)

    def _expire_queued(self, t: float) -> None:
        for sat in self.sats:
            for pid in list(sat.queue):
                p = self.plans[pid]
                if p.end <= t:
                    sat.queue.remove(pid)
                    p.status = m.PlanStatus.expired
                    for a in p.acts:
                        a.status = m.ActivityStatus.skipped
                        a.reasons.add("never_uplinked")
                        self._release(a)
                    self._emit(m.EventType.plan_expired, satellite_id=sat.id, plan_id=pid)

    # ------------------------------------------------------------------ #
    # satellite execution
    # ------------------------------------------------------------------ #

    def _run_satellite(self, i: int, sat: SatRec, t0: float, t1: float, lonlat_prev) -> None:
        spec = sat.spec
        dt = t1 - t0
        rules = self.scn.rules

        # finish activities that ended
        while sat.onboard and sat.onboard[0].end <= t0:
            self._finish_activity(sat, sat.onboard.pop(0), t0)

        # safe mode transitions
        frac = sat.battery_wh / spec.battery_capacity_wh
        anomaly = (self.mc_rng is not None and self.anomaly_rate > 0 and sat.health != m.HealthStatus.safe
                   and self.mc_rng.random() < self.anomaly_rate * dt / 86400.0)
        if sat.health != m.HealthStatus.safe and (anomaly or frac < rules.safe_mode_enter_battery_frac):
            sat.health, sat.mode = m.HealthStatus.safe, m.SatelliteMode.safe
            sat.safe_until = t0 + (self.anomaly_recovery_s if anomaly else 0.0)
            self._close_image(sat, t0)
            self._emit(m.EventType.safe_mode_entered, satellite_id=sat.id,
                       data={"battery_frac": frac, "cause": "anomaly" if anomaly else "low_battery"})
        elif sat.health == m.HealthStatus.safe and frac >= rules.safe_mode_exit_battery_frac and t0 >= sat.safe_until:
            sat.health = m.HealthStatus.nominal
            sat.mode = m.SatelliteMode.idle
            self._emit(m.EventType.safe_mode_exited, satellite_id=sat.id)

        act = next((a for a in sat.onboard if a.start < t1 and a.end > t0), None)
        power = spec.idle_power_w
        if act is not sat.current_act:
            if sat.current_act is not None and sat.current_act.type == "imaging":
                self._close_image(sat, t0)
            sat.current_act = act
        if act is not None and act.status == m.ActivityStatus.scheduled:
            act.status = m.ActivityStatus.executing
            self._emit(m.EventType.activity_started, t=max(act.start, t0), satellite_id=sat.id,
                       plan_id=act.plan_id, activity_id=act.id, data={"type": act.type})

        if sat.health == m.HealthStatus.safe:
            if act is not None:
                act.reasons.add("safe_mode")
        elif act is None:
            sat.mode = m.SatelliteMode.idle
        else:
            overlap = min(act.end, t1) - max(act.start, t0)
            if act.type == "imaging":
                sat.mode = m.SatelliteMode.imaging
                power = spec.imaging_power_w
                if self.sun_el[i] < spec.min_sun_elevation_deg:
                    act.reasons.add("sun_too_low")
                    self._close_image(sat, t0)
                else:
                    self._image(i, sat, act, t0, t1, overlap, lonlat_prev)
            else:
                sat.mode = m.SatelliteMode.downlink
                power = spec.downlink_power_w
                j = act.station
                if not self.in_view[i, j]:
                    act.reasons.add("station_not_in_view")
                elif (i, j) in self.failed_pairs:
                    act.reasons.add("pass_failed")
                else:
                    rate = spec.downlink_rate_mbps
                    if self.stations[j].max_downlink_rate_mbps:
                        rate = min(rate, self.stations[j].max_downlink_rate_mbps)
                    self._downlink(sat, act, j, rate / 8.0 * overlap, t1)
                    self.antenna_busy_s += overlap

        # power
        gen = spec.solar_power_w if self.sunlit[i] else 0.0
        sat.battery_wh = min(max(sat.battery_wh + (gen - power) * dt / 3600.0, 0.0), spec.battery_capacity_wh)
        frac = sat.battery_wh / spec.battery_capacity_wh
        if frac < 0.2 and not sat.battery_low_flag:
            sat.battery_low_flag = True
            self._emit(m.EventType.battery_low, satellite_id=sat.id, data={"battery_frac": frac})
        elif frac > 0.3:
            sat.battery_low_flag = False

    def _finish_activity(self, sat: SatRec, a: ActRec, t: float) -> None:
        if a.type == "imaging":
            self._close_image(sat, t)
        if a.data_mb <= 0:
            a.status = m.ActivityStatus.failed
        elif a.reasons:
            a.status = m.ActivityStatus.partial
        else:
            a.status = m.ActivityStatus.completed
        self._release(a)
        sat.completed_act_ids.append(a.id)
        if sat.current_act is a:
            sat.current_act = None
        self._emit(m.EventType.activity_ended, t=a.end, satellite_id=sat.id, plan_id=a.plan_id, activity_id=a.id,
                   data={"type": a.type, "status": a.status.value, "data_mb": round(a.data_mb, 3),
                         "reasons": sorted(a.reasons)})

    def _image(self, i, sat, act, t0, t1, overlap, lonlat_prev) -> None:
        spec = sat.spec
        free = spec.storage_capacity_mb - sat.storage_mb
        want = spec.imaging_data_rate_mbps / 8.0 * overlap
        got = min(want, free)
        if got < want:
            act.reasons.add("storage_full")
            if not sat.storage_full_flag:
                sat.storage_full_flag = True
                self._emit(m.EventType.storage_full, satellite_id=sat.id)
        if got <= 0:
            self._close_image(sat, t0)
            return
        if sat.open_image is None:
            img = ImageRec(self._id("img"), i, act.plan_id, act.id, start=max(act.start, t0), end=t1)
            img.centerline.append((round(float(lonlat_prev[1][i]), 4), round(float(lonlat_prev[0][i]), 4)))
            self.images[img.id] = img
            self.image_order.append(img.id)
            sat.open_image = img.id
            sat.onboard_images.append(img.id)
            act.image_ids.append(img.id)
        img = self.images[sat.open_image]
        img.end = min(act.end, t1)
        img.size_mb += got
        sat.storage_mb += got
        act.data_mb += got
        self._imaging_now.append((i, img.id))  # ground-track bookkeeping happens in _imaging_geometry

    def _close_image(self, sat: SatRec, t: float) -> None:
        if sat.open_image is None:
            return
        img = self.images[sat.open_image]
        sat.open_image = None
        self._emit(m.EventType.image_captured, t=img.end, satellite_id=sat.id, image_id=img.id, plan_id=img.plan_id,
                   activity_id=img.act_id, data={"size_mb": round(img.size_mb, 3), "request_ids": img.request_ids})

    def _downlink(self, sat: SatRec, act: ActRec, j: int, budget_mb: float, t1: float) -> None:
        while budget_mb > 1e-9 and sat.onboard_images:
            img = self.images[sat.onboard_images[0]]
            remaining = img.size_mb - img.downlinked_mb
            if img.id == sat.open_image and remaining <= 1e-9:
                break
            take = min(remaining, budget_mb)
            img.downlinked_mb += take
            sat.storage_mb = max(0.0, sat.storage_mb - take)
            act.data_mb += take
            budget_mb -= take
            if img.downlinked_mb >= img.size_mb - 1e-6 and img.id != sat.open_image:
                img.status, img.downlinked_at, img.station = m.ImageStatus.downlinked, t1, j
                sat.onboard_images.pop(0)
                self._emit(m.EventType.image_downlinked, t=t1, satellite_id=sat.id, station_id=self.stations[j].id,
                           image_id=img.id, data={"latency_s": round(t1 - img.end, 1)})
                for rid in img.request_ids:
                    r = self.requests[rid]
                    if r.status == m.RequestStatus.captured and t1 <= r.win_end:
                        r.status, r.fulfilled_at = m.RequestStatus.fulfilled, t1
                        self._emit(m.EventType.request_fulfilled, t=t1, request_id=rid, image_id=img.id,
                                   satellite_id=sat.id)
            elif img.id == sat.open_image:
                break
        if sat.storage_mb < sat.spec.storage_capacity_mb * 0.99:
            sat.storage_full_flag = False

    # ------------------------------------------------------------------ #
    # coverage & requests
    # ------------------------------------------------------------------ #

    def _imaging_geometry(self, g_prev: np.ndarray, t: float) -> None:
        """Centerline, clouds, coverage and request hits for every satellite that imaged this step."""
        if not self._imaging_now:
            return
        idx = np.array([i for i, _ in self._imaging_now])
        imgs = [self.images[k] for _, k in self._imaging_now]
        g0, g1 = g_prev[idx], self.ground_unit[idx]
        lat1, lon1 = self.lat[idx], self.lon[idx]
        clouds = self.cloud_actual(lat1, lon1, t)
        for k, img in enumerate(imgs):
            img.centerline.append((round(float(lon1[k]), 4), round(float(lat1[k]), 4)))
            img.cloud_sum += float(clouds[k])
            img.cloud_n += 1

        # coverage: mark cells along each segment's centerline
        f = np.linspace(0, 1, 8)[None, :, None]
        p = ph.unit(g0[:, None, :] * (1 - f) + g1[:, None, :] * f).reshape(-1, 3)
        lat = np.rad2deg(np.arcsin(np.clip(p[:, 2], -1, 1)))
        lon = np.rad2deg(np.arctan2(p[:, 1], p[:, 0]))
        R, C = self.coverage.shape
        r = np.minimum(((90 - lat) / self.cov_res).astype(int), R - 1)
        c = np.minimum(((lon + 180) / self.cov_res).astype(int), C - 1)
        self.coverage[r, c] = t

        # requests whose target lies inside the swath of a segment
        reqs = [q for q in self.requests.values()
                if q.status in (m.RequestStatus.open, m.RequestStatus.captured) and q.win_start <= t <= q.win_end]
        if not reqs:
            return
        U = np.array([q.unit for q in reqs])
        n = ph.unit(np.cross(g0, g1))
        xt = np.arcsin(np.clip(U @ n.T, -1, 1)) * ph.RE  # (K, M)
        # (g0 x u).n = u.(n x g0) and (u x g1).n = u.(g1 x n)
        between = (U @ np.cross(n, g0).T >= 0) & (U @ np.cross(g1, n).T >= 0)
        half = np.array([self.sats[i].spec.swath_km / 2 for i in idx])
        for k, j in zip(*np.nonzero(between & (np.abs(xt) <= half[None, :]))):
            q, img = reqs[k], imgs[j]
            if q.id in img.request_ids:
                continue
            cloud = float(self.cloud_actual(q.spec.target.lat_deg, q.spec.target.lon_deg, t))
            if cloud > q.spec.max_cloud_fraction:
                continue
            img.request_ids.append(q.id)
            q.image_ids.append(img.id)
            if q.status == m.RequestStatus.open:
                q.status, q.captured_at = m.RequestStatus.captured, t
                self._emit(m.EventType.request_captured, t=t, request_id=q.id, image_id=img.id,
                           satellite_id=self.sats[img.sat].id, data={"cloud_fraction": round(cloud, 3)})

    def _update_requests(self, t: float) -> None:
        for r in self.requests.values():
            if r.status in (m.RequestStatus.open, m.RequestStatus.captured) and t > r.win_end:
                r.status = m.RequestStatus.expired
                self._emit(m.EventType.request_expired, t=t, request_id=r.id)

    def _update_plans(self, t: float) -> None:
        for p in [self.plans[k] for k in self.live_plans]:
            if p.status not in (m.PlanStatus.uplinked, m.PlanStatus.active):
                self.live_plans.discard(p.id)
                continue
            if p.status == m.PlanStatus.uplinked and t >= p.start:
                p.status = m.PlanStatus.active
                self._emit(m.EventType.plan_activated, t=max(p.start, p.uplinked_at or 0), satellite_id=self.sats[p.sat].id, plan_id=p.id)
            if p.status == m.PlanStatus.active and t >= p.end:
                p.status = m.PlanStatus.completed
                self.live_plans.discard(p.id)
                self._emit(m.EventType.plan_completed, t=p.end, satellite_id=self.sats[p.sat].id, plan_id=p.id)

    # ------------------------------------------------------------------ #
    # plan validation / submission
    # ------------------------------------------------------------------ #

    def _sat(self, sid: str) -> int:
        if sid not in self.sat_index:
            raise ApiError(404, "NOT_FOUND", f"unknown satellite {sid!r}")
        return self.sat_index[sid]

    def validate(self, sub: m.PlanSubmission, pending: list[ActRec] = ()) -> tuple[m.PlanValidation, list[ActRec]]:
        """Check a plan. ``pending`` = reservations from earlier plans in the same batch."""
        V = m.Violation
        rules, viol, warn = self.scn.rules, [], []
        if sub.satellite_id not in self.sat_index:
            return m.PlanValidation(valid=False, violations=[V(code="UNKNOWN_SATELLITE", message=sub.satellite_id)]), []
        i = self.sat_index[sub.satellite_id]
        spec = self.sats[i].spec
        ps, pe = self.to_s(sub.start), self.to_s(sub.end)
        if pe <= ps:
            viol.append(V(code="BAD_INTERVAL", message="plan end must be after start"))
        if pe - ps > rules.plan_max_horizon_s:
            viol.append(V(code="OUT_OF_HORIZON", message=f"plan longer than {rules.plan_max_horizon_s} s"))
        if pe <= self.t:
            viol.append(V(code="IN_PAST", message="plan ends before current sim time"))
        if len(sub.activities) > rules.plan_max_activities:
            viol.append(V(code="TOO_MANY_ACTIVITIES", message=f"max {rules.plan_max_activities}"))

        acts = sorted(enumerate(sub.activities), key=lambda x: x[1].start)
        recs: list[ActRec] = []
        prev = None
        for k, a in acts:
            s, e = self.to_s(a.start), self.to_s(a.end)
            if e <= s:
                viol.append(V(code="BAD_INTERVAL", message="activity end must be after start", activity_index=k))
            if s < ps or e > pe:
                viol.append(V(code="OUTSIDE_PLAN", message="activity outside plan [start, end)", activity_index=k))
            if s < self.t:
                warn.append(V(code="STARTS_IN_PAST", message="activity starts before sim time; it will be skipped",
                              activity_index=k))
            if prev is not None:
                pk, pa, pe_ = prev
                gap = s - pe_
                need = 0.0 if (pa.type == a.type == "imaging") else spec.transition_time_s
                if gap < 0:
                    viol.append(V(code="OVERLAP", message=f"overlaps activity {pk}", activity_index=k))
                elif gap < need:
                    viol.append(V(code="TRANSITION_TIME", message=f"needs {need:.0f} s after activity {pk}, has {gap:.0f}",
                                  activity_index=k))
            prev = (k, a, e)
            rec = ActRec(id="", plan_id="", sat=i, type=a.type, start=s, end=e)
            if isinstance(a, m.DownlinkActivity):
                if a.station_id not in self.st_index:
                    viol.append(V(code="UNKNOWN_STATION", message=a.station_id, activity_index=k))
                else:
                    j = self.st_index[a.station_id]
                    rec.station = j
                    if not self._in_contact_whole(i, j, s, e):
                        viol.append(V(code="NOT_IN_CONTACT",
                                      message=f"{a.station_id} not above {self.stations[j].min_el} deg for the whole interval",
                                      activity_index=k))
                    conflicts = self._antenna_conflicts(j, i, s, e, pending)
                    if conflicts:
                        viol.append(V(code="ANTENNA_CONFLICT",
                                      message=f"all {self.stations[j].antennas} antennas at {a.station_id} reserved: "
                                              + ", ".join(sorted({self.sats[c.sat].id for c in conflicts})),
                                      activity_index=k))
            else:
                rec.request_ids = list(a.request_ids)
            recs.append(rec)
        return m.PlanValidation(valid=not viol, violations=viol, warnings=warn), recs

    def _in_contact_whole(self, i: int, j: int, s: float, e: float) -> bool:
        ts = np.arange(s, e, 5.0)
        ts = np.append(ts, e - 1e-3)
        r, _ = self.orbits.eci(ts, idx=[i])
        ecef = ph.rot_z(r[0], ph.gmst_rad(self.epoch_unix + ts))
        el = ph.elevation_deg(ecef, self.st_ecef[j])
        return bool(np.all(el >= self.stations[j].min_el - 0.25))

    def _antenna_conflicts(self, j: int, sat: int, s: float, e: float, pending: list[ActRec] = ()) -> list[ActRec]:
        """Other satellites' reservations at station j that fill every antenna somewhere in [s, e).

        Reservations by the same satellite are ignored: a new plan supersedes the old one.
        """
        others = [a for a in [*self.reservations[j], *(p for p in pending if p.station == j)]
                  if a.start < e and a.end > s and a.sat != sat]
        if not others:
            return []
        pts = sorted({max(s, a.start) for a in others} | {s})
        worst: list[ActRec] = []
        for p in pts:
            live = [a for a in others if a.start <= p < a.end]
            if len(live) > len(worst):
                worst = live
        return worst if len(worst) >= self.stations[j].antennas else []

    def submit(self, sub: m.PlanSubmission) -> PlanRec:
        with self.lock:
            val, recs = self.validate(sub)
            if not val.valid:
                code = "ANTENNA_CONFLICT" if all(v.code == "ANTENNA_CONFLICT" for v in val.violations) else "PLAN_INVALID"
                raise ApiError(409 if code == "ANTENNA_CONFLICT" else 422, code, "plan rejected",
                               val.model_dump(mode="json"))
            return self._accept(sub, val, recs)

    def _accept(self, sub: m.PlanSubmission, val: m.PlanValidation, recs: list[ActRec]) -> PlanRec:
        i = self.sat_index[sub.satellite_id]
        pid = self._id("plan")
        for r in recs:
            r.id, r.plan_id = self._id("act"), pid
        plan = PlanRec(pid, i, self.to_s(sub.start), self.to_s(sub.end), sub.label, recs, self.t,
                       warnings=val.warnings)
        self.plans[pid] = plan
        for r in recs:
            if r.station is not None:
                bisect.insort(self.reservations[r.station], r, key=lambda a: a.start)
        self.sats[i].queue.append(pid)
        self._emit(m.EventType.plan_queued, satellite_id=sub.satellite_id, plan_id=pid,
                   data={"activities": len(recs)})
        return plan

    def submit_batch(self, batch: m.PlanBatchSubmission) -> m.PlanBatchResult:
        with self.lock:
            # validate in order; each valid plan's reservations constrain the plans after it
            checked, pending = [], []
            for sub in batch.plans:
                v, recs = self.validate(sub, pending)
                checked.append((v, recs))
                if v.valid:
                    pending.extend(r for r in recs if r.station is not None)
            if batch.atomic and not all(v.valid for v, _ in checked):
                items = [m.PlanBatchItem(index=k, validation=v) for k, (v, _) in enumerate(checked)]
                return m.PlanBatchResult(accepted=0, rejected=len(items), items=items)
            items, accepted = [], 0
            for k, (sub, (v, recs)) in enumerate(zip(batch.plans, checked)):
                if v.valid:
                    plan = self._accept(sub, v, recs)
                    accepted += 1
                    items.append(m.PlanBatchItem(index=k, plan=self.plan_model(plan), validation=v))
                else:
                    items.append(m.PlanBatchItem(index=k, validation=v))
            return m.PlanBatchResult(accepted=accepted, rejected=len(items) - accepted, items=items)

    def cancel(self, pid: str) -> PlanRec:
        p = self.get_plan(pid)
        if p.status != m.PlanStatus.queued:
            raise ApiError(409, "NOT_CANCELLABLE", f"plan is {p.status.value}; only queued plans can be cancelled")
        self.sats[p.sat].queue.remove(pid)
        p.status = m.PlanStatus.cancelled
        for a in p.acts:
            a.status = m.ActivityStatus.skipped
            a.reasons.add("cancelled")
            self._release(a)
        self._emit(m.EventType.plan_cancelled, satellite_id=self.sats[p.sat].id, plan_id=pid)
        return p

    def get_plan(self, pid: str) -> PlanRec:
        if pid not in self.plans:
            raise ApiError(404, "NOT_FOUND", f"unknown plan {pid!r}")
        return self.plans[pid]

    # ------------------------------------------------------------------ #
    # requests
    # ------------------------------------------------------------------ #

    def create_request(self, spec: m.ImagingRequestCreate) -> ReqRec:
        rid = self._id("req")
        ws = self.to_s(spec.window_start) if spec.window_start else self.t
        u = ph.unit(ph.ecef_from_geodetic(spec.target.lat_deg, spec.target.lon_deg))
        r = ReqRec(rid, spec, self.t, ws, self.to_s(spec.window_end), u)
        self.requests[rid] = r
        self._emit(m.EventType.request_created, request_id=rid)
        return r

    def get_request(self, rid: str) -> ReqRec:
        if rid not in self.requests:
            raise ApiError(404, "NOT_FOUND", f"unknown request {rid!r}")
        return self.requests[rid]

    # ------------------------------------------------------------------ #
    # access computations
    # ------------------------------------------------------------------ #

    def contacts(self, s: float, e: float, sat_ids, st_ids, min_el: float | None) -> list[m.ContactWindow]:
        if e - s > 7 * 86400:
            raise ApiError(400, "WINDOW_TOO_LONG", "max 7 days")
        sats = [self._sat(x) for x in sat_ids] if sat_ids else list(range(len(self.sats)))
        sts = [self._st(x) for x in st_ids] if st_ids else list(range(len(self.stations)))
        step = 10.0
        ts = np.arange(s, e + step, step)
        g = ph.gmst_rad(self.epoch_unix + ts)
        out = []
        for chunk in range(0, len(sats), 16):
            idx = sats[chunk:chunk + 16]
            r, _ = self.orbits.eci(ts, idx=idx)
            ecef = ph.rot_z(r, g)  # (n, T, 3)
            for jj in sts:
                mask = self.stations[jj].min_el if min_el is None else min_el
                el = ph.elevation_deg(ecef, self.st_ecef[jj])  # (n, T)
                for row, i in enumerate(idx):
                    out.extend(self._windows(el[row], ts, mask, i, jj, s, e))
        out.sort(key=lambda c: c.aos)
        return out

    def _windows(self, el, ts, mask, i, j, s, e):
        above = el >= mask
        if not above.any():
            return []
        d = np.diff(above.astype(int))
        starts = list(np.nonzero(d == 1)[0] + 1)
        ends = list(np.nonzero(d == -1)[0] + 1)
        if above[0]:
            starts.insert(0, 0)
        if above[-1]:
            ends.append(len(above))
        res = []

        def cross(k):  # interpolate crossing between k-1 and k
            if k <= 0 or k >= len(ts):
                return ts[min(max(k, 0), len(ts) - 1)]
            e0, e1 = el[k - 1], el[k]
            f = (mask - e0) / (e1 - e0) if e1 != e0 else 0
            return ts[k - 1] + f * (ts[k] - ts[k - 1])

        for a, b in zip(starts, ends):
            aos = max(s, cross(a)) if a > 0 else ts[0]
            los = min(e, cross(b)) if b < len(ts) else min(e, ts[-1])
            if los <= aos:
                continue
            k = a + int(np.argmax(el[a:b]))
            res.append(m.ContactWindow(
                satellite_id=self.sats[i].id, station_id=self.stations[j].id, aos=self.to_dt(aos),
                los=self.to_dt(los), tca=self.to_dt(ts[k]), max_elevation_deg=round(float(el[k]), 2),
                duration_s=round(float(los - aos), 1)))
        return res

    def _st(self, sid: str) -> int:
        if sid not in self.st_index:
            raise ApiError(404, "NOT_FOUND", f"unknown station {sid!r}")
        return self.st_index[sid]

    def opportunities(self, q: m.OpportunityQuery) -> list[m.ImagingOpportunity]:
        s, e = self.to_s(q.start), self.to_s(q.end)
        if e - s > 7 * 86400:
            raise ApiError(400, "WINDOW_TOO_LONG", "max 7 days")
        targets: list[tuple[m.GeoPoint, str | None]] = [(p, None) for p in q.targets]
        for rid in q.request_ids:
            targets.append((self.get_request(rid).spec.target, rid))
        if not targets:
            return []
        sats = [self._sat(x) for x in q.satellite_ids] if q.satellite_ids else list(range(len(self.sats)))
        step = 10.0
        ts = np.arange(s, e + step, step)
        g = ph.gmst_rad(self.epoch_unix + ts)
        sun = ph.rot_z(ph.sun_eci_unit(self.epoch_unix + ts), g)
        U = np.array([ph.unit(ph.ecef_from_geodetic(p.lat_deg, p.lon_deg)) for p, _ in targets])  # (K,3)
        out = []
        for chunk in range(0, len(sats), 16):
            idx = sats[chunk:chunk + 16]
            r, _ = self.orbits.eci(ts, idx=idx)
            gu = ph.unit(ph.rot_z(r, g))  # (n,T,3)
            for row, i in enumerate(idx):
                p0, p1 = gu[row, :-1], gu[row, 1:]
                n = ph.unit(np.cross(p0, p1))  # (T-1,3)
                xt = np.arcsin(np.clip(U @ n.T, -1, 1)) * ph.RE  # (K, T-1)
                # (p0 x u).n = u.(n x p0) and (u x p1).n = u.(p1 x n)
                between = (U @ np.cross(n, p0).T >= 0) & (U @ np.cross(p1, n).T >= 0)
                swath = self.sats[i].spec.swath_km
                hits = between & (np.abs(xt) <= swath / 2)
                for k, tk in zip(*np.nonzero(hits)):
                    # along-track fraction of the segment at closest approach
                    a0 = ph.angular_distance_rad(p0[tk], U[k])
                    seg = ph.angular_distance_rad(p0[tk], p1[tk])
                    ct = abs(xt[k, tk]) / ph.RE
                    along = math.acos(min(1.0, math.cos(a0) / max(math.cos(ct), 1e-12)))
                    f = min(max(along / seg, 0.0), 1.0) if seg > 0 else 0.0
                    tca = ts[tk] + f * (ts[tk + 1] - ts[tk])
                    sun_el = float(ph.sun_elevation_deg(U[k], sun[tk]))
                    min_sun = q.min_sun_elevation_deg
                    if min_sun is not None and sun_el < min_sun:
                        continue
                    pt, rid = targets[k]
                    # ground speed ~ 7 km/s; half a frame (~10 km) either side
                    half = 1.5
                    out.append(m.ImagingOpportunity(
                        satellite_id=self.sats[i].id, target=pt, request_id=rid, tca=self.to_dt(tca),
                        start=self.to_dt(tca - half), end=self.to_dt(tca + half),
                        cross_track_km=round(float(xt[k, tk]), 2), sun_elevation_deg=round(sun_el, 2)))
        out.sort(key=lambda o: o.tca)
        return out

    def ephemeris(self, sid: str, s: float, e: float, step: float) -> m.Ephemeris:
        i = self._sat(sid)
        if (e - s) / step > 20000:
            raise ApiError(400, "TOO_MANY_POINTS", "reduce window or increase step_s (max 20000 points)")
        ts = np.arange(s, e + 1e-9, step)
        r, v = self.orbits.eci(ts, idx=[i])
        r, v = r[0], v[0]
        g = ph.gmst_rad(self.epoch_unix + ts)
        ecef = ph.rot_z(r, g)
        lat, lon, alt = ph.geodetic_from_ecef(ecef)
        sun = ph.sun_eci_unit(self.epoch_unix + ts)
        lit = ph.in_sunlight(r, sun)
        sel = ph.sun_elevation_deg(ecef, ph.rot_z(sun, g))
        pts = [m.EphemerisPoint(
            time=self.to_dt(t), position=m.GeoPosition(lat_deg=float(lat[k]), lon_deg=float(lon[k]), alt_km=float(alt[k])),
            position_eci_km=tuple(map(float, r[k])), velocity_eci_km_s=tuple(map(float, v[k])),
            in_sunlight=bool(lit[k]), sun_elevation_deg=float(sel[k])) for k, t in enumerate(ts)]
        return m.Ephemeris(satellite_id=sid, step_s=step, points=pts)

    def cloud_forecast(self, q: m.CloudQuery) -> list[m.CloudForecast]:
        if len(q.times) not in (1, len(q.points)):
            raise ApiError(400, "BAD_REQUEST", "times must have length 1 or len(points)")
        out = []
        for k, p in enumerate(q.points):
            when = q.times[0] if len(q.times) == 1 else q.times[k]
            t = self.to_s(when)
            sigma = float(self._forecast_sigma(t, self.t))
            out.append(m.CloudForecast(point=p, time=when, issued_at=self.to_dt(self.t),
                                       cloud_fraction=float(self._forecast_mean(p.lat_deg, p.lon_deg, t, self.t)),
                                       sigma=round(sigma, 3)))
        return out

    # ------------------------------------------------------------------ #
    # serialisation
    # ------------------------------------------------------------------ #

    def status(self) -> m.SimStatus:
        return m.SimStatus(sim_time=self.to_dt(self.t), epoch=self.epoch, running=self.running, rate=self.rate,
                           step_s=self.step_s, scenario=self.scn.name, last_event_seq=len(self.events))

    def scenario_info(self) -> m.ScenarioInfo:
        return m.ScenarioInfo(name=self.scn.name, description=self.scn.description, epoch=self.epoch,
                              seed=self.cfg.seed, rules=self.scn.rules, flocks=self.flocks,
                              ground_station_ids=[s.id for s in self.stations],
                              frames={"inertial": "ECI (mean-of-date approx)", "earth_fixed": "ECEF (GMST rotation, spherical Earth)"})

    def _resources(self, sat: SatRec, snap: tuple | None = None) -> m.Resources:
        storage, battery, n_img = snap or self._resources_tuple(sat)
        return m.Resources(storage_used_mb=round(storage, 3), storage_capacity_mb=sat.spec.storage_capacity_mb,
                           battery_wh=round(battery, 3), battery_capacity_wh=sat.spec.battery_capacity_wh,
                           images_onboard=n_img)

    def act_model(self, a: ActRec):
        common = dict(id=a.id, start=self.to_dt(a.start), end=self.to_dt(a.end), status=a.status,
                      result={"data_mb": round(a.data_mb, 3), "reasons": sorted(a.reasons), "image_ids": a.image_ids})
        if a.type == "imaging":
            return m.ImagingActivity(request_ids=a.request_ids, **common)
        return m.DownlinkActivity(station_id=self.stations[a.station].id, **common)

    def plan_model(self, p: PlanRec) -> m.Plan:
        return m.Plan(id=p.id, satellite_id=self.sats[p.sat].id, start=self.to_dt(p.start), end=self.to_dt(p.end),
                      label=p.label, status=p.status, activities=[self.act_model(a) for a in p.acts],
                      submitted_at=self.to_dt(p.submitted_at),
                      uplinked_at=self.to_dt(p.uplinked_at) if p.uplinked_at is not None else None,
                      uplink_station_id=self.stations[p.uplink_station].id if p.uplink_station is not None else None,
                      superseded_by=p.superseded_by, warnings=p.warnings)

    def orbit_model(self, i: int) -> m.OrbitElements:
        raan, u = self.orbits.angles(self.t, idx=[i])
        return m.OrbitElements(epoch=self.to_dt(self.t), semi_major_axis_km=float(self.orbits.a[i]), eccentricity=0.0,
                               inclination_deg=float(np.rad2deg(self.orbits.inc[i])),
                               raan_deg=float(np.rad2deg(raan[0]) % 360), arg_perigee_deg=0.0,
                               mean_anomaly_deg=float(np.rad2deg(u[0]) % 360))

    def sat_model(self, i: int) -> m.SatelliteState:
        sat = self.sats[i]
        cur = sat.current_act if sat.current_act and sat.current_act.start <= self.t < sat.current_act.end else None
        return m.SatelliteState(
            id=sat.id, name=sat.name, flock=sat.flock, as_of=self.to_dt(self.t), health=sat.health, mode=sat.mode,
            position=m.GeoPosition(lat_deg=float(self.lat[i]), lon_deg=float(self.lon[i]), alt_km=float(self.alt[i])),
            position_eci_km=tuple(map(float, self.r_eci[i])), velocity_eci_km_s=tuple(map(float, self.v_eci[i])),
            in_sunlight=bool(self.sunlit[i]), sun_elevation_deg=float(self.sun_el[i]), orbit=self.orbit_model(i),
            resources=self._resources(sat), current_activity=self.act_model(cur) if cur else None,
            active_plan_id=cur.plan_id if cur else None, queued_plan_ids=list(sat.queue),
            stations_in_view=[self.stations[j].id for j in np.nonzero(self.in_view[i])[0]],
            last_contact_at=self.to_dt(sat.last_contact) if sat.last_contact is not None else None, spec=sat.spec)

    def station_model(self, j: int) -> m.GroundStationState:
        st = self.stations[j]
        res = [m.Reservation(satellite_id=self.sats[a.sat].id, plan_id=a.plan_id, activity_id=a.id,
                             start=self.to_dt(a.start), end=self.to_dt(a.end))
               for a in self.reservations[j] if a.end > self.t]
        return m.GroundStationState(
            id=st.id, name=st.name, location=m.GeoPosition(lat_deg=st.lat, lon_deg=st.lon, alt_km=st.alt_m / 1000),
            min_elevation_deg=st.min_el, antennas=st.antennas,
            max_downlink_rate_mbps=st.max_downlink_rate_mbps, as_of=self.to_dt(self.t),
            satellites_in_view=[self.sats[i].id for i in np.nonzero(self.in_view[:, j])[0]],
            antennas_busy=len(self._antennas_busy(j, self.t)), reservations=res)

    def onboard_schedule(self, sid: str) -> m.OnboardSchedule:
        i = self._sat(sid)
        sat = self.sats[i]
        recent = [a for p in self.plans.values() if p.sat == i for a in p.acts
                  if a.status in (m.ActivityStatus.completed, m.ActivityStatus.partial, m.ActivityStatus.failed)
                  and a.end > self.t - 6 * 3600]
        acts = sorted(recent + sat.onboard, key=lambda a: a.start)
        return m.OnboardSchedule(satellite_id=sid, as_of=self.to_dt(self.t), activities=[self.act_model(a) for a in acts],
                                 queued_plan_ids=list(sat.queue))

    def telemetry(self, sid: str) -> m.SatelliteTelemetry:
        sat = self.sats[self._sat(sid)]
        if sat.telemetry is None:
            return m.SatelliteTelemetry(satellite_id=sid, sampled_at=None, received_at=None)
        t, j, health, mode, res, plan_id, done = sat.telemetry
        return m.SatelliteTelemetry(
            satellite_id=sid, sampled_at=self.to_dt(t), received_at=self.to_dt(t), received_via=self.stations[j].id,
            health=health, mode=mode, resources=self._resources(sat, res), active_plan_id=plan_id,
            completed_activity_ids=list(done))

    def image_model(self, img: ImageRec) -> m.Image:
        return m.Image(id=img.id, satellite_id=self.sats[img.sat].id, plan_id=img.plan_id, activity_id=img.act_id,
                       start=self.to_dt(img.start), end=self.to_dt(img.end), size_mb=round(img.size_mb, 3),
                       downlinked_mb=round(img.downlinked_mb, 3), status=img.status,
                       downlinked_at=self.to_dt(img.downlinked_at) if img.downlinked_at is not None else None,
                       downlink_station_id=self.stations[img.station].id if img.station is not None else None,
                       cloud_fraction=round(img.cloud_sum / img.cloud_n, 3) if img.cloud_n else 0.0,
                       swath_km=self.sats[img.sat].spec.swath_km, centerline=img.centerline,
                       request_ids=img.request_ids)

    def request_model(self, r: ReqRec) -> m.ImagingRequest:
        return m.ImagingRequest(**r.spec.model_dump(exclude={"window_start"}), window_start=self.to_dt(r.win_start),
                                id=r.id, created_at=self.to_dt(r.created_at), status=r.status, image_ids=r.image_ids,
                                captured_at=self.to_dt(r.captured_at) if r.captured_at is not None else None,
                                fulfilled_at=self.to_dt(r.fulfilled_at) if r.fulfilled_at is not None else None)

    def _cell_weights(self, res: float) -> np.ndarray:
        lat_c = 90 - (np.arange(int(180 / res)) + 0.5) * res
        return np.repeat(np.cos(np.deg2rad(lat_c))[:, None], int(360 / res), axis=1)

    def coverage_grid(self, res: float, since: float) -> m.CoverageGrid:
        if res < self.cov_res or abs(res / self.cov_res - round(res / self.cov_res)) > 1e-9:
            raise ApiError(400, "BAD_RESOLUTION", f"resolution_deg must be a multiple of {self.cov_res}")
        f = int(round(res / self.cov_res))
        R, C = self.coverage.shape
        grid = self.coverage.reshape(R // f, f, C // f, f)
        agg = np.fmax.reduce(np.fmax.reduce(grid, axis=3), axis=1)  # NaN-ignoring max
        hit = ~np.isnan(agg) & (agg >= since)
        w = self._cell_weights(res)
        frac = float((w * hit).sum() / w.sum())
        rows = [[None if (np.isnan(v) or v < since) else round(float(v), 1) for v in row] for row in agg]
        return m.CoverageGrid(resolution_deg=res, since=self.to_dt(since), sim_time=self.to_dt(self.t),
                              coverage_fraction=round(frac, 5), rows=agg.shape[0], cols=agg.shape[1],
                              origin=m.GeoPoint(lat_deg=90, lon_deg=-180), last_imaged_s=rows)

    def metrics(self, window_s: float) -> m.Metrics:
        since = max(0.0, self.t - window_s)
        imgs = [self.images[k] for k in self.image_order if self.images[k].end >= since]
        down = [x for x in imgs if x.status == m.ImageStatus.downlinked]
        lat = np.array([x.downlinked_at - x.end for x in down]) if down else None
        hit = ~np.isnan(self.coverage) & (np.nan_to_num(self.coverage, nan=-1) >= since)
        w = self._cell_weights(self.cov_res)
        reqs = list(self.requests.values())
        cnt = lambda st: sum(r.status == st for r in reqs)  # noqa: E731
        ant_avail = sum(s.antennas for s in self.stations) * max(self.t, 1.0)
        return m.Metrics(
            sim_time=self.to_dt(self.t), window_start=self.to_dt(since), images_captured=len(imgs),
            images_downlinked=len(down), data_captured_mb=round(sum(x.size_mb for x in imgs), 3),
            data_downlinked_mb=round(sum(x.downlinked_mb for x in imgs), 3),
            data_onboard_mb=round(sum(s.storage_mb for s in self.sats), 3),
            mean_latency_s=round(float(lat.mean()), 1) if lat is not None else None,
            p90_latency_s=round(float(np.percentile(lat, 90)), 1) if lat is not None else None,
            coverage_fraction=round(float((w * hit).sum() / w.sum()), 5),
            station_utilization=round(self.antenna_busy_s / ant_avail, 5),
            satellites_in_safe_mode=sum(s.health == m.HealthStatus.safe for s in self.sats),
            requests=m.RequestMetrics(
                total=len(reqs), open=cnt(m.RequestStatus.open), captured=cnt(m.RequestStatus.captured),
                fulfilled=cnt(m.RequestStatus.fulfilled), expired=cnt(m.RequestStatus.expired),
                fulfilled_priority=sum(r.spec.priority for r in reqs if r.status == m.RequestStatus.fulfilled),
                total_priority=sum(r.spec.priority for r in reqs)))
