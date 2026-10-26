"""Regenerate ../api/openapi.json from the pydantic models + mock server routes."""

import json
from pathlib import Path

from planetsim.mock.server import create_app

out = Path(__file__).resolve().parents[2] / "api" / "openapi.json"
out.write_text(json.dumps(create_app().openapi(), indent=2) + "\n")
print(f"wrote {out}")
