# Constellation Simulator API — v1

The contract between the **simulator** (the Vulkan visualizer, which owns the
clock, physics and truth) and **planners** (Python, via `planetsim.SimClient`).

- Machine-readable spec: [`api/openapi.json`](../api/openapi.json). It's generated
  from the pydantic models in `python/planetsim/models.py`. Change the models,
  then run `python scripts/export_openapi.py`. A test fails if you forget.
- Reference implementation: `python/planetsim/mock/`. It's light on physics but
  follows the contract, so you can use it to write schedulers before the
  real simulator exists.

---

## 1. Operations concept

The simulator models a PlanetScope-like constellation: about 180 SuperDove-class
3U cubesats in morning sun-synchronous orbits (475–525 km). Each one carries a
**nadir-only** push-frame imager with a ~32.5 km swath and an X-band
downlink. There's a network of ground stations, mostly at high latitude.

What makes this a scheduling problem, rather than "image everything all the time":

| Constraint | Effect |
|---|---|
| **Plans only travel during contacts.** A plan you submit sits in the ground queue until its satellite passes over a station. | Planning is *ahead of time*. Anything scheduled before the next uplink opportunity is skipped. |
| **Storage is finite.** Imaging (400 Mbps) fills it 4× faster than downlink (1600 Mbps) drains it. | You can't image all the daylit land. You have to choose. |
| **Ground antennas are shared.** Each station has 1–3 antennas, and many satellites are in view at once over the poles. | Downlink assignment is a resource-constrained interval scheduling problem. |
| **One activity at a time + slew time.** Downlinking needs a different attitude from imaging, with a 60 s transition between them. | Downlink time costs imaging time. |
| **Nadir only.** Doves can't point off-track. | Whether a target can be captured depends only on *when the camera is on*. |
| **Sunlight / power / clouds.** No data is captured below 10° sun elevation. The battery drains in eclipse, and low battery triggers safe mode. Cloudy captures don't satisfy requests. | Use forecasts, and avoid power-negative plans. |
| **The ground only knows what telemetry says.** `/telemetry` is the last frame received during a contact. | A realistic planner should use telemetry, not truth. |

### Objectives (exposed by `/metrics`)

- **Coverage**: area-weighted fraction of the globe imaged in the last 24 h.
  This is Planet's "image the whole landmass daily" mission.
- **Latency**: time from capture to fully downlinked.
- **Request fulfilment**: priority-weighted count of point requests that were
  imaged under their cloud limit *and* downlinked inside their window.
- Station utilisation and data volume.

---

## 2. Conventions

- Base path: `/api/v1`. JSON everywhere.
- Times are ISO-8601 UTC (`2026-01-01T00:00:00Z`). Intervals are half-open `[start, end)`.
- Every unit is a field-name suffix: `_s`, `_km`, `_m`, `_deg`, `_mb` (megabytes),
  `_mbps` (megabits/s), `_wh`, `_w`.
- Longitudes are in [-180, 180). Image centerlines are `[lon, lat]` pairs (GeoJSON order).
- IDs are opaque strings: satellites `SD-001`, stations `svalbard`, and
  server-assigned `plan-…`, `act-…`, `img-…`, `req-…`.
- Errors always use this shape:
  ```json
  {"error": {"code": "PLAN_INVALID", "message": "plan rejected", "details": {...}}}
  ```
  | status | when |
  |---|---|
  | 400 `BAD_REQUEST` / `WINDOW_TOO_LONG` / … | malformed input |
  | 404 `NOT_FOUND` | unknown id |
  | 409 `ANTENNA_CONFLICT`, `NOT_CANCELLABLE`, `SIM_RUNNING` | state conflict |
  | 422 `PLAN_INVALID` | plan violates constraints (`details` is a `PlanValidation`) |

---

## 3. Simulation clock

The simulator owns the clock. It's either **running** (sim seconds = `rate` ×
wall seconds) or **paused**. While paused, `POST /sim/step` advances it
deterministically, which is the right mode for algorithm practice and tests.

| Method | Path | Body → Response |
|---|---|---|
| GET | `/sim` | → `SimStatus` (sim_time, running, rate, last_event_seq …) |
| POST | `/sim/control` | `{"action": "pause" \| "resume" \| "set_rate", "rate"?}` → `SimStatus` |
| POST | `/sim/step` | `{"seconds": 3600}` or `{"until": "…Z"}` → `SimStatus` (409 if running) |
| POST | `/sim/reset` | `SimReset {scenario, seed, epoch?, num_satellites?, random_requests}` → `SimStatus` |
| GET | `/scenario` | → `ScenarioInfo` (rules, flocks, station ids, frames) |

The same `seed` must produce the same run.

---

## 4. State

| Method | Path | Response |
|---|---|---|
| GET | `/constellation` | `ConstellationState`: sim status + **every** satellite + **every** station (one call = whole world, which is what the visualizer would render) |
| GET | `/satellites` | `SatelliteState[]` |
| GET | `/satellites/{id}` | `SatelliteState` (truth) |
| GET | `/satellites/{id}/telemetry` | `SatelliteTelemetry` (ground knowledge: last frame received in a contact) |
| GET | `/satellites/{id}/schedule` | `OnboardSchedule`: activities actually onboard (truth) + queued plan ids |
| GET | `/satellites/{id}/ephemeris?start&end&step_s` | `Ephemeris` (≤ 20 000 points) |
| GET | `/ground-stations` | `GroundStationState[]` |
| GET | `/ground-stations/{id}` | `GroundStationState` incl. future `reservations` |

`SatelliteState` (abridged):

```json
{
  "id": "SD-001", "name": "flock-a-01", "flock": "flock-a",
  "as_of": "2026-01-01T01:00:00Z",
  "health": "nominal", "mode": "imaging",
  "position": {"lat_deg": 41.2, "lon_deg": -12.9, "alt_km": 475.0},
  "position_eci_km": [..], "velocity_eci_km_s": [..],
  "in_sunlight": true, "sun_elevation_deg": 38.1,
  "orbit": {"epoch": "…", "semi_major_axis_km": 6853.1, "eccentricity": 0.0,
            "inclination_deg": 97.31, "raan_deg": 67.8, "arg_perigee_deg": 0.0,
            "mean_anomaly_deg": 98.8, "tle": null},
  "resources": {"storage_used_mb": 41250.0, "storage_capacity_mb": 250000.0,
                "battery_wh": 51.2, "battery_capacity_wh": 60.0, "images_onboard": 3},
  "current_activity": {"type": "imaging", "id": "act-000123", "start": "…", "end": "…", "status": "executing"},
  "active_plan_id": "plan-000120", "queued_plan_ids": [],
  "stations_in_view": [], "last_contact_at": "…",
  "spec": {"swath_km": 32.5, "imaging_data_rate_mbps": 400, "downlink_rate_mbps": 1600,
           "transition_time_s": 60, "min_sun_elevation_deg": 10, "pointing": "nadir_only", "…": "…"}
}
```

`mode` is one of `idle | imaging | downlink | transition | safe`.

---

## 5. Access geometry (planner helpers)

The simulator is the source of truth for orbits, so it also answers the
geometry questions every planner asks. You can propagate locally from `orbit`
if you'd rather.

| Method | Path | Response |
|---|---|---|
| GET | `/access/contacts?start&end&satellite_id=…&station_id=…&min_elevation_deg` | `ContactWindow[]` `{satellite_id, station_id, aos, los, tca, max_elevation_deg, duration_s}`. Repeat `satellite_id` / `station_id` to filter. Max window 7 days. |
| POST | `/access/opportunities` | `OpportunityQuery {start, end, targets[], request_ids[], satellite_ids?, min_sun_elevation_deg?}` → `ImagingOpportunity[]` `{satellite_id, target, request_id, tca, start, end, cross_track_km, sun_elevation_deg}`. An opportunity is a pass where the target falls inside the nadir swath. To capture it, the camera has to be on over `[start, end)`. |

---

## 6. Plans (the write side)

### 6.1 Shape

A **plan** is a timeline for **one satellite** over `[start, end)`. It holds
two kinds of activity:

```json
{
  "satellite_id": "SD-014",
  "start": "2026-01-01T06:00:00Z",
  "end":   "2026-01-01T18:00:00Z",
  "label": "greedy-v3",
  "activities": [
    {"type": "imaging",  "start": "…T06:21:10Z", "end": "…T06:39:40Z", "request_ids": ["req-000017"]},
    {"type": "downlink", "start": "…T07:02:05Z", "end": "…T07:09:30Z", "station_id": "svalbard"}
  ]
}
```

- **imaging**: camera on, nadir. Produces data only while the sun is ≥
  `min_sun_elevation_deg` at the sub-satellite point and storage isn't full.
  `request_ids` are optional tags. Fulfilment is decided by geometry, not by tags.
- **downlink**: point at `station_id` and dump stored data, oldest first.
  It **reserves one antenna** at that station for the interval.

### 6.2 Validation (synchronous, at submission)

| code | rule |
|---|---|
| `UNKNOWN_SATELLITE` / `UNKNOWN_STATION` | ids must exist |
| `BAD_INTERVAL` | end > start |
| `OUT_OF_HORIZON` | `end - start ≤ rules.plan_max_horizon_s` (72 h) |
| `IN_PAST` | plan end > sim_time |
| `TOO_MANY_ACTIVITIES` | ≤ `rules.plan_max_activities` |
| `OUTSIDE_PLAN` | each activity inside the plan interval |
| `OVERLAP` | activities don't overlap |
| `TRANSITION_TIME` | gap ≥ `spec.transition_time_s` between activities, except imaging→imaging |
| `NOT_IN_CONTACT` | a downlink's station must be above its elevation mask for the whole interval |
| `ANTENNA_CONFLICT` | at no instant can a station's reservations by *other* satellites, plus this one, exceed `antennas`. Returns **409** when this is the only problem. |

Warnings don't block acceptance. For example, `STARTS_IN_PAST` means that
activity will be skipped.

Use `POST /plans/validate` to check a plan without queuing it. It returns a
`PlanValidation {valid, violations[], warnings[]}`.

### 6.3 Lifecycle

```
            submit                 contact with ≥ uplink_duration_s in view
 (client) ─────────► queued ───────────────────────────────────────► uplinked ──► active ──► completed
                        │                                               │  t≥start     t≥end
                        │ DELETE /plans/{id}                            │
                        ├──────────► cancelled                          └─► superseded (a newer plan
                        │ plan.end passes while queued                       replaced all its remaining activities)
                        └──────────► expired
```

**Uplink rules**

1. Queued plans for a satellite are uplinked, in submission order, once it has
   been continuously in view of a station for `rules.uplink_duration_s` (60 s).
2. The station needs a free antenna. That means either the satellite is in a
   downlink it reserved there, or (when `rules.opportunistic_uplink` is true)
   some antenna isn't reserved by another satellite at that moment.
3. **Supersession.** When plan *P* is uplinked, every still-scheduled onboard
   activity that starts at or after `P.start` is dropped (`superseded`) and
   its reservation is released. A plan that loses all its remaining
   activities becomes `superseded`. To replace a satellite's schedule from
   time *T*, send a plan that starts at *T*.
4. Activities in *P* whose start time has already passed when it's uplinked
   are `skipped`, with reason `uplinked_after_start`.
5. Ground-side antenna reservations exist from the moment a plan is accepted.
   They're released when the activity finishes or is skipped, superseded or
   cancelled.

**Activity outcomes** (`activity.status`, `activity.result`)

| status | meaning |
|---|---|
| `pending` | plan not onboard yet |
| `scheduled` / `executing` | onboard |
| `completed` | ran fully |
| `partial` | produced data, but a constraint hit (`result.reasons`: `storage_full`, `sun_too_low`, `station_not_in_view`, `safe_mode`) |
| `failed` | ran, produced nothing |
| `skipped` | `uplinked_after_start`, `never_uplinked` or `cancelled` |
| `superseded` | replaced by a newer plan |

`result` = `{"data_mb": …, "reasons": [...], "image_ids": [...]}`.

### 6.4 Endpoints

| Method | Path | Body → Response |
|---|---|---|
| POST | `/plans/validate` | `PlanSubmission` → `PlanValidation` |
| POST | `/plans` | `PlanSubmission` → **201** `Plan` (422/409 on violation) |
| POST | `/plans/batch` | `{plans: PlanSubmission[], atomic: bool}` → `PlanBatchResult {accepted, rejected, items[{index, plan?, validation}]}`. Non-atomic batches are validated in order, so later plans see earlier plans' reservations. |
| GET | `/plans?satellite_id&status=…` | `Plan[]` |
| GET | `/plans/{id}` | `Plan` (activities include live status/result) |
| DELETE | `/plans/{id}` | cancels a `queued` plan → `Plan`, otherwise 409 |

---

## 7. Demand: imaging requests

Point targets with a value, a deadline and a cloud tolerance.

| Method | Path | |
|---|---|---|
| POST | `/requests` | `ImagingRequestCreate {name?, target{lat_deg,lon_deg}, priority, window_start?, window_end, max_cloud_fraction}` → 201 `ImagingRequest` |
| GET | `/requests?status=` | `ImagingRequest[]` |
| GET | `/requests/{id}` | `ImagingRequest` |
| DELETE | `/requests/{id}` | cancel |

Status flow: `open → captured` (imaged within the window, cloud ≤ max) `→
fulfilled` (that image fully downlinked by `window_end`), or `→ expired`.

---

## 8. Products, events, metrics, weather

| Method | Path | Response |
|---|---|---|
| GET | `/images?satellite_id&status&since&offset&limit` | `ImagePage`. Each `Image` is one continuous strip: `start/end`, `size_mb`, `downlinked_mb`, `status (onboard\|downlinked\|lost)`, `cloud_fraction`, `centerline [[lon,lat]…]`, `swath_km`, `request_ids` |
| GET | `/events?after_seq&limit&type=…&satellite_id` | `EventPage {events[], next_after_seq, has_more}`. `seq` is monotonic. Keep the last `seq` you saw and poll with `after_seq`. |
| GET | `/metrics?window_s=86400` | `Metrics` |
| GET | `/coverage?resolution_deg=1&since` | `CoverageGrid`: `last_imaged_s[row][col]` (seconds since epoch, or null). Row 0 is the north edge, col 0 is -180°. |
| POST | `/weather/cloud-forecast` | `{points[], times[]}` → `{forecasts[{point,time,issued_at,cloud_fraction,sigma}]}`. `sigma` grows with lead time. Truth is only revealed in captured images. |

**Event types**: `sim_reset`, `contact_start`, `contact_end`, `telemetry_received`,
`plan_queued`, `plan_uplinked`, `plan_activated`, `plan_completed`, `plan_superseded`,
`plan_expired`, `plan_cancelled`, `activity_started`, `activity_ended`,
`image_captured`, `image_downlinked`, `request_created`, `request_captured`,
`request_fulfilled`, `request_expired`, `storage_full`, `battery_low`,
`safe_mode_entered`, `safe_mode_exited`.

---

## 9. Implementing this in the Vulkan simulator

- The simulator must implement every path in `openapi.json` with the same JSON
  shapes. `python -m pytest python/tests` runs against the mock. To point the
  suite (or your planners) at the real simulator, construct
  `SimClient("http://localhost:8080")`.
- **Required for planning**: §3, §4, §6, `/events`, `/metrics`.
  **Nice to have**: `/access/*` (planners can compute these themselves from
  `orbit`), `/coverage`, `/weather`.
- Run the HTTP server on its own thread and hand commands to the sim thread
  through a queue. Copy state snapshots out under a lock or double buffer, so
  that `/constellation` never blocks rendering.
- Things the mock simplifies that the real sim should do better: SGP4 or a
  numerical propagator with drag (Doves use **differential drag** for
  phasing); WGS-84 Earth; a land mask for coverage; swath-polygon coverage
  instead of centerline cells; uplink occupying antenna time; activity
  boundaries finer than the 10 s step; a real cloud field.
- Possible v2 additions: polygon AOIs, a websocket event stream (`/ws/events`),
  differential-drag attitude activities, per-image priority downlink order,
  stochastic anomalies (missed passes, safe-mode faults).
