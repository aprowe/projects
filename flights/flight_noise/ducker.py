"""Map aircraft loudness to a smooth Roku volume "reverse duck".

A normal audio duck *lowers* one source when another appears. Here we do the
reverse: when a plane gets loud overhead and masks the TV, we *raise* the TV
volume to compensate, then ramp it back down as the plane passes.

The mapping is two-stage:

1. ``target_steps(dba)`` turns the current loudest aircraft level into a desired
   number of volume steps above the user's baseline (linear between a floor and
   a ceiling dBA).
2. ``update(dba)`` moves the *applied* offset toward that target by at most
   ``max_step_per_tick`` each call, which is what makes the change smooth rather
   than jumping the volume around abruptly.

The controller never sets absolute volume; it only ever returns a relative delta
and remembers the cumulative offset, so it can always restore the baseline.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass
class DuckConfig:
    db_floor: float = 60.0       # at/below this, no boost
    db_ceiling: float = 95.0     # at/above this, full boost
    max_boost_steps: int = 12    # max volume steps above baseline
    max_step_per_tick: int = 2   # smoothing: cap change applied per update


class VolumeDucker:
    def __init__(self, config: DuckConfig | None = None):
        self.cfg = config or DuckConfig()
        self.applied = 0  # current offset above baseline, in volume steps

    def target_steps(self, dba: float | None) -> int:
        """Desired offset above baseline for the given loudest level (dBA)."""
        c = self.cfg
        if dba is None or dba <= c.db_floor:
            return 0
        if dba >= c.db_ceiling:
            return c.max_boost_steps
        frac = (dba - c.db_floor) / (c.db_ceiling - c.db_floor)
        return round(frac * c.max_boost_steps)

    def update(self, dba: float | None) -> int:
        """Advance one tick toward the target. Returns the delta to apply.

        Positive = raise volume, negative = lower it. Updates the internal
        offset by the (smoothing-limited) delta.
        """
        target = self.target_steps(dba)
        delta = target - self.applied
        cap = self.cfg.max_step_per_tick
        delta = max(-cap, min(cap, delta))
        self.applied += delta
        return delta

    def reset(self) -> int:
        """Return the delta needed to restore baseline, and clear the offset."""
        delta = -self.applied
        self.applied = 0
        return delta
