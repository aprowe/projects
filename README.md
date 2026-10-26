# Constellation Simulator

A practice environment for **Earth-observation mission planning**, modelled on
Planet's PlanetScope (SuperDove) constellation. It comes in two parts:

1. **Simulator + visualizer** (Vulkan, *to be written*). It owns the clock,
   orbits, satellites, ground stations and the truth, and serves an HTTP API.
2. **Python interface** (this repo, `python/planetsim`). It's a typed client
   for that API, so you can write schedulers that read the constellation's
   state and submit plans.

The plans are realistic in one key way: **they only reach a satellite when it
passes over a ground station.** You submit to the ground segment, and the
uplink happens at the next contact.

```
.
├── api/openapi.json          # the contract (generated, don't edit by hand)
├── docs/API.md               # concepts, rules, plan lifecycle, endpoint reference  ← start here
└── python/
    ├── planetsim/
    │   ├── models.py         # pydantic wire models = source of truth for the API
    │   ├── client.py         # SimClient, PlanBuilder
    │   └── mock/             # reference server (FastAPI + simple physics) for use until the Vulkan sim exists
    ├── examples/greedy_scheduler.py   # baseline scheduler to beat
    ├── scripts/export_openapi.py
    └── tests/
```

## Quick start

```bash
cd python
pip install -e '.[dev]'

# option A: in-process mock, no server
python examples/greedy_scheduler.py --inprocess

# option B: run the mock as a real HTTP server (later: the Vulkan sim on the same port)
python -m planetsim.mock --port 8080 --requests 300 &
python examples/greedy_scheduler.py --url http://localhost:8080

pytest tests
```

```python
from datetime import timedelta
from planetsim import SimClient, PlanBuilder

sim = SimClient("http://localhost:8080")
sim.reset(seed=1, random_requests=200)
now = sim.now()

world = sim.constellation()                       # every satellite + every station
passes = sim.contacts(now, now + timedelta(hours=6), station_ids=["svalbard"])
c = passes[0]

plan = (PlanBuilder(c.satellite_id, now, now + timedelta(hours=6))
        .downlink(c.station_id, c.aos, c.los)
        .build())
print(sim.validate_plan(plan))                    # dry run
sim.submit_plan(plan)                             # queued -> uplinked at next contact

sim.step(timedelta(hours=6))                      # deterministic while paused
print(sim.metrics())
for ev in sim.iter_events(types=["plan_uplinked"]):
    print(ev.time, ev.satellite_id, ev.plan_id)
```

## Baseline numbers (greedy scheduler, 180 sats, 24 h, mock)

```
images=2410 downlinked=1767 onboard=35.6 TB coverage=84.5% mean latency=6.4 h requests 75/300
```

Storage is saturated and downlink is the bottleneck, so there's lots of room
to improve. Ideas to practise:

- Downlink assignment as weighted interval scheduling or max-flow over
  (satellite, station, antenna, time) slots, solved with a MILP or CP-SAT.
- Plan around uplink latency: start each satellite's plan at its next uplink
  opportunity.
- Choose imaging by value: land, requests, cloud forecasts, and how long ago
  each cell was last imaged.
- Keep plans power-feasible, especially for eclipse downlinks.
- Rolling-horizon replanning driven by `/events` and telemetry instead of truth.

## Default scenario (illustrative numbers, tune in `mock/scenario.py`)

| | |
|---|---|
| Satellites | 180 SuperDove-like 3U, 3 SSO planes (475/500/525 km, LTDN 09:45 / 10:15 / 10:45) |
| Imager | nadir-only, 32.5 km swath, 3.7 m GSD, 400 Mbps |
| Storage / downlink | 250 GB / 1600 Mbps X-band |
| Power | 60 Wh battery, 30 W solar, 8/22/28 W idle/imaging/downlink |
| Ground | 12 stations (Svalbard, Inuvik, Fairbanks, Keflavik, Bremen, Grand Forks, Singapore, Hartebeesthoek, Dubbo, Awarua, Punta Arenas, Troll), 1–3 antennas each, 10° mask |
| Rules | 60 s slew between activities, 60 s contact needed to uplink, 72 h max plan |

These are based loosely on public descriptions of PlanetScope. They are not
Planet's actual figures or ground-station list.
