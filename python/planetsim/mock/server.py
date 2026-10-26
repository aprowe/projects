"""FastAPI app exposing the v1 API on top of the mock engine.

Run:  python -m planetsim.mock --port 8080
"""

from __future__ import annotations

from datetime import datetime
from typing import Annotated

from fastapi import Body, FastAPI, Query, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse

from .. import models as m
from .engine import ApiError, Engine

P = f"/api/{m.API_VERSION}"
ERRORS = {r: {"model": m.ErrorResponse} for r in (400, 404, 409, 422)}


def create_app(engine: Engine | None = None) -> FastAPI:
    eng = engine or Engine()
    app = FastAPI(
        title="Constellation Simulator API",
        version="1.0.0",
        description=(
            "State and planning interface for a PlanetScope-like Earth-observation constellation. "
            "Plans are submitted to the ground segment and only reach a satellite when it next "
            "passes over a ground station (see docs/API.md)."
        ),
        responses=ERRORS,
    )
    app.state.engine = eng

    @app.exception_handler(ApiError)
    async def _api_error(_: Request, exc: ApiError):
        body = m.ErrorResponse(error=m.ErrorBody(code=exc.code, message=exc.message, details=exc.details))
        return JSONResponse(status_code=exc.status, content=body.model_dump(mode="json"))

    @app.exception_handler(RequestValidationError)
    async def _validation_error(_: Request, exc: RequestValidationError):
        body = m.ErrorResponse(error=m.ErrorBody(code="BAD_REQUEST", message="request validation failed",
                                                 details=jsonable(exc.errors())))
        return JSONResponse(status_code=400, content=body.model_dump(mode="json"))

    def synced() -> Engine:
        eng.sync()
        return eng

    # ---------------------------------------------------------------- sim
    @app.get(P + "/sim", response_model=m.SimStatus, tags=["sim"])
    def get_sim():
        return synced().status()

    @app.post(P + "/sim/control", response_model=m.SimStatus, tags=["sim"])
    def control(body: m.SimControl):
        e = synced()
        with e.lock:
            if body.action == m.SimControlAction.pause:
                e.running = False
            elif body.action == m.SimControlAction.resume:
                e.running = True
            elif body.action == m.SimControlAction.set_rate:
                if body.rate is None:
                    raise ApiError(400, "BAD_REQUEST", "rate required")
                e.rate = body.rate
            return e.status()

    @app.post(P + "/sim/step", response_model=m.SimStatus, tags=["sim"])
    def step(body: m.SimStep):
        e = synced()
        with e.lock:
            if e.running:
                raise ApiError(409, "SIM_RUNNING", "pause the simulation before stepping")
            if (body.seconds is None) == (body.until is None):
                raise ApiError(400, "BAD_REQUEST", "give exactly one of seconds / until")
            target = e.t + body.seconds if body.seconds is not None else e.to_s(body.until)
            if target > e.t + 14 * 86400:
                raise ApiError(400, "BAD_REQUEST", "max 14 days per step call")
            e.advance_to(target)
            return e.status()

    @app.post(P + "/sim/reset", response_model=m.SimStatus, tags=["sim"])
    def reset(body: m.SimReset = Body(default_factory=m.SimReset)):
        eng.reset(body)
        return eng.status()

    @app.get(P + "/scenario", response_model=m.ScenarioInfo, tags=["sim"])
    def scenario():
        return synced().scenario_info()

    # ---------------------------------------------------------------- state
    @app.get(P + "/constellation", response_model=m.ConstellationState, tags=["state"])
    def constellation():
        e = synced()
        with e.lock:
            return m.ConstellationState(
                sim=e.status(),
                satellites=[e.sat_model(i) for i in range(len(e.sats))],
                ground_stations=[e.station_model(j) for j in range(len(e.stations))],
            )

    @app.get(P + "/satellites", response_model=list[m.SatelliteState], tags=["state"])
    def satellites():
        e = synced()
        with e.lock:
            return [e.sat_model(i) for i in range(len(e.sats))]

    @app.get(P + "/satellites/{satellite_id}", response_model=m.SatelliteState, tags=["state"])
    def satellite(satellite_id: str):
        e = synced()
        with e.lock:
            return e.sat_model(e._sat(satellite_id))

    @app.get(P + "/satellites/{satellite_id}/telemetry", response_model=m.SatelliteTelemetry, tags=["state"])
    def telemetry(satellite_id: str):
        e = synced()
        with e.lock:
            return e.telemetry(satellite_id)

    @app.get(P + "/satellites/{satellite_id}/schedule", response_model=m.OnboardSchedule, tags=["state"])
    def schedule(satellite_id: str):
        e = synced()
        with e.lock:
            return e.onboard_schedule(satellite_id)

    @app.get(P + "/satellites/{satellite_id}/ephemeris", response_model=m.Ephemeris, tags=["access"])
    def ephemeris(satellite_id: str, start: datetime, end: datetime, step_s: Annotated[float, Query(gt=0)] = 60.0):
        e = synced()
        return e.ephemeris(satellite_id, e.to_s(start), e.to_s(end), step_s)

    @app.get(P + "/ground-stations", response_model=list[m.GroundStationState], tags=["state"])
    def stations():
        e = synced()
        with e.lock:
            return [e.station_model(j) for j in range(len(e.stations))]

    @app.get(P + "/ground-stations/{station_id}", response_model=m.GroundStationState, tags=["state"])
    def station(station_id: str):
        e = synced()
        with e.lock:
            return e.station_model(e._st(station_id))

    # ---------------------------------------------------------------- access
    @app.get(P + "/access/contacts", response_model=list[m.ContactWindow], tags=["access"])
    def contacts(
        start: datetime,
        end: datetime,
        satellite_id: Annotated[list[str] | None, Query()] = None,
        station_id: Annotated[list[str] | None, Query()] = None,
        min_elevation_deg: float | None = None,
    ):
        e = synced()
        return e.contacts(e.to_s(start), e.to_s(end), satellite_id, station_id, min_elevation_deg)

    @app.post(P + "/access/opportunities", response_model=list[m.ImagingOpportunity], tags=["access"])
    def opportunities(q: m.OpportunityQuery):
        return synced().opportunities(q)

    # ---------------------------------------------------------------- plans
    @app.post(P + "/plans/validate", response_model=m.PlanValidation, tags=["plans"])
    def validate(plan: m.PlanSubmission):
        e = synced()
        with e.lock:
            return e.validate(plan)[0]

    @app.post(P + "/plans", response_model=m.Plan, status_code=201, tags=["plans"])
    def submit(plan: m.PlanSubmission):
        e = synced()
        with e.lock:
            return e.plan_model(e.submit(plan))

    @app.post(P + "/plans/batch", response_model=m.PlanBatchResult, tags=["plans"])
    def submit_batch(batch: m.PlanBatchSubmission):
        return synced().submit_batch(batch)

    @app.get(P + "/plans", response_model=list[m.Plan], tags=["plans"])
    def list_plans(satellite_id: str | None = None, status: Annotated[list[m.PlanStatus] | None, Query()] = None):
        e = synced()
        with e.lock:
            return [e.plan_model(p) for p in e.plans.values()
                    if (satellite_id is None or e.sats[p.sat].id == satellite_id)
                    and (not status or p.status in status)]

    @app.get(P + "/plans/{plan_id}", response_model=m.Plan, tags=["plans"])
    def get_plan(plan_id: str):
        e = synced()
        with e.lock:
            return e.plan_model(e.get_plan(plan_id))

    @app.delete(P + "/plans/{plan_id}", response_model=m.Plan, tags=["plans"])
    def cancel_plan(plan_id: str):
        e = synced()
        with e.lock:
            return e.plan_model(e.cancel(plan_id))

    # ---------------------------------------------------------------- requests
    @app.post(P + "/requests", response_model=m.ImagingRequest, status_code=201, tags=["requests"])
    def create_request(req: m.ImagingRequestCreate):
        e = synced()
        with e.lock:
            return e.request_model(e.create_request(req))

    @app.get(P + "/requests", response_model=list[m.ImagingRequest], tags=["requests"])
    def list_requests(status: m.RequestStatus | None = None):
        e = synced()
        with e.lock:
            return [e.request_model(r) for r in e.requests.values() if status is None or r.status == status]

    @app.get(P + "/requests/{request_id}", response_model=m.ImagingRequest, tags=["requests"])
    def get_request(request_id: str):
        e = synced()
        with e.lock:
            return e.request_model(e.get_request(request_id))

    @app.delete(P + "/requests/{request_id}", response_model=m.ImagingRequest, tags=["requests"])
    def cancel_request(request_id: str):
        e = synced()
        with e.lock:
            r = e.get_request(request_id)
            if r.status in (m.RequestStatus.open, m.RequestStatus.captured):
                r.status = m.RequestStatus.cancelled
            return e.request_model(r)

    # ---------------------------------------------------------------- products & telemetry streams
    @app.get(P + "/images", response_model=m.ImagePage, tags=["products"])
    def images(
        satellite_id: str | None = None,
        status: m.ImageStatus | None = None,
        since: datetime | None = None,
        offset: Annotated[int, Query(ge=0)] = 0,
        limit: Annotated[int, Query(ge=1, le=5000)] = 500,
    ):
        e = synced()
        with e.lock:
            t0 = e.to_s(since) if since else None
            sel = [e.images[k] for k in e.image_order
                   if (satellite_id is None or e.sats[e.images[k].sat].id == satellite_id)
                   and (status is None or e.images[k].status == status)
                   and (t0 is None or e.images[k].end >= t0)]
            return m.ImagePage(total=len(sel), offset=offset, limit=limit,
                               items=[e.image_model(x) for x in sel[offset:offset + limit]])

    @app.get(P + "/events", response_model=m.EventPage, tags=["products"])
    def events(
        after_seq: Annotated[int, Query(ge=0)] = 0,
        limit: Annotated[int, Query(ge=1, le=10000)] = 1000,
        type: Annotated[list[m.EventType] | None, Query()] = None,
        satellite_id: str | None = None,
    ):
        e = synced()
        with e.lock:
            out, last = [], after_seq
            for ev in e.events[after_seq:]:
                last = ev.seq
                if (not type or ev.type in type) and (satellite_id is None or ev.satellite_id == satellite_id):
                    out.append(ev)
                    if len(out) >= limit:
                        break
            return m.EventPage(events=out, next_after_seq=last, has_more=last < len(e.events))

    @app.get(P + "/metrics", response_model=m.Metrics, tags=["products"])
    def metrics(window_s: Annotated[float, Query(gt=0)] = 86400.0):
        e = synced()
        with e.lock:
            return e.metrics(window_s)

    @app.get(P + "/coverage", response_model=m.CoverageGrid, tags=["products"])
    def coverage(resolution_deg: float = 1.0, since: datetime | None = None):
        e = synced()
        with e.lock:
            return e.coverage_grid(resolution_deg, e.to_s(since) if since else max(0.0, e.t - 86400))

    @app.post(P + "/weather/cloud-forecast", response_model=m.CloudForecastResponse, tags=["weather"])
    def cloud_forecast(q: m.CloudQuery):
        e = synced()
        return m.CloudForecastResponse(forecasts=e.cloud_forecast(q))

    return app


def jsonable(x):
    if isinstance(x, dict):
        return {k: jsonable(v) for k, v in x.items() if k != "ctx"}
    if isinstance(x, (list, tuple)):
        return [jsonable(v) for v in x]
    if isinstance(x, (str, int, float, bool)) or x is None:
        return x
    return str(x)
