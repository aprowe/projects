"""Mock simulator: a lightweight reference implementation of the v1 API."""

from __future__ import annotations

from typing import Any


def inprocess_client(**reset: Any):
    """A SimClient wired straight to an in-process mock server (no network, no uvicorn).

    ``reset`` kwargs go to :class:`planetsim.models.SimReset` (seed, num_satellites, random_requests, ...).
    """
    from fastapi.testclient import TestClient

    from .. import models as m
    from ..client import SimClient
    from .engine import Engine
    from .server import create_app

    http = TestClient(create_app(Engine(m.SimReset(**reset))))
    http._planetsim_inprocess = True
    return SimClient(http=http)
