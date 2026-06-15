"""Physically grounded model for estimating aircraft noise at a ground location.

The model takes a single aircraft's position and dynamics (from a flight
tracker) plus the listener's location, and estimates the A-weighted sound
pressure level (dBA) heard on the ground.

It is intentionally transparent: every step is a documented dB term so the
estimate can be reasoned about and tuned, rather than a black box.

Chain of reasoning
------------------
    Lp = Lw  +  Gthrust  +  Gframe  -  Aspread  -  Aatm  -  Aground

  * Lw       sound power level of the source (set by aircraft size class)
  * Gthrust  engine-load adjustment (takeoff/climb is far louder than cruise)
  * Gframe   airframe noise adjustment from airspeed (minor)
  * Aspread  geometric spreading loss with slant distance (the dominant term)
  * Aatm     atmospheric absorption (roughly proportional to distance)
  * Aground  excess attenuation when the plane is low on the horizon

The Lw values are calibrated so that a "Large" jet (e.g. a 737) climbing out
at ~1000 ft (305 m) slant range registers ~88 dBA at the listener, which is
consistent with published departure-noise measurements over residential areas.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

# --- Geometry ----------------------------------------------------------------

EARTH_RADIUS_M = 6_371_000.0


def haversine_m(lat1: float, lon1: float, lat2: float, lon2: float) -> float:
    """Great-circle ground distance between two lat/lon points, in metres."""
    p1, p2 = math.radians(lat1), math.radians(lat2)
    dphi = math.radians(lat2 - lat1)
    dlmb = math.radians(lon2 - lon1)
    a = math.sin(dphi / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dlmb / 2) ** 2
    return 2 * EARTH_RADIUS_M * math.asin(math.sqrt(a))


# --- Source levels by aircraft size class ------------------------------------
#
# Keyed by the OpenSky ADS-B "category" code. Values are approximate broadband
# A-weighted sound *power* levels (dB re 1 pW) at a nominal operating thrust.
#
#   0  No information           1  No ADS-B category
#   2  Light (<15500 lb)        3  Small (15500-75000 lb)
#   4  Large (75000-300000 lb)  5  High-Vortex Large (e.g. B757)
#   6  Heavy (>300000 lb)       7  High Performance (>5g, >400 kt)
#   8  Rotorcraft               9+ Gliders/UAV/ground/etc.
SOUND_POWER_BY_CATEGORY = {
    0: 138.0,   # unknown -> assume a small/medium aircraft
    1: 138.0,
    2: 125.0,   # light GA (Cessna-class)
    3: 140.0,   # small jet / turboprop / regional
    4: 150.0,   # large narrowbody (737 / A320)
    5: 153.0,   # high-vortex large (757)
    6: 158.0,   # heavy widebody (777 / 747 / A350)
    7: 150.0,   # high performance / military jet
    8: 142.0,   # helicopter
}
DEFAULT_SOUND_POWER = 138.0

# Atmospheric absorption: representative A-weighted broadband value (~5 dB/km).
ATMOSPHERIC_ABS_DB_PER_M = 0.005

# Ambient acoustic floor of a typical residential neighbourhood (dBA). Below
# this, an aircraft is effectively inaudible against background noise.
AMBIENT_FLOOR_DBA = 45.0


@dataclass
class NoiseEstimate:
    """Result of estimating one aircraft's noise at the listener."""

    callsign: str
    icao24: str
    latitude: float
    longitude: float
    heading: float | None
    slant_distance_m: float
    horizontal_distance_m: float
    altitude_m: float
    dba: float
    audible: bool
    description: str
    components: dict  # per-term dB breakdown, for transparency

    @property
    def slant_distance_ft(self) -> float:
        return self.slant_distance_m * 3.28084


def _thrust_gain(altitude_agl_m: float, vertical_rate_mps: float | None) -> float:
    """Engine-load adjustment in dB.

    Takeoff/climb thrust is dramatically louder than cruise or descent. We
    infer engine load from how low and how steeply climbing the aircraft is.
    """
    vr = vertical_rate_mps or 0.0
    gain = 0.0
    low = altitude_agl_m < 1500  # within the noisy departure/arrival corridor
    if low and vr > 1.0:
        # Climbing out at high thrust: scale up to +6 dB the steeper the climb.
        gain += min(6.0, 6.0 * (vr / 12.0))
    elif low and vr < -1.0:
        # On approach: reduced thrust but added gear/flap airframe noise.
        gain += 2.0
    return gain


def _airframe_gain(velocity_mps: float | None) -> float:
    """Minor airframe-noise term that grows with airspeed."""
    v = velocity_mps or 0.0
    if v <= 0:
        return 0.0
    # Reference 100 m/s (~195 kt); ~ +3 dB per doubling of speed, clamped.
    return max(-3.0, min(4.0, 10.0 * math.log10(max(v, 1.0) / 100.0)))


def _ground_attenuation(altitude_agl_m: float, horizontal_m: float) -> float:
    """Excess attenuation when the source is low on the horizon.

    Sound from an aircraft near the horizon is attenuated by ground effect and
    terrain/buildings far more than sound coming from directly overhead.
    """
    # Elevation angle above the horizon, in degrees.
    angle = math.degrees(math.atan2(max(altitude_agl_m, 0.0), max(horizontal_m, 1.0)))
    if angle >= 30:
        return 0.0
    if angle <= 1:
        return 12.0
    # Linear ramp from 0 dB (30 deg) up to 12 dB (1 deg).
    return 12.0 * (30.0 - angle) / 29.0


def describe_level(dba: float) -> str:
    """Human-readable description of a dBA level."""
    if dba < AMBIENT_FLOOR_DBA:
        return "below ambient (inaudible)"
    if dba < 55:
        return "faint hum"
    if dba < 65:
        return "noticeable"
    if dba < 75:
        return "moderate (normal conversation level)"
    if dba < 85:
        return "loud (busy street)"
    if dba < 95:
        return "very loud (disruptive, like a vacuum cleaner up close)"
    return "extremely loud (hearing-damage range with exposure)"


def estimate_aircraft_noise(
    aircraft,
    listener_lat: float,
    listener_lon: float,
    listener_elevation_m: float = 0.0,
) -> NoiseEstimate:
    """Estimate the dBA produced by one ``Aircraft`` at the listener's location."""
    horiz = haversine_m(listener_lat, listener_lon, aircraft.latitude, aircraft.longitude)

    altitude_msl = aircraft.altitude_m if aircraft.altitude_m is not None else 0.0
    altitude_agl = max(altitude_msl - listener_elevation_m, 0.0)

    slant = math.sqrt(horiz ** 2 + altitude_agl ** 2)
    slant = max(slant, 1.0)  # guard against log(0) directly overhead/at sensor

    lw = SOUND_POWER_BY_CATEGORY.get(aircraft.category, DEFAULT_SOUND_POWER)
    g_thrust = _thrust_gain(altitude_agl, aircraft.vertical_rate_mps)
    g_frame = _airframe_gain(aircraft.velocity_mps)

    # Spherical spreading from a point source: Lp = Lw - 20log10(r) - 11.
    a_spread = 20 * math.log10(slant) + 11.0
    a_atm = ATMOSPHERIC_ABS_DB_PER_M * slant
    a_ground = _ground_attenuation(altitude_agl, horiz)

    dba = lw + g_thrust + g_frame - a_spread - a_atm - a_ground

    components = {
        "sound_power_Lw": round(lw, 1),
        "thrust_gain": round(g_thrust, 1),
        "airframe_gain": round(g_frame, 1),
        "spreading_loss": round(-a_spread, 1),
        "atmospheric_loss": round(-a_atm, 1),
        "ground_attenuation": round(-a_ground, 1),
    }

    return NoiseEstimate(
        callsign=aircraft.callsign or aircraft.icao24,
        icao24=aircraft.icao24,
        latitude=aircraft.latitude,
        longitude=aircraft.longitude,
        heading=aircraft.true_track,
        slant_distance_m=slant,
        horizontal_distance_m=horiz,
        altitude_m=altitude_agl,
        dba=round(dba, 1),
        audible=dba >= AMBIENT_FLOOR_DBA,
        description=describe_level(dba),
        components=components,
    )
