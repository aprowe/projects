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

## Reverse-ducking a Roku TV

When a plane gets loud overhead it *masks* your TV audio. With `--roku`, the tool
**boosts the Roku's volume to compensate, then smoothly ramps it back down** as
the plane passes — a "reverse duck" (a normal duck lowers a source; this raises
it). Works best paired with `--watch`.

Roku TVs are controlled over the [External Control Protocol](https://developer.roku.com/docs/developer-program/dev-tools/external-control-api.md)
(HTTP on port 8060). They only support *relative* `VolumeUp`/`VolumeDown`
keypresses, so the tool tracks an offset relative to your current volume and
always restores it on exit (Ctrl-C).

```bash
# Try it offline first — prints the volume keypresses instead of sending them:
python3 -m flight_noise --demo --roku --roku-dry-run --watch 3

# For real, against your TV (auto-discovers via SSDP, or pass --roku-ip):
python3 -m flight_noise --watch 10 --roku --roku-ip 192.168.1.23
```

How the boost is chosen (all tunable):

| Flag | Default | Meaning |
|------|---------|---------|
| `--duck-floor` | 60 dBA | at/below this, no boost |
| `--duck-ceiling` | 95 dBA | at/above this, full boost |
| `--duck-max-steps` | 12 | most volume steps it will add above baseline |
| `--duck-rate` | 2 | max steps changed per refresh (controls how *smooth*) |

The desired boost scales linearly between the floor and ceiling based on the
loudest audible aircraft; each refresh the applied offset moves toward that
target by at most `--duck-rate` steps, so the volume eases up and down rather
than jumping. Smaller `--watch` intervals + a modest `--duck-rate` give the
smoothest ramp.

> Set `ROKU_IP` in your environment to skip discovery. The TV and the machine
> running this must be on the same LAN.

## Project layout

```
flights/
├── flight_noise/
│   ├── acoustics.py   # the dBA model (pure, well-documented)
│   ├── opensky.py     # OpenSky API client (OAuth2 + anonymous), Aircraft model
│   ├── geocode.py     # address -> lat/lon (Nominatim, with fallback)
│   ├── config.py      # default address & search box
│   ├── estimator.py   # fetch + rank into a Snapshot
│   ├── roku.py        # Roku TV ECP client (relative volume + SSDP discovery)
│   ├── ducker.py      # loudness -> smooth volume-offset controller
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
    --roku        reverse-duck a Roku TV's volume as planes pass
    --roku-ip     Roku IP (else ROKU_IP env, else SSDP discovery)
    --roku-dry-run  print volume keypresses instead of sending them
    --duck-floor / --duck-ceiling / --duck-max-steps / --duck-rate
                  tune the loudness -> volume-boost mapping
```
