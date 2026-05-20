// Rolling latency tracker. Keeps a fixed-size ring of the most recent
// samples per stage and exposes mean / p50 / p99 / max snapshots.
//
// Samples are stored as plain Numbers in microseconds. A 256-sample window
// at 60 fps is ~4 s of history, which is enough to surface jank without
// smoothing transient spikes into invisibility.

export class LatencyTracker {
  constructor(capacity = 256) {
    this.cap = capacity;
    this.stages = {
      decodeToSend: new RingBuffer(capacity),
      sendToRecv: new RingBuffer(capacity),
      recvToPaint: new RingBuffer(capacity),
      total: new RingBuffer(capacity),
    };
  }

  record(sample) {
    for (const k of Object.keys(this.stages)) {
      const v = sample[k];
      // Negative samples mean a clock went backwards (e.g. wall-clock NTP
      // adjust). Drop them — they contaminate percentiles more than they
      // inform.
      if (Number.isFinite(v) && v >= 0) {
        this.stages[k].push(v);
      }
    }
  }

  snapshot() {
    const out = { n: this.stages.total.length };
    if (out.n === 0) return null;
    for (const [k, ring] of Object.entries(this.stages)) {
      out[k] = ring.stats();
    }
    return out;
  }
}

class RingBuffer {
  constructor(cap) {
    this.cap = cap;
    this.buf = new Float64Array(cap);
    this.idx = 0;
    this.length = 0;
  }

  push(v) {
    this.buf[this.idx] = v;
    this.idx = (this.idx + 1) % this.cap;
    if (this.length < this.cap) this.length++;
  }

  stats() {
    const n = this.length;
    if (n === 0) return { mean: 0, p50: 0, p99: 0, max: 0 };

    // Copy out the valid slice and sort for percentiles.
    const sorted = this.buf.slice(0, n).sort();
    let sum = 0;
    for (let i = 0; i < n; i++) sum += sorted[i];
    return {
      mean: sum / n,
      p50: sorted[Math.floor(n * 0.5)],
      p99: sorted[Math.min(n - 1, Math.floor(n * 0.99))],
      max: sorted[n - 1],
    };
  }
}

export function fmtUs(us) {
  if (us < 1000) return `${us.toFixed(0)}µs`;
  if (us < 1_000_000) return `${(us / 1000).toFixed(2)}ms`;
  return `${(us / 1_000_000).toFixed(2)}s`;
}
