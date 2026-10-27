"""What-if rollouts: run candidate plans forward on forks of the engine."""

from __future__ import annotations

import time

import numpy as np

from .. import models as m
from .engine import ApiError, Engine

MAX_ROLLOUT_S = 7 * 86400.0

# Metrics summarised across samples; None (e.g. no latency yet) is skipped.
SUMMARY_FIELDS = {
    "coverage_fraction": lambda s: s.metrics.coverage_fraction,
    "images_captured": lambda s: s.metrics.images_captured,
    "images_downlinked": lambda s: s.metrics.images_downlinked,
    "data_captured_mb": lambda s: s.metrics.data_captured_mb,
    "data_downlinked_mb": lambda s: s.metrics.data_downlinked_mb,
    "data_onboard_mb": lambda s: s.metrics.data_onboard_mb,
    "mean_latency_s": lambda s: s.metrics.mean_latency_s,
    "station_utilization": lambda s: s.metrics.station_utilization,
    "satellites_in_safe_mode": lambda s: s.metrics.satellites_in_safe_mode,
    "requests_fulfilled": lambda s: s.requests_fulfilled,
    "fulfilled_priority": lambda s: s.fulfilled_priority,
}


def stats(values: list[float]) -> m.MetricStats:
    a = np.asarray(values, float)
    p10, p50, p90 = np.percentile(a, [10, 50, 90])
    return m.MetricStats(mean=float(a.mean()), std=float(a.std()), min=float(a.min()), p10=float(p10),
                         p50=float(p50), p90=float(p90), max=float(a.max()))


def run_rollout(live: Engine, req: m.RolloutRequest) -> m.RolloutResult:
    wall0 = time.monotonic()
    base = live.fork()  # copy under the live lock, then run without blocking the live sim
    start = base.t
    if (req.until is None) == (req.duration_s is None):
        raise ApiError(400, "BAD_REQUEST", "give exactly one of until / duration_s")
    end = start + req.duration_s if req.duration_s is not None else base.to_s(req.until)
    if end <= start:
        raise ApiError(400, "BAD_REQUEST", "rollout must end after the current sim time")
    if end - start > MAX_ROLLOUT_S:
        raise ApiError(400, "WINDOW_TOO_LONG", f"max rollout is {MAX_ROLLOUT_S:.0f} s")

    if req.satellite_ids is not None:
        mask = np.zeros(len(base.sats), bool)
        for sid in req.satellite_ids:
            mask[base._sat(sid)] = True
        base.sim_mask = mask

    if req.cancel_queued:
        targets = {base._sat(p.satellite_id) for p in req.plans if p.satellite_id in base.sat_index}
        for sat_i in targets:
            for pid in list(base.sats[sat_i].queue):
                base.cancel(pid)

    validation = base.submit_batch(m.PlanBatchSubmission(plans=req.plans, atomic=req.atomic))
    plan_ids = [it.plan.id for it in validation.items if it.plan is not None]
    act_ids = [a.id for pid in plan_ids for a in base.plans[pid].acts]
    open_reqs = [r.id for r in base.requests.values()
                 if r.status in (m.RequestStatus.open, m.RequestStatus.captured)]
    busy0 = base.antenna_busy_s

    mc = req.monte_carlo
    n = mc.samples if mc else 1
    samples: list[m.RolloutSample] = []
    uplinked = dict.fromkeys(plan_ids, 0)
    act_ok = dict.fromkeys(act_ids, 0)
    fulfilled = dict.fromkeys(open_reqs, 0)

    for k in range(n):
        eng = base.fork() if k < n - 1 else base  # last sample may consume the base copy
        seed = (mc.seed + k) if mc else None
        eng.configure_uncertainty(mc, mc.cloud_model if mc else req.cloud_model, seed)
        eng.advance_to(end)

        met = eng.metrics(end - start)
        n_ant = sum(s.antennas for s in eng.stations)
        met.station_utilization = round((eng.antenna_busy_s - busy0) / (n_ant * (end - start)), 5)
        if eng.sim_mask is not None:
            met.satellites_in_safe_mode = sum(
                s.health == m.HealthStatus.safe for s, on in zip(eng.sats, eng.sim_mask) if on)
        done = [rid for rid in open_reqs if eng.requests[rid].status == m.RequestStatus.fulfilled]
        samples.append(m.RolloutSample(
            index=k, seed=seed, metrics=met, requests_fulfilled=len(done),
            fulfilled_priority=sum(eng.requests[rid].spec.priority for rid in done)))
        for rid in done:
            fulfilled[rid] += 1
        for pid in plan_ids:
            p = eng.plans[pid]
            uplinked[pid] += p.uplinked_at is not None
            for a in p.acts:
                act_ok[a.id] += a.status in (m.ActivityStatus.completed, m.ActivityStatus.partial) or (
                    a.status == m.ActivityStatus.executing and a.data_mb > 0)
        if k == 0:
            first_plans = [eng.plan_model(eng.plans[pid]) for pid in plan_ids] if req.include_plans else []
            first_events = [e for e in eng.events
                            if not req.event_types or e.type in req.event_types] if req.include_events else []

    summary = {}
    for name, get in SUMMARY_FIELDS.items():
        vals = [v for v in (get(s) for s in samples) if v is not None]
        if vals:
            summary[name] = stats(vals)

    return m.RolloutResult(
        start=base.to_dt(start), end=base.to_dt(end), monte_carlo=mc is not None, samples_run=n,
        validation=validation, samples=samples, summary=summary,
        request_fulfillment_prob={k: v / n for k, v in fulfilled.items()},
        plan_uplink_prob={k: v / n for k, v in uplinked.items()},
        activity_success_prob={k: v / n for k, v in act_ok.items()},
        plans=first_plans, events=first_events, wall_time_s=round(time.monotonic() - wall0, 3))
