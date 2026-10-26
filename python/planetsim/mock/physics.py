"""Deliberately simple astrodynamics for the mock server.

* Circular orbits, two-body + J2 secular RAAN drift.
* Spherical Earth, GMST rotation only (no precession/nutation/polar motion).
* Low-precision analytic sun; cylindrical Earth shadow.

Good to a few km / a few seconds of contact timing — fine for practising
scheduling, not for real ops. The real simulator should replace all of this.
"""

from __future__ import annotations

import numpy as np

MU = 398600.4418  # km^3/s^2
RE = 6378.137  # km
J2 = 1.08262668e-3
OMEGA_E = 7.2921150e-5  # rad/s
SSO_RAAN_RATE = 2 * np.pi / (365.2422 * 86400.0)  # rad/s


def julian_date(unix_s: np.ndarray | float) -> np.ndarray | float:
    return np.asarray(unix_s) / 86400.0 + 2440587.5


def gmst_rad(unix_s):
    jd = julian_date(unix_s)
    deg = 280.46061837 + 360.98564736629 * (jd - 2451545.0)
    return np.deg2rad(np.mod(deg, 360.0))


def sun_eci_unit(unix_s):
    """Unit vector to the sun in (approx) ECI. Shape (..., 3)."""
    n = julian_date(unix_s) - 2451545.0
    L = np.deg2rad(np.mod(280.460 + 0.9856474 * n, 360))
    g = np.deg2rad(np.mod(357.528 + 0.9856003 * n, 360))
    lam = L + np.deg2rad(1.915) * np.sin(g) + np.deg2rad(0.020) * np.sin(2 * g)
    eps = np.deg2rad(23.439 - 4.0e-7 * n)
    return np.stack([np.cos(lam), np.cos(eps) * np.sin(lam), np.sin(eps) * np.sin(lam)], axis=-1)


def sun_ra_rad(unix_s: float) -> float:
    s = sun_eci_unit(unix_s)
    return float(np.arctan2(s[1], s[0]))


def mean_motion(a_km):
    return np.sqrt(MU / np.asarray(a_km) ** 3)


def j2_raan_rate(a_km, inc_rad):
    n = mean_motion(a_km)
    return -1.5 * n * J2 * (RE / a_km) ** 2 * np.cos(inc_rad)


def sso_inclination_rad(a_km: float) -> float:
    n = mean_motion(a_km)
    cos_i = -SSO_RAAN_RATE / (1.5 * n * J2 * (RE / a_km) ** 2)
    return float(np.arccos(cos_i))


def rot_z(vec, theta):
    """Rotate vectors (..., 3) by -theta about z (ECI -> ECEF when theta = GMST)."""
    c, s = np.cos(theta), np.sin(theta)
    x, y, z = vec[..., 0], vec[..., 1], vec[..., 2]
    return np.stack([c * x + s * y, -s * x + c * y, z], axis=-1)


class Orbits:
    """Vectorised circular orbits for N satellites."""

    def __init__(self, a_km, inc_rad, raan0_rad, u0_rad):
        self.a = np.asarray(a_km, float)
        self.inc = np.asarray(inc_rad, float)
        self.raan0 = np.asarray(raan0_rad, float)
        self.u0 = np.asarray(u0_rad, float)
        self.n = mean_motion(self.a)
        self.raan_dot = j2_raan_rate(self.a, self.inc)

    def __len__(self) -> int:
        return len(self.a)

    def angles(self, t_s, idx=None):
        """RAAN and argument of latitude at sim seconds t (broadcast: (N,1) x (T,))."""
        sl = slice(None) if idx is None else idx
        t = np.asarray(t_s, float)
        if t.ndim == 1:
            a = lambda x: x[sl][:, None]  # noqa: E731
        else:
            a = lambda x: x[sl]  # noqa: E731
        raan = a(self.raan0) + a(self.raan_dot) * t
        u = a(self.u0) + a(self.n) * t
        return raan, u

    def eci(self, t_s, idx=None):
        """Position and velocity, shape (N, 3) for scalar t or (N, T, 3) for array t."""
        sl = slice(None) if idx is None else idx
        raan, u = self.angles(t_s, idx)
        t = np.asarray(t_s, float)
        inc = self.inc[sl][:, None] if t.ndim == 1 else self.inc[sl]
        a = self.a[sl][:, None] if t.ndim == 1 else self.a[sl]
        n = self.n[sl][:, None] if t.ndim == 1 else self.n[sl]
        cO, sO, cu, su, ci, si = np.cos(raan), np.sin(raan), np.cos(u), np.sin(u), np.cos(inc), np.sin(inc)
        r = np.stack([cO * cu - sO * su * ci, sO * cu + cO * su * ci, su * si * np.ones_like(cO)], axis=-1) * a[..., None]
        v = np.stack([-cO * su - sO * cu * ci, -sO * su + cO * cu * ci, cu * si * np.ones_like(cO)], axis=-1) * (a * n)[..., None]
        return r, v


def geodetic_from_ecef(r):
    """Spherical-Earth lat/lon/alt (deg, deg, km)."""
    norm = np.linalg.norm(r, axis=-1)
    lat = np.rad2deg(np.arcsin(r[..., 2] / norm))
    lon = np.rad2deg(np.arctan2(r[..., 1], r[..., 0]))
    return lat, lon, norm - RE


def ecef_from_geodetic(lat_deg, lon_deg, alt_km=0.0):
    lat, lon = np.deg2rad(lat_deg), np.deg2rad(lon_deg)
    r = RE + np.asarray(alt_km)
    return np.stack([r * np.cos(lat) * np.cos(lon), r * np.cos(lat) * np.sin(lon), r * np.sin(lat) * np.ones_like(lon)], axis=-1)


def unit(v):
    return v / np.linalg.norm(v, axis=-1, keepdims=True)


def elevation_deg(sat_ecef, site_ecef):
    """Elevation of sat(s) seen from site(s); broadcasting over leading dims."""
    up = unit(site_ecef)
    rho = sat_ecef - site_ecef
    return np.rad2deg(np.arcsin(np.sum(rho * up, axis=-1) / np.linalg.norm(rho, axis=-1)))


def sun_elevation_deg(site_ecef, sun_ecef_unit):
    return np.rad2deg(np.arcsin(np.clip(np.sum(unit(site_ecef) * sun_ecef_unit, axis=-1), -1, 1)))


def in_sunlight(r_eci, sun_unit):
    proj = np.sum(r_eci * sun_unit, axis=-1)
    perp = np.linalg.norm(r_eci - proj[..., None] * sun_unit, axis=-1)
    return (proj > 0) | (perp > RE)


def angular_distance_rad(u1, u2):
    return np.arccos(np.clip(np.sum(u1 * u2, axis=-1), -1, 1))
