"""Flight-noise estimator: how loud is the plane overhead, right now?"""

from .acoustics import NoiseEstimate, estimate_aircraft_noise
from .config import Location, default_location
from .ducker import DuckConfig, VolumeDucker
from .estimator import Snapshot, estimate_from_aircraft, estimate_live
from .opensky import Aircraft, OpenSkyClient
from .roku import RokuTV

__all__ = [
    "NoiseEstimate",
    "estimate_aircraft_noise",
    "Location",
    "default_location",
    "Snapshot",
    "estimate_from_aircraft",
    "estimate_live",
    "Aircraft",
    "OpenSkyClient",
    "DuckConfig",
    "VolumeDucker",
    "RokuTV",
]
__version__ = "1.0.0"
