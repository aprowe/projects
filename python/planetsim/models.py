"""Wire models for the constellation simulator API (v1).

These pydantic models are the single source of truth for the HTTP contract.
`scripts/export_openapi.py` turns them into `api/openapi.json`, which is what the
Vulkan simulator implements.

Conventions
-----------
* All timestamps are ISO-8601 UTC strings (``2026-01-01T00:00:00Z``).
* Field names carry their unit as a suffix: ``_s`` seconds, ``_km``, ``_m``,
  ``_deg``, ``_mb`` (megabytes), ``_mbps`` (megabits/s), ``_wh``, ``_w``.
* Positions: geodetic ``lat_deg``/``lon_deg`` (lon in [-180, 180)), and
  Earth-centred inertial (``_eci_km``, J2000-ish / TEME, the sim decides and
  reports it in ``ScenarioInfo.frames``).
* Time intervals are half-open: ``[start, end)``.
"""

from __future__ import annotations

from enum import Enum
from typing import Annotated, Any, Literal, Union

from pydantic import AwareDatetime, BaseModel, ConfigDict, Field

API_VERSION = "v1"


class _Model(BaseModel):
    model_config = ConfigDict(extra="ignore", populate_by_name=True)


# --------------------------------------------------------------------------- #
# Enums
# --------------------------------------------------------------------------- #


class SatelliteMode(str, Enum):
    idle = "idle"            # nadir, camera off, not downlinking
    imaging = "imaging"      # nadir, camera on
    downlink = "downlink"    # pointing X-band antenna at a ground station
    transition = "transition"  # slewing between attitudes (no imaging / downlink)
    safe = "safe"            # fault / low-power; ignores its plan until recovered


class HealthStatus(str, Enum):
    nominal = "nominal"
    degraded = "degraded"
    safe = "safe"
    decommissioned = "decommissioned"


class PlanStatus(str, Enum):
    queued = "queued"          # accepted by ground, waiting for an uplink opportunity
    uplinked = "uplinked"      # onboard, not yet started
    active = "active"          # onboard, sim_time within [start, end)
    completed = "completed"    # onboard and sim_time >= end
    superseded = "superseded"  # all of its remaining activities replaced by a newer plan
    expired = "expired"        # never uplinked before its end time
    cancelled = "cancelled"    # cancelled by the operator before uplink


class ActivityStatus(str, Enum):
    pending = "pending"        # plan not onboard yet
    scheduled = "scheduled"    # onboard, in the future
    executing = "executing"
    completed = "completed"
    partial = "partial"        # ran, but constraint hit part-way (storage full, LOS, darkness...)
    failed = "failed"          # ran, produced nothing
    skipped = "skipped"        # start time already past when uplinked, or safe mode
    superseded = "superseded"  # replaced by a newer plan before it ran


class ImageStatus(str, Enum):
    onboard = "onboard"
    downlinked = "downlinked"
    lost = "lost"


class RequestStatus(str, Enum):
    open = "open"
    captured = "captured"      # acquired onboard, not yet on the ground
    fulfilled = "fulfilled"    # acquired *and* downlinked within the window
    expired = "expired"
    cancelled = "cancelled"


class EventType(str, Enum):
    sim_reset = "sim_reset"
    contact_start = "contact_start"
    contact_end = "contact_end"
    telemetry_received = "telemetry_received"
    plan_queued = "plan_queued"
    plan_uplinked = "plan_uplinked"
    plan_activated = "plan_activated"
    plan_completed = "plan_completed"
    plan_superseded = "plan_superseded"
    plan_expired = "plan_expired"
    plan_cancelled = "plan_cancelled"
    activity_started = "activity_started"
    activity_ended = "activity_ended"
    image_captured = "image_captured"
    image_downlinked = "image_downlinked"
    request_created = "request_created"
    request_captured = "request_captured"
    request_fulfilled = "request_fulfilled"
    request_expired = "request_expired"
    storage_full = "storage_full"
    battery_low = "battery_low"
    safe_mode_entered = "safe_mode_entered"
    safe_mode_exited = "safe_mode_exited"


# --------------------------------------------------------------------------- #
# Shared value types
# --------------------------------------------------------------------------- #


class GeoPoint(_Model):
    lat_deg: float = Field(ge=-90, le=90)
    lon_deg: float = Field(ge=-180, le=180)


class GeoPosition(GeoPoint):
    alt_km: float


class TimeWindow(_Model):
    start: AwareDatetime
    end: AwareDatetime


class OrbitElements(_Model):
    """Osculating/mean Keplerian elements at ``epoch``.

    Enough for a client to propagate locally (two-body + J2 secular). ``tle`` is
    provided when the simulator can produce one (lets you use sgp4/skyfield).
    """

    epoch: AwareDatetime
    semi_major_axis_km: float
    eccentricity: float
    inclination_deg: float
    raan_deg: float
    arg_perigee_deg: float
    mean_anomaly_deg: float
    tle: tuple[str, str] | None = None


# --------------------------------------------------------------------------- #
# Error envelope
# --------------------------------------------------------------------------- #


class ErrorBody(_Model):
    code: str
    message: str
    details: Any | None = None


class ErrorResponse(_Model):
    error: ErrorBody


# --------------------------------------------------------------------------- #
# Simulation control / scenario
# --------------------------------------------------------------------------- #


class SimStatus(_Model):
    sim_time: AwareDatetime
    epoch: AwareDatetime = Field(description="Scenario start time.")
    running: bool
    rate: float = Field(description="Sim seconds per wall-clock second while running.")
    step_s: float = Field(description="Internal integration step.")
    scenario: str
    last_event_seq: int


class SimControlAction(str, Enum):
    pause = "pause"
    resume = "resume"
    set_rate = "set_rate"


class SimControl(_Model):
    action: SimControlAction
    rate: float | None = Field(default=None, gt=0)


class SimStep(_Model):
    """Advance a *paused* simulation deterministically."""

    seconds: float | None = Field(default=None, gt=0)
    until: AwareDatetime | None = None


class SimReset(_Model):
    scenario: str = "planetscope-default"
    seed: int = 0
    epoch: AwareDatetime | None = None
    num_satellites: int | None = Field(
        default=None, ge=1, description="Truncate the constellation (handy for tests)."
    )
    random_requests: int = Field(
        default=0, ge=0, description="Generate this many random point imaging requests."
    )


class SpacecraftSpec(_Model):
    """Static capabilities of one spacecraft (SuperDove-like by default)."""

    bus: str = "3U"
    instrument: str = "PSB.SD"
    bands: int = 8
    gsd_m: float
    swath_km: float
    storage_capacity_mb: float
    imaging_data_rate_mbps: float
    downlink_rate_mbps: float
    battery_capacity_wh: float
    solar_power_w: float
    idle_power_w: float
    imaging_power_w: float
    downlink_power_w: float
    min_sun_elevation_deg: float = Field(description="Imaging produces data only if the sun is at least this high at the sub-satellite point.")
    transition_time_s: float = Field(description="Minimum gap between consecutive activities of different type/target (attitude slew).")
    pointing: Literal["nadir_only"] = "nadir_only"


class SimRules(_Model):
    plan_max_horizon_s: float = Field(description="Max (plan.end - plan.start).")
    plan_max_activities: int
    uplink_duration_s: float = Field(description="Continuous in-view time needed to uplink queued plans.")
    opportunistic_uplink: bool = Field(description="If true, queued plans uplink at *any* contact with a free antenna, not only during scheduled downlinks.")
    telemetry_on_contact: bool = True
    safe_mode_enter_battery_frac: float
    safe_mode_exit_battery_frac: float
    downlink_order: Literal["fifo", "priority"] = "fifo"


class FlockInfo(_Model):
    name: str
    satellite_ids: list[str]
    altitude_km: float
    inclination_deg: float
    ltdn_hours: float | None = Field(default=None, description="Local time of descending node (SSO planes).")


class ScenarioInfo(_Model):
    name: str
    description: str
    epoch: AwareDatetime
    seed: int
    frames: dict[str, str] = Field(default_factory=lambda: {"inertial": "TEME", "earth_fixed": "ITRF-approx"})
    rules: SimRules
    flocks: list[FlockInfo]
    ground_station_ids: list[str]


# --------------------------------------------------------------------------- #
# Activities & plans
# --------------------------------------------------------------------------- #


class _ActivityBase(_Model):
    start: AwareDatetime
    end: AwareDatetime
    # Server-populated:
    id: str | None = None
    status: ActivityStatus | None = None
    result: dict[str, Any] | None = Field(default=None, description="Execution outcome (data_mb, reason, image_ids, ...).")


class ImagingActivity(_ActivityBase):
    """Turn the (nadir-pointing) camera on for [start, end)."""

    type: Literal["imaging"] = "imaging"
    request_ids: list[str] = Field(default_factory=list, description="Optional tags: requests this strip is meant to serve.")


class DownlinkActivity(_ActivityBase):
    """Point at ``station_id`` and dump stored imagery for [start, end).

    Reserves one antenna at the station for the interval.
    """

    type: Literal["downlink"] = "downlink"
    station_id: str


Activity = Annotated[Union[ImagingActivity, DownlinkActivity], Field(discriminator="type")]


class PlanSubmission(_Model):
    satellite_id: str
    start: AwareDatetime
    end: AwareDatetime
    activities: list[Activity] = Field(default_factory=list)
    label: str | None = None


class Violation(_Model):
    code: str = Field(description="Machine readable, e.g. OVERLAP, NOT_IN_CONTACT, ANTENNA_CONFLICT, OUT_OF_HORIZON.")
    message: str
    activity_index: int | None = None


class PlanValidation(_Model):
    valid: bool
    violations: list[Violation] = Field(default_factory=list)
    warnings: list[Violation] = Field(default_factory=list)


class Plan(_Model):
    id: str
    satellite_id: str
    start: AwareDatetime
    end: AwareDatetime
    label: str | None = None
    status: PlanStatus
    activities: list[Activity]
    submitted_at: AwareDatetime
    uplinked_at: AwareDatetime | None = None
    uplink_station_id: str | None = None
    superseded_by: str | None = None
    warnings: list[Violation] = Field(default_factory=list)


class PlanBatchSubmission(_Model):
    plans: list[PlanSubmission]
    atomic: bool = Field(default=False, description="If true, reject all plans when any one is invalid.")


class PlanBatchItem(_Model):
    index: int
    plan: Plan | None = None
    validation: PlanValidation


class PlanBatchResult(_Model):
    accepted: int
    rejected: int
    items: list[PlanBatchItem]


class OnboardSchedule(_Model):
    """What the satellite will actually do (truth), plus what is waiting on the ground."""

    satellite_id: str
    as_of: AwareDatetime
    activities: list[Activity] = Field(description="Onboard timeline, including recently finished activities.")
    queued_plan_ids: list[str]


# --------------------------------------------------------------------------- #
# Satellites
# --------------------------------------------------------------------------- #


class Resources(_Model):
    storage_used_mb: float
    storage_capacity_mb: float
    battery_wh: float
    battery_capacity_wh: float
    images_onboard: int


class SatelliteState(_Model):
    """Ground-truth state of one satellite at ``as_of``."""

    id: str
    name: str
    flock: str
    as_of: AwareDatetime
    health: HealthStatus
    mode: SatelliteMode
    position: GeoPosition
    position_eci_km: tuple[float, float, float]
    velocity_eci_km_s: tuple[float, float, float]
    in_sunlight: bool
    sun_elevation_deg: float = Field(description="Sun elevation at the sub-satellite point.")
    orbit: OrbitElements
    resources: Resources
    current_activity: Activity | None = None
    active_plan_id: str | None = None
    queued_plan_ids: list[str] = Field(default_factory=list)
    stations_in_view: list[str] = Field(default_factory=list)
    last_contact_at: AwareDatetime | None = None
    spec: SpacecraftSpec


class SatelliteTelemetry(_Model):
    """What the *ground* knows: last telemetry frame received during a contact."""

    satellite_id: str
    sampled_at: AwareDatetime | None
    received_at: AwareDatetime | None
    received_via: str | None = None
    health: HealthStatus | None = None
    mode: SatelliteMode | None = None
    resources: Resources | None = None
    active_plan_id: str | None = None
    completed_activity_ids: list[str] = Field(default_factory=list)


class EphemerisPoint(_Model):
    time: AwareDatetime
    position: GeoPosition
    position_eci_km: tuple[float, float, float]
    velocity_eci_km_s: tuple[float, float, float]
    in_sunlight: bool
    sun_elevation_deg: float


class Ephemeris(_Model):
    satellite_id: str
    step_s: float
    points: list[EphemerisPoint]


# --------------------------------------------------------------------------- #
# Ground stations
# --------------------------------------------------------------------------- #


class Reservation(_Model):
    satellite_id: str
    plan_id: str
    activity_id: str
    start: AwareDatetime
    end: AwareDatetime


class GroundStation(_Model):
    id: str
    name: str
    location: GeoPosition
    min_elevation_deg: float
    antennas: int
    max_downlink_rate_mbps: float | None = None


class GroundStationState(GroundStation):
    as_of: AwareDatetime
    satellites_in_view: list[str]
    antennas_busy: int
    reservations: list[Reservation] = Field(description="Upcoming/ongoing downlink reservations.")


# --------------------------------------------------------------------------- #
# Whole-constellation snapshot
# --------------------------------------------------------------------------- #


class ConstellationState(_Model):
    sim: SimStatus
    satellites: list[SatelliteState]
    ground_stations: list[GroundStationState]


# --------------------------------------------------------------------------- #
# Access (geometry helpers for planners)
# --------------------------------------------------------------------------- #


class ContactWindow(_Model):
    satellite_id: str
    station_id: str
    aos: AwareDatetime
    los: AwareDatetime
    tca: AwareDatetime = Field(description="Time of max elevation.")
    max_elevation_deg: float
    duration_s: float


class ImagingOpportunity(_Model):
    satellite_id: str
    target: GeoPoint
    request_id: str | None = None
    tca: AwareDatetime = Field(description="Time the target is closest to the ground track.")
    start: AwareDatetime = Field(description="Target enters the swath (along-track frame edge).")
    end: AwareDatetime
    cross_track_km: float = Field(description="Signed: + = right of ground track.")
    sun_elevation_deg: float


class OpportunityQuery(_Model):
    start: AwareDatetime
    end: AwareDatetime
    targets: list[GeoPoint] = Field(default_factory=list)
    request_ids: list[str] = Field(default_factory=list)
    satellite_ids: list[str] | None = None
    min_sun_elevation_deg: float | None = None


# --------------------------------------------------------------------------- #
# Demand: imaging requests
# --------------------------------------------------------------------------- #


class ImagingRequestCreate(_Model):
    name: str | None = None
    target: GeoPoint
    priority: float = Field(default=1.0, gt=0, description="Relative value; scoring weight.")
    window_start: AwareDatetime | None = None
    window_end: AwareDatetime
    max_cloud_fraction: float = Field(default=1.0, ge=0, le=1)


class ImagingRequest(ImagingRequestCreate):
    id: str
    created_at: AwareDatetime
    status: RequestStatus
    image_ids: list[str] = Field(default_factory=list)
    captured_at: AwareDatetime | None = None
    fulfilled_at: AwareDatetime | None = None


# --------------------------------------------------------------------------- #
# Products: images
# --------------------------------------------------------------------------- #


class Image(_Model):
    """One continuous strip captured during an imaging activity."""

    id: str
    satellite_id: str
    plan_id: str | None
    activity_id: str | None
    start: AwareDatetime
    end: AwareDatetime
    size_mb: float
    downlinked_mb: float
    status: ImageStatus
    downlinked_at: AwareDatetime | None = None
    downlink_station_id: str | None = None
    cloud_fraction: float
    swath_km: float
    centerline: list[tuple[float, float]] = Field(description="[lon_deg, lat_deg] samples along the ground track.")
    request_ids: list[str] = Field(default_factory=list, description="Requests whose target falls inside the strip.")


# --------------------------------------------------------------------------- #
# Events, metrics, coverage, weather
# --------------------------------------------------------------------------- #


class Event(_Model):
    seq: int = Field(description="Monotonic; use as a cursor (`after_seq`).")
    time: AwareDatetime
    type: EventType
    satellite_id: str | None = None
    station_id: str | None = None
    plan_id: str | None = None
    activity_id: str | None = None
    image_id: str | None = None
    request_id: str | None = None
    data: dict[str, Any] = Field(default_factory=dict)


class EventPage(_Model):
    events: list[Event]
    next_after_seq: int
    has_more: bool


class Page(_Model):
    total: int
    offset: int
    limit: int


class ImagePage(Page):
    items: list[Image]


class RequestMetrics(_Model):
    total: int
    open: int
    captured: int
    fulfilled: int
    expired: int
    fulfilled_priority: float
    total_priority: float


class Metrics(_Model):
    sim_time: AwareDatetime
    window_start: AwareDatetime
    images_captured: int
    images_downlinked: int
    data_captured_mb: float
    data_downlinked_mb: float
    data_onboard_mb: float
    mean_latency_s: float | None = Field(description="Capture end -> fully downlinked.")
    p90_latency_s: float | None
    coverage_fraction: float = Field(description="Area-weighted fraction of grid cells imaged in the window.")
    station_utilization: float = Field(description="Antenna-seconds used for downlink / antenna-seconds available.")
    satellites_in_safe_mode: int
    requests: RequestMetrics


class CoverageGrid(_Model):
    resolution_deg: float
    since: AwareDatetime
    sim_time: AwareDatetime
    mask: Literal["none", "land"] = "none"
    coverage_fraction: float
    rows: int
    cols: int
    origin: GeoPoint = Field(description="North-west corner of cell [0][0]. Rows go south, cols go east.")
    last_imaged_s: list[list[float | None]] = Field(description="Seconds since epoch of the latest image of each cell, or null.")


class CloudQuery(_Model):
    points: list[GeoPoint]
    times: list[AwareDatetime] = Field(description="Either one time (applied to all points) or one per point.")


class CloudForecast(_Model):
    point: GeoPoint
    time: AwareDatetime
    issued_at: AwareDatetime
    cloud_fraction: float = Field(ge=0, le=1)
    sigma: float = Field(description="1-sigma forecast uncertainty; grows with lead time.")


class CloudForecastResponse(_Model):
    forecasts: list[CloudForecast]


# --------------------------------------------------------------------------- #
# Rollouts (what-if simulation, optionally Monte Carlo)
# --------------------------------------------------------------------------- #


class CloudModel(str, Enum):
    forecast = "forecast"  # clouds = the forecast issued at rollout start (what the planner expects)
    sampled = "sampled"    # forecast + random error drawn per sample with the forecast's sigma
    truth = "truth"        # oracle: the simulator's true cloud field (upper bound / debugging only)


class MonteCarloConfig(_Model):
    """Uncertainty injected independently into each rollout sample."""

    samples: int = Field(default=20, ge=2, le=500)
    seed: int = Field(default=0, description="Sample k uses seed + k, so results are reproducible.")
    cloud_model: CloudModel = CloudModel.sampled
    pass_failure_prob: float = Field(default=0.0, ge=0, le=1, description="Chance a whole contact fails (no telemetry, uplink or downlink).")
    uplink_failure_prob: float = Field(default=0.0, ge=0, le=1, description="Chance an uplink attempt fails; retried at the next contact.")
    anomaly_rate_per_sat_day: float = Field(default=0.0, ge=0, description="Poisson rate of random safe-mode faults per satellite per day.")
    anomaly_recovery_s: float = Field(default=6 * 3600.0, ge=0, description="Minimum time a faulted satellite stays in safe mode.")


class RolloutRequest(_Model):
    """Simulate candidate plans forward on a *copy* of the current state. The live sim is untouched.

    The copy includes everything already queued/onboard; candidate plans are validated and
    applied on top exactly as ``POST /plans/batch`` would.
    """

    plans: list[PlanSubmission] = Field(default_factory=list)
    atomic: bool = False
    until: AwareDatetime | None = None
    duration_s: float | None = Field(default=None, gt=0, description="Alternative to `until`.")
    cancel_queued: bool = Field(default=False, description="First cancel still-queued plans of every satellite that has a candidate plan.")
    satellite_ids: list[str] | None = Field(default=None, description="Simulate only these satellites (others are frozen). Much faster; metrics cover only these.")
    cloud_model: CloudModel = Field(default=CloudModel.forecast, description="Cloud model for a deterministic (non Monte Carlo) rollout.")
    monte_carlo: MonteCarloConfig | None = None
    include_plans: bool = Field(default=True, description="Final state of the candidate plans (deterministic run, or Monte Carlo sample 0).")
    include_events: bool = False
    event_types: list[EventType] | None = None


class MetricStats(_Model):
    mean: float
    std: float
    min: float
    p10: float
    p50: float
    p90: float
    max: float


class RolloutSample(_Model):
    index: int
    seed: int | None
    metrics: Metrics
    requests_fulfilled: int = Field(description="Requests fulfilled during the rollout.")
    fulfilled_priority: float


class RolloutResult(_Model):
    start: AwareDatetime
    end: AwareDatetime
    monte_carlo: bool
    samples_run: int
    validation: PlanBatchResult
    samples: list[RolloutSample]
    summary: dict[str, MetricStats] = Field(description="Per-metric statistics across samples (std = 0 for one sample).")
    request_fulfillment_prob: dict[str, float] = Field(description="For each request still open/captured at start: fraction of samples in which it was fulfilled.")
    plan_uplink_prob: dict[str, float] = Field(description="For each accepted candidate plan: fraction of samples in which it reached the satellite.")
    activity_success_prob: dict[str, float] = Field(description="For each candidate activity id: fraction of samples in which it completed (fully or partially).")
    plans: list[Plan] = Field(default_factory=list)
    events: list[Event] = Field(default_factory=list)
    wall_time_s: float
