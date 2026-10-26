"""python -m planetsim.mock [--host 127.0.0.1] [--port 8080] [--seed 0] [--requests 0]"""

import argparse

import uvicorn

from .. import models as m
from .engine import Engine
from .server import create_app


def main() -> None:
    ap = argparse.ArgumentParser(description="Mock constellation simulator (reference implementation of the API).")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8080)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--satellites", type=int, default=None, help="truncate the constellation")
    ap.add_argument("--requests", type=int, default=0, help="random imaging requests to generate")
    a = ap.parse_args()
    eng = Engine(m.SimReset(seed=a.seed, num_satellites=a.satellites, random_requests=a.requests))
    uvicorn.run(create_app(eng), host=a.host, port=a.port)


if __name__ == "__main__":
    main()
