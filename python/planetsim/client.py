"""Typed Python client for the constellation simulator HTTP API."""

from __future__ import annotations

import time
from collections.abc import Iterable, Iterator, Sequence
from datetime import datetime, timedelta, timezone
from typing import Any, TypeVar

import httpx
from pydantic import BaseModel, TypeAdapter

from . import models as m

T = TypeVar("T")

DEFAULT_URL = "http://localhost:8080"
PREFIX = f"/api/{m.API_VERSION}"


class SimAPIError(RuntimeError):
    def __init__(self, status: int, code: str, message: str, details: Any = None):
        super().__init__(f"[{status} {code}] {message}")
        self.status, self.code, self.message, self.details = status, code, message, details


class PlanRejected(SimAPIError):
    """Raised when a submitted plan fails validation (HTTP 422 / 409)."""

    @property
    def validation(self) -> m.PlanValidation:
        return m.PlanValidation.model_validate(self.details or {"valid": False})


def iso(t: datetime | str) -> str:
    if isinstance(t, str):
        return t
    if t.tzinfo is None:
        raise ValueError("naive datetime; use timezone-aware UTC datetimes")
    return t.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


class SimClient:
    """Thin, typed wrapper around the REST API.

    >>> sim = SimClient()                      # doctest: +SKIP
    >>> sim.reset(seed=1, random_requests=100)  # doctest: +SKIP
    >>> sats = sim.satellites()                 # doctest: +SKIP
    """

    def __init__(
        self,
        base_url: str = DEFAULT_URL,
        *,
        timeout: float = 60.0,
        http: httpx.Client | None = None,
    ):
        self._http = http or httpx.Client(base_url=base_url, timeout=timeout)

    def close(self) -> None:
        self._http.close()

    def __enter__(self) -> SimClient:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    # ------------------------------------------------------------------ #
    # plumbing
    # ------------------------------------------------------------------ #

    def _request(self, method: str, path: str, *, params: dict | None = None, body: Any = None,
                 timeout: Any = httpx.USE_CLIENT_DEFAULT) -> Any:
        if isinstance(body, BaseModel):
            body = body.model_dump(mode="json", exclude_none=True)
        params = {k: v for k, v in (params or {}).items() if v is not None}
        kw = {}
        if timeout is not httpx.USE_CLIENT_DEFAULT and not getattr(self._http, "_planetsim_inprocess", False):
            kw["timeout"] = timeout  # the in-process transport has no timeouts (and warns if given one)
        resp = self._http.request(method, PREFIX + path, params=params, json=body, **kw)
        if resp.status_code >= 400:
            try:
                err = m.ErrorResponse.model_validate(resp.json()).error
                code, msg, details = err.code, err.message, err.details
            except Exception:
                code, msg, details = "HTTP_ERROR", resp.text, None
            cls = PlanRejected if code in ("PLAN_INVALID", "ANTENNA_CONFLICT") else SimAPIError
            raise cls(resp.status_code, code, msg, details)
        return resp.json() if resp.content else None

    def _get(self, path: str, model: type[T] | Any, **params: Any) -> T:
        return TypeAdapter(model).validate_python(self._request("GET", path, params=params))

    def _post(self, path: str, model: type[T] | Any, body: Any = None, **params: Any) -> T:
        return TypeAdapter(model).validate_python(self._request("POST", path, params=params, body=body))

    # ------------------------------------------------------------------ #
    # simulation control
    # ------------------------------------------------------------------ #

    def status(self) -> m.SimStatus:
        return self._get("/sim", m.SimStatus)

    def now(self) -> datetime:
        return self.status().sim_time

    def pause(self) -> m.SimStatus:
        return self._post("/sim/control", m.SimStatus, m.SimControl(action=m.SimControlAction.pause))

    def resume(self, rate: float | None = None) -> m.SimStatus:
        if rate is not None:
            self.set_rate(rate)
        return self._post("/sim/control", m.SimStatus, m.SimControl(action=m.SimControlAction.resume))

    def set_rate(self, rate: float) -> m.SimStatus:
        return self._post("/sim/control", m.SimStatus, m.SimControl(action=m.SimControlAction.set_rate, rate=rate))

    def step(self, seconds: float | timedelta | None = None, *, until: datetime | None = None) -> m.SimStatus:
        """Advance a paused sim by ``seconds`` or to ``until``."""
        if isinstance(seconds, timedelta):
            seconds = seconds.total_seconds()
        return self._post("/sim/step", m.SimStatus, m.SimStep(seconds=seconds, until=until))

    def reset(self, **kwargs: Any) -> m.SimStatus:
        """Reset the scenario. kwargs: scenario, seed, epoch, num_satellites, random_requests."""
        return self._post("/sim/reset", m.SimStatus, m.SimReset(**kwargs))

    def scenario(self) -> m.ScenarioInfo:
        return self._get("/scenario", m.ScenarioInfo)

    # ------------------------------------------------------------------ #
    # state
    # ------------------------------------------------------------------ #

    def constellation(self) -> m.ConstellationState:
        """Full snapshot: sim status + every satellite + every ground station."""
        return self._get("/constellation", m.ConstellationState)

    def satellites(self) -> list[m.SatelliteState]:
        return self._get("/satellites", list[m.SatelliteState])

    def satellite(self, satellite_id: str) -> m.SatelliteState:
        return self._get(f"/satellites/{satellite_id}", m.SatelliteState)

    def telemetry(self, satellite_id: str) -> m.SatelliteTelemetry:
        """Ground knowledge (last telemetry received during a contact), not truth."""
        return self._get(f"/satellites/{satellite_id}/telemetry", m.SatelliteTelemetry)

    def ephemeris(self, satellite_id: str, start: datetime, end: datetime, step_s: float = 60.0) -> m.Ephemeris:
        return self._get(
            f"/satellites/{satellite_id}/ephemeris", m.Ephemeris, start=iso(start), end=iso(end), step_s=step_s
        )

    def onboard_schedule(self, satellite_id: str) -> m.OnboardSchedule:
        return self._get(f"/satellites/{satellite_id}/schedule", m.OnboardSchedule)

    def ground_stations(self) -> list[m.GroundStationState]:
        return self._get("/ground-stations", list[m.GroundStationState])

    def ground_station(self, station_id: str) -> m.GroundStationState:
        return self._get(f"/ground-stations/{station_id}", m.GroundStationState)

    # ------------------------------------------------------------------ #
    # access geometry
    # ------------------------------------------------------------------ #

    def contacts(
        self,
        start: datetime,
        end: datetime,
        *,
        satellite_ids: Sequence[str] | None = None,
        station_ids: Sequence[str] | None = None,
        min_elevation_deg: float | None = None,
    ) -> list[m.ContactWindow]:
        return self._get(
            "/access/contacts",
            list[m.ContactWindow],
            start=iso(start),
            end=iso(end),
            satellite_id=list(satellite_ids) if satellite_ids else None,
            station_id=list(station_ids) if station_ids else None,
            min_elevation_deg=min_elevation_deg,
        )

    def opportunities(
        self,
        start: datetime,
        end: datetime,
        *,
        targets: Iterable[m.GeoPoint | tuple[float, float]] = (),
        request_ids: Sequence[str] = (),
        satellite_ids: Sequence[str] | None = None,
        min_sun_elevation_deg: float | None = None,
    ) -> list[m.ImagingOpportunity]:
        """When does each target (or request) pass under some satellite's swath?

        ``targets`` may be GeoPoints or (lat, lon) tuples.
        """
        pts = [t if isinstance(t, m.GeoPoint) else m.GeoPoint(lat_deg=t[0], lon_deg=t[1]) for t in targets]
        q = m.OpportunityQuery(
            start=start,
            end=end,
            targets=pts,
            request_ids=list(request_ids),
            satellite_ids=list(satellite_ids) if satellite_ids else None,
            min_sun_elevation_deg=min_sun_elevation_deg,
        )
        return self._post("/access/opportunities", list[m.ImagingOpportunity], q)

    # ------------------------------------------------------------------ #
    # plans
    # ------------------------------------------------------------------ #

    def validate_plan(self, plan: m.PlanSubmission) -> m.PlanValidation:
        """Dry run: check a plan against all constraints without queuing it."""
        return self._post("/plans/validate", m.PlanValidation, plan)

    def submit_plan(self, plan: m.PlanSubmission) -> m.Plan:
        """Queue a plan for uplink. Raises PlanRejected if invalid."""
        return self._post("/plans", m.Plan, plan)

    def submit_plans(self, plans: Sequence[m.PlanSubmission], *, atomic: bool = False) -> m.PlanBatchResult:
        return self._post("/plans/batch", m.PlanBatchResult, m.PlanBatchSubmission(plans=list(plans), atomic=atomic))

    def rollout(
        self,
        plans: Sequence[m.PlanSubmission] = (),
        *,
        until: datetime | None = None,
        duration: float | timedelta | None = None,
        monte_carlo: m.MonteCarloConfig | dict | int | None = None,
        cloud_model: m.CloudModel | str = m.CloudModel.forecast,
        satellite_ids: Sequence[str] | None = None,
        cancel_queued: bool = False,
        atomic: bool = False,
        include_plans: bool = True,
        include_events: bool = False,
        event_types: Sequence[m.EventType | str] | None = None,
        timeout: float | None = None,
    ) -> m.RolloutResult:
        """Simulate ``plans`` forward on a copy of the current state; the live sim is untouched.

        ``monte_carlo`` turns on sampling: an int (number of samples), a dict of
        :class:`~planetsim.models.MonteCarloConfig` fields, or the model itself.
        Rollouts can be slow, so there is no client timeout by default.
        """
        if isinstance(duration, timedelta):
            duration = duration.total_seconds()
        if isinstance(monte_carlo, int):
            monte_carlo = m.MonteCarloConfig(samples=monte_carlo)
        elif isinstance(monte_carlo, dict):
            monte_carlo = m.MonteCarloConfig(**monte_carlo)
        req = m.RolloutRequest(
            plans=list(plans), atomic=atomic, until=until, duration_s=duration, cancel_queued=cancel_queued,
            satellite_ids=list(satellite_ids) if satellite_ids is not None else None,
            cloud_model=cloud_model, monte_carlo=monte_carlo, include_plans=include_plans,
            include_events=include_events, event_types=list(event_types) if event_types else None,
        )
        return TypeAdapter(m.RolloutResult).validate_python(
            self._request("POST", "/rollouts", body=req, timeout=timeout))

    def plans(
        self, *, satellite_id: str | None = None, status: m.PlanStatus | Sequence[m.PlanStatus] | None = None
    ) -> list[m.Plan]:
        if isinstance(status, m.PlanStatus):
            status = [status]
        return self._get(
            "/plans",
            list[m.Plan],
            satellite_id=satellite_id,
            status=[getattr(s, "value", s) for s in status] if status else None,
        )

    def plan(self, plan_id: str) -> m.Plan:
        return self._get(f"/plans/{plan_id}", m.Plan)

    def cancel_plan(self, plan_id: str) -> m.Plan:
        """Cancel a plan that has not been uplinked yet (409 otherwise)."""
        return TypeAdapter(m.Plan).validate_python(self._request("DELETE", f"/plans/{plan_id}"))

    # ------------------------------------------------------------------ #
    # demand
    # ------------------------------------------------------------------ #

    def create_request(self, req: m.ImagingRequestCreate) -> m.ImagingRequest:
        return self._post("/requests", m.ImagingRequest, req)

    def requests(self, *, status: m.RequestStatus | None = None) -> list[m.ImagingRequest]:
        return self._get("/requests", list[m.ImagingRequest], status=status.value if status else None)

    def request(self, request_id: str) -> m.ImagingRequest:
        return self._get(f"/requests/{request_id}", m.ImagingRequest)

    def cancel_request(self, request_id: str) -> m.ImagingRequest:
        return TypeAdapter(m.ImagingRequest).validate_python(self._request("DELETE", f"/requests/{request_id}"))

    # ------------------------------------------------------------------ #
    # products / telemetry streams
    # ------------------------------------------------------------------ #

    def images(
        self,
        *,
        satellite_id: str | None = None,
        status: m.ImageStatus | None = None,
        since: datetime | None = None,
        offset: int = 0,
        limit: int = 500,
    ) -> m.ImagePage:
        return self._get(
            "/images",
            m.ImagePage,
            satellite_id=satellite_id,
            status=status.value if status else None,
            since=iso(since) if since else None,
            offset=offset,
            limit=limit,
        )

    def events(
        self,
        after_seq: int = 0,
        *,
        limit: int = 1000,
        types: Sequence[m.EventType] | None = None,
        satellite_id: str | None = None,
    ) -> m.EventPage:
        return self._get(
            "/events",
            m.EventPage,
            after_seq=after_seq,
            limit=limit,
            type=[getattr(t, "value", t) for t in types] if types else None,
            satellite_id=satellite_id,
        )

    def iter_events(self, after_seq: int = 0, **kw: Any) -> Iterator[m.Event]:
        """Iterate every event after ``after_seq`` currently available (follows pagination)."""
        while True:
            page = self.events(after_seq, **kw)
            yield from page.events
            after_seq = page.next_after_seq
            if not page.has_more:
                return

    def metrics(self, window_s: float = 86400.0) -> m.Metrics:
        return self._get("/metrics", m.Metrics, window_s=window_s)

    def coverage(self, *, resolution_deg: float = 1.0, since: datetime | None = None) -> m.CoverageGrid:
        return self._get("/coverage", m.CoverageGrid, resolution_deg=resolution_deg, since=iso(since) if since else None)

    def cloud_forecast(
        self, points: Sequence[m.GeoPoint | tuple[float, float]], times: datetime | Sequence[datetime]
    ) -> list[m.CloudForecast]:
        pts = [p if isinstance(p, m.GeoPoint) else m.GeoPoint(lat_deg=p[0], lon_deg=p[1]) for p in points]
        ts = [times] if isinstance(times, datetime) else list(times)
        return self._post("/weather/cloud-forecast", m.CloudForecastResponse, m.CloudQuery(points=pts, times=ts)).forecasts

    # ------------------------------------------------------------------ #
    # convenience
    # ------------------------------------------------------------------ #

    def wait_until(self, t: datetime, *, poll_s: float = 0.5, timeout_s: float | None = None) -> m.SimStatus:
        """Block until sim_time >= t. Steps directly if the sim is paused."""
        st = self.status()
        if not st.running:
            return self.step(until=t) if st.sim_time < t else st
        deadline = None if timeout_s is None else time.monotonic() + timeout_s
        while st.sim_time < t:
            if deadline is not None and time.monotonic() > deadline:
                raise TimeoutError(f"sim did not reach {t} (at {st.sim_time})")
            time.sleep(poll_s)
            st = self.status()
        return st


class PlanBuilder:
    """Fluent helper for assembling a :class:`PlanSubmission`.

    >>> p = (PlanBuilder("SD-001", t0, t0 + timedelta(hours=6))   # doctest: +SKIP
    ...      .image(t1, t2).downlink("svalbard", t3, t4).build())
    """

    def __init__(self, satellite_id: str, start: datetime, end: datetime, label: str | None = None):
        self.satellite_id, self.start, self.end, self.label = satellite_id, start, end, label
        self._acts: list[m.ImagingActivity | m.DownlinkActivity] = []

    def image(self, start: datetime, end: datetime, request_ids: Sequence[str] = ()) -> PlanBuilder:
        self._acts.append(m.ImagingActivity(start=start, end=end, request_ids=list(request_ids)))
        return self

    def downlink(self, station_id: str, start: datetime, end: datetime) -> PlanBuilder:
        self._acts.append(m.DownlinkActivity(start=start, end=end, station_id=station_id))
        return self

    def build(self) -> m.PlanSubmission:
        acts = sorted(self._acts, key=lambda a: a.start)
        return m.PlanSubmission(
            satellite_id=self.satellite_id, start=self.start, end=self.end, activities=acts, label=self.label
        )
