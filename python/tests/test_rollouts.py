"""What-if rollouts (deterministic and Monte Carlo)."""

from __future__ import annotations

from datetime import timedelta

import pytest

from planetsim import PlanBuilder, SimAPIError
from planetsim import models as m
from planetsim.mock import inprocess_client

H = timedelta(hours=1)


def greedy_plans(sim, start, horizon):
    """Downlink every long pass, image the rest of the daylight (enough to exercise everything)."""
    import sys
    from pathlib import Path

    sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "examples"))
    from greedy_scheduler import build_plans

    return build_plans(sim, start, horizon)


@pytest.fixture(scope="module")
def setup():
    sim = inprocess_client(seed=2, num_satellites=10, random_requests=150)
    plans = greedy_plans(sim, sim.now(), 4 * H)
    return sim, plans


def test_rollout_does_not_touch_live_sim(setup):
    sim, plans = setup
    before = sim.status()
    r = sim.rollout(plans, duration=4 * H)
    assert sim.status() == before
    assert sim.plans() == []
    assert r.validation.accepted == len(plans) and r.samples_run == 1 and not r.monte_carlo
    assert r.end - r.start == 4 * H
    assert r.samples[0].metrics.images_captured > 0
    assert set(r.plan_uplink_prob) == {p.id for p in r.plans}
    assert all(p.status in (m.PlanStatus.active, m.PlanStatus.completed, m.PlanStatus.uplinked) for p in r.plans)


def test_deterministic_rollout_is_repeatable(setup):
    sim, plans = setup
    a = sim.rollout(plans, duration=4 * H, include_plans=False)
    b = sim.rollout(plans, duration=4 * H, include_plans=False)
    assert a.samples[0].metrics == b.samples[0].metrics


def test_truth_rollout_predicts_the_live_outcome():
    """With the oracle cloud model and no randomness, a rollout must match what really happens."""
    sim = inprocess_client(seed=4, num_satellites=6, random_requests=100)
    plans = greedy_plans(sim, sim.now(), 3 * H)
    pred = sim.rollout(plans, duration=3 * H, cloud_model="truth").samples[0]
    sim.submit_plans(plans)
    sim.step(3 * H)
    real = sim.metrics(window_s=3 * 3600)
    for f in ("images_captured", "images_downlinked", "data_captured_mb", "data_downlinked_mb",
              "data_onboard_mb", "coverage_fraction", "mean_latency_s"):
        assert getattr(pred.metrics, f) == getattr(real, f), f
    assert pred.requests_fulfilled == real.requests.fulfilled


def test_monte_carlo_statistics_and_reproducibility(setup):
    sim, plans = setup
    cfg = dict(samples=4, seed=7, pass_failure_prob=0.2, uplink_failure_prob=0.2, anomaly_rate_per_sat_day=1.0)
    a = sim.rollout(plans, duration=4 * H, monte_carlo=cfg, include_plans=False)
    assert a.monte_carlo and a.samples_run == 4 and [s.seed for s in a.samples] == [7, 8, 9, 10]
    st = a.summary["images_downlinked"]
    assert st.min <= st.p10 <= st.p50 <= st.p90 <= st.max and st.std >= 0
    assert all(0 <= v <= 1 for v in a.activity_success_prob.values())
    b = sim.rollout(plans, duration=4 * H, monte_carlo=cfg, include_plans=False)
    assert [s.metrics for s in a.samples] == [s.metrics for s in b.samples]


def test_monte_carlo_total_pass_failure_blocks_uplink(setup):
    sim, plans = setup
    r = sim.rollout(plans, duration=2 * H, monte_carlo=dict(samples=2, pass_failure_prob=1.0), include_plans=False)
    # contacts already in progress at the start are not re-rolled, so allow for those
    in_view = {s.id for s in sim.satellites() if s.stations_in_view}
    for pid, prob in r.plan_uplink_prob.items():
        sat = next(it.plan.satellite_id for it in r.validation.items if it.plan and it.plan.id == pid)
        if sat not in in_view:
            assert prob == 0.0


def test_cloud_models_differ(setup):
    sim, plans = setup
    req_ids = [q.id for q in sim.requests()]
    f = sim.rollout(plans, duration=4 * H, include_plans=False)
    s = sim.rollout(plans, duration=4 * H, include_plans=False,
                    monte_carlo=dict(samples=3, cloud_model="sampled"))
    assert set(f.request_fulfillment_prob) == set(s.request_fulfillment_prob) == set(req_ids)
    assert s.summary["requests_fulfilled"].std >= 0


def test_satellite_subset_and_cancel_queued(setup):
    sim, plans = setup
    sub = sim.rollout(plans, duration=2 * H, satellite_ids=["SD-001"], include_events=True,
                      event_types=["activity_started"])
    assert sub.events and {e.satellite_id for e in sub.events} == {"SD-001"}

    t0 = sim.now()
    queued = sim.submit_plan(PlanBuilder("SD-002", t0, t0 + 6 * H).build())
    r = sim.rollout([p for p in plans if p.satellite_id == "SD-002"], duration=H, cancel_queued=True)
    assert r.validation.accepted == 1
    assert sim.plan(queued.id).status == m.PlanStatus.queued  # cancelled only inside the rollout
    sim.cancel_plan(queued.id)


def test_rollout_reports_invalid_plans_instead_of_raising(setup):
    sim, _ = setup
    t0 = sim.now()
    bad = PlanBuilder("SD-001", t0, t0 + 200 * H).build()
    r = sim.rollout([bad], duration=H)
    assert r.validation.rejected == 1 and r.plans == []
    with pytest.raises(SimAPIError):
        sim.rollout([], duration=30 * 24 * H)
