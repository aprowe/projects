# Flight Noise Estimator

Estimate **how loud an aircraft is, right now, at a specific street address.**

The tool pulls live aircraft positions from a flight-tracker API, then runs each
plane through a transparent acoustic model that uses its **slant distance,
altitude, size class, climb state, and airspeed** to estimate the A-weighted
sound level (dBA) heard on the ground.

Default target address: **5071 Lotus Street, San Diego, CA 92107** (Point Loma),
which sits directly under the departure path of San Diego International (KSAN)
runway 27 — a great real-world test case.

```
Listener: 5071 Lotus Street, San Diego, CA 92107
          (32.74480, -117.24170), ground ~30 m MSL

Combined audible level (all aircraft): 86.2 dBA (2 audible of 6 tracked)

CALLSIGN      dBA  ALT(ft)  SLANT(ft)  PERCEIVED
------------------------------------------------
SWA1234      86.0     1903       1987  very loud (disruptive, like a vacuum cleaner up close)
HEMS3        73.6      951       1354  moderate (normal conversation level)
UAL55        39.8    19915      22422  below ambient (inaudible)
...
```

## Quick start

No dependencies — pure Python 3 standard library.

```bash
cd flights

# Offline demo with bundled sample data (no network/credentials needed):
python3 -m flight_noise --demo

# Show the per-aircraft dB breakdown:
python3 -m flight_noise --demo -v

# Live data (see "Flight data" below for credentials):
python3 -m flight_noise

# A different address, refreshing every 30 s:
python3 -m flight_noise -a "1 Infinite Loop, Cupertino, CA" --watch 30
```

## Flight data (OpenSky Network)

Live aircraft come from the free [OpenSky Network](https://opensky-network.org/)
API. As of 2025 OpenSky requires **OAuth2 client credentials** (anonymous access
is heavily rate-limited and usually returns HTTP 403).

1. Make a free account at <https://opensky-network.org/>.
2. Under **Account → API Client**, create a client and copy its id/secret.
3. Export them before running:

```bash
export OPENSKY_CLIENT_ID=your-client-id
export OPENSKY_CLIENT_SECRET=your-client-secret
python3 -m flight_noise
```

Without credentials the tool still tries anonymous access, and `--demo` always
works fully offline.

## How the noise model works

For each aircraft the model builds the level from documented dB terms
(`flight_noise/acoustics.py`):

```
Lp = Lw + Gthrust + Gframe − Aspread − Aatm − Aground
```

| Term | Meaning | Driver |
|------|---------|--------|
| `Lw` | Source sound **power** level | Aircraft **size class** (ADS-B emitter category): light GA ≈ 125 dB, narrowbody jet ≈ 150 dB, heavy widebody ≈ 158 dB |
| `Gthrust` | Engine-load adjustment | **Climb rate + altitude** — takeoff/climb thrust adds up to +6 dB; approach adds +2 dB |
| `Gframe` | Airframe noise | **Airspeed** (minor term) |
| `Aspread` | Geometric spreading loss | **Slant distance** — the dominant term, ~6 dB per doubling |
| `Aatm` | Atmospheric absorption | Distance (~5 dB/km) |
| `Aground` | Excess ground attenuation | **Elevation angle** — planes low on the horizon lose up to 12 dB |

**Geometry.** The slant distance is `√(horizontal² + altitude²)`, where the
horizontal distance is the haversine ground distance between the address and the
aircraft, and altitude is height above the address's ground elevation.

**Calibration.** `Lw` values are set so a 737-class jet climbing out at ~1000 ft
slant range reads ~88 dBA, consistent with published residential departure-noise
measurements. They are deliberately easy to tune in one place.

When several aircraft are audible, the combined level is computed by **incoherent
(energy) addition**, not by simply taking the loudest.

> This is an estimate, not a calibrated measurement. It ignores per-type engine
> data, terrain shadowing, wind, temperature gradients, and reflections. Treat it
> as a physically-reasonable approximation, not a regulatory sound reading.

## Project layout

```
flights/
├── flight_noise/
│   ├── acoustics.py   # the dBA model (pure, well-documented)
│   ├── opensky.py     # OpenSky API client (OAuth2 + anonymous), Aircraft model
│   ├── geocode.py     # address -> lat/lon (Nominatim, with fallback)
│   ├── config.py      # default address & search box
│   ├── estimator.py   # fetch + rank into a Snapshot
│   └── cli.py         # command-line interface
├── data/sample_states.json   # offline demo data
└── tests/                     # unit tests (no network)
```

## Tests

```bash
cd flights
python3 -m unittest discover -s tests -v
```

## CLI options

```
-a, --address     address to monitor (default: 5071 Lotus Street, San Diego)
    --lat/--lon   use exact coordinates (skip geocoding)
    --elevation   ground elevation at the address (m MSL, default 30)
    --bbox        half-size of the search box in degrees (default 0.25 ≈ 28 km)
-n, --top         max aircraft to list (default 10)
-v, --verbose     show the per-aircraft dB breakdown
    --demo        use bundled sample data (offline)
    --watch SECS  refresh continuously every SECS
```
