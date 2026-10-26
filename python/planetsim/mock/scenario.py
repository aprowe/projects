"""Default scenario: a PlanetScope-like SuperDove constellation.

Numbers are *illustrative*, loosely based on public descriptions of PlanetScope
(3U SuperDoves in ~475-525 km sun-synchronous orbits, morning descending node,
8-band PSB.SD imager, ~3-4 m GSD, ~32 km swath, nadir-only, X-band downlink to
a mostly high-latitude ground network). Tune them freely; they are chosen so
that storage, downlink capacity and antenna contention all bite.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .. import models as m

SUPERDOVE = m.SpacecraftSpec(
    gsd_m=3.7,
    swath_km=32.5,
    storage_capacity_mb=250_000.0,
    imaging_data_rate_mbps=400.0,
    downlink_rate_mbps=1600.0,
    battery_capacity_wh=60.0,
    solar_power_w=30.0,
    idle_power_w=8.0,
    imaging_power_w=22.0,
    downlink_power_w=28.0,
    min_sun_elevation_deg=10.0,
    transition_time_s=60.0,
)

RULES = m.SimRules(
    plan_max_horizon_s=72 * 3600.0,
    plan_max_activities=2000,
    uplink_duration_s=60.0,
    opportunistic_uplink=True,
    telemetry_on_contact=True,
    safe_mode_enter_battery_frac=0.10,
    safe_mode_exit_battery_frac=0.40,
    downlink_order="fifo",
)


@dataclass
class PlaneDef:
    name: str
    count: int
    altitude_km: float
    ltdn_hours: float  # local time of descending node


@dataclass
class StationDef:
    id: str
    name: str
    lat: float
    lon: float
    antennas: int = 2
    min_el: float = 10.0
    alt_m: float = 0.0
    max_downlink_rate_mbps: float | None = None


@dataclass
class ScenarioDef:
    name: str
    description: str
    planes: list[PlaneDef]
    stations: list[StationDef]
    spec: m.SpacecraftSpec = field(default_factory=lambda: SUPERDOVE)
    rules: m.SimRules = field(default_factory=lambda: RULES)
    step_s: float = 10.0
    extra: dict = field(default_factory=dict)


# Ground stations: a plausible, high-latitude-heavy network. Not Planet's
# actual site list.
STATIONS = [
    StationDef("svalbard", "Svalbard, NO", 78.23, 15.39, antennas=3),
    StationDef("inuvik", "Inuvik, CA", 68.32, -133.55),
    StationDef("fairbanks", "Fairbanks, US", 64.86, -147.85),
    StationDef("keflavik", "Keflavik, IS", 63.98, -22.62),
    StationDef("bremen", "Bremen, DE", 53.08, 8.80),
    StationDef("grand-forks", "Grand Forks, US", 47.92, -97.03),
    StationDef("singapore", "Singapore, SG", 1.35, 103.82, antennas=1),
    StationDef("hartebeesthoek", "Hartebeesthoek, ZA", -25.89, 27.69),
    StationDef("dubbo", "Dubbo, AU", -32.24, 148.60),
    StationDef("awarua", "Awarua, NZ", -46.53, 168.38),
    StationDef("punta-arenas", "Punta Arenas, CL", -53.00, -70.85),
    StationDef("troll", "Troll, AQ", -72.01, 2.53, antennas=1),
]

SCENARIOS: dict[str, ScenarioDef] = {
    "planetscope-default": ScenarioDef(
        name="planetscope-default",
        description="~180 SuperDove-like 3U cubesats in three morning SSO planes, 12 ground stations.",
        planes=[
            PlaneDef("flock-a", 60, 475.0, 9.75),
            PlaneDef("flock-b", 60, 500.0, 10.25),
            PlaneDef("flock-c", 60, 525.0, 10.75),
        ],
        stations=STATIONS,
    ),
}
