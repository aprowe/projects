"use strict";
/*
 * Headless CFD renderer -> animated GIF.
 *
 * Runs the same stable-fluids Navier-Stokes solver as index.html, but with a
 * scripted scene: rain drops falling under gravity onto a solid dome. Each
 * frame is mapped through a 256-colour gradient palette straight to
 * GIF colour indices (no quantisation needed), then LZW-encoded into a looping
 * GIF89a. Pure Node, zero npm dependencies.
 *
 *   node render.js [frames] [cellPx]
 */

const fs = require("fs");

// ----------------------------- Solver --------------------------------
const N = 72;
const SIZE = (N + 2) * (N + 2);
const DT = 0.16, DIFF = 0.00002, VISC = 0.00002, ITER = 16;
const IX = (i, j) => i + (N + 2) * j;
const F = () => new Float32Array(SIZE);

let u = F(), v = F(), uPrev = F(), vPrev = F(), dens = F(), densPrev = F();

function addSource(x, s, dt) { for (let i = 0; i < SIZE; i++) x[i] += dt * s[i]; }
function setBnd(b, x) {
  for (let i = 1; i <= N; i++) {
    x[IX(0, i)]     = b === 1 ? -x[IX(1, i)] : x[IX(1, i)];
    x[IX(N + 1, i)] = b === 1 ? -x[IX(N, i)] : x[IX(N, i)];
    x[IX(i, 0)]     = b === 2 ? -x[IX(i, 1)] : x[IX(i, 1)];
    x[IX(i, N + 1)] = b === 2 ? -x[IX(i, N)] : x[IX(i, N)];
  }
  x[IX(0,0)]       = 0.5*(x[IX(1,0)]+x[IX(0,1)]);
  x[IX(0,N+1)]     = 0.5*(x[IX(1,N+1)]+x[IX(0,N)]);
  x[IX(N+1,0)]     = 0.5*(x[IX(N,0)]+x[IX(N+1,1)]);
  x[IX(N+1,N+1)]   = 0.5*(x[IX(N,N+1)]+x[IX(N+1,N)]);
}
function linSolve(b, x, x0, a, c) {
  const invC = 1 / c;
  for (let k = 0; k < ITER; k++) {
    for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++)
      x[IX(i,j)] = (x0[IX(i,j)] + a*(x[IX(i-1,j)]+x[IX(i+1,j)]+x[IX(i,j-1)]+x[IX(i,j+1)]))*invC;
    setBnd(b, x);
  }
}
function diffuse(b, x, x0, diff, dt) { const a = dt*diff*N*N; linSolve(b, x, x0, a, 1+4*a); }
function advect(b, d, d0, uu, vv, dt) {
  const dt0 = dt * N;
  for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++) {
    let x = i - dt0*uu[IX(i,j)], y = j - dt0*vv[IX(i,j)];
    if (x < 0.5) x = 0.5; if (x > N+0.5) x = N+0.5;
    if (y < 0.5) y = 0.5; if (y > N+0.5) y = N+0.5;
    const i0 = x|0, i1 = i0+1, j0 = y|0, j1 = j0+1;
    const s1 = x-i0, s0 = 1-s1, t1 = y-j0, t0 = 1-t1;
    d[IX(i,j)] = s0*(t0*d0[IX(i0,j0)]+t1*d0[IX(i0,j1)]) + s1*(t0*d0[IX(i1,j0)]+t1*d0[IX(i1,j1)]);
  }
  setBnd(b, d);
}
function project(uu, vv, p, divg) {
  const h = 1 / N;
  for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++) {
    divg[IX(i,j)] = -0.5*h*(uu[IX(i+1,j)]-uu[IX(i-1,j)]+vv[IX(i,j+1)]-vv[IX(i,j-1)]);
    p[IX(i,j)] = 0;
  }
  setBnd(0, divg); setBnd(0, p);
  linSolve(0, p, divg, 1, 4);
  for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++) {
    uu[IX(i,j)] -= 0.5*(p[IX(i+1,j)]-p[IX(i-1,j)])/h;
    vv[IX(i,j)] -= 0.5*(p[IX(i,j+1)]-p[IX(i,j-1)])/h;
  }
  setBnd(1, uu); setBnd(2, vv);
}
function densStep(dt) {
  addSource(dens, densPrev, dt);
  [dens, densPrev] = [densPrev, dens]; diffuse(0, dens, densPrev, DIFF, dt);
  [dens, densPrev] = [densPrev, dens]; advect(0, dens, densPrev, u, v, dt);
  for (let i = 0; i < SIZE; i++) dens[i] *= 0.997; // keep water "heavy" so it sinks all the way
}
function velStep(dt) {
  addSource(u, uPrev, dt); addSource(v, vPrev, dt);
  [u, uPrev] = [uPrev, u]; diffuse(1, u, uPrev, VISC, dt);
  [v, vPrev] = [vPrev, v]; diffuse(2, v, vPrev, VISC, dt);
  project(u, v, uPrev, vPrev);
  [u, uPrev] = [uPrev, u]; [v, vPrev] = [vPrev, v];
  advect(1, u, uPrev, uPrev, vPrev, dt);
  advect(2, v, vPrev, uPrev, vPrev, dt);
  project(u, v, uPrev, vPrev);
}

// --------------------------- Scripted scene --------------------------
// A faucet drips discrete droplets onto a solid dome. Each drop is a Lagrangian
// particle (a grid-based smoke solver smears small blobs away, so we track the
// drops directly) that falls under gravity, slides off the dome, and pools on
// the floor. Every frame the particles are stamped into the density field so
// the colour map and marching-squares contour render them as round liquid.
const DROP_G = 0.13;          // gravity on a droplet (grid cells / frame^2)
const DROP_R = 2.7;           // droplet render radius (cells)
const DRIP_PERIOD = 30;       // frames between drips: one drip = one drop
const DRIP_X = Math.round(N / 2) - 2, DRIP_Y = 5; // faucet just off the apex so drops roll off
const DOME_CX = N / 2, DOME_CY = N, DOME_R = N * 0.30;

const solid = new Uint8Array(SIZE);
function buildDome() {
  for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++) {
    const dx = i - DOME_CX, dy = j - DOME_CY;
    if (dx * dx + dy * dy <= DOME_R * DOME_R) solid[IX(i, j)] = 1;
  }
}

const drops = [];                       // active droplets {x, y, vx, vy}
const puddle = new Float32Array(N + 2); // water collected per floor column

function updateDrops(frame) {
  if (frame % DRIP_PERIOD === 0) drops.push({ x: DRIP_X, y: DRIP_Y, vx: 0, vy: 0.5 });

  const surf = DOME_R + DROP_R * 0.5;   // keep the drop centre this far off the dome
  for (const d of drops) {
    if (d.dead) continue;
    d.vy += DROP_G;                     // fall
    d.x += d.vx; d.y += d.vy;
    d.vx *= 0.98;

    // Dome contact: project back onto the surface and slide (remove the inward
    // velocity component, keep the tangential part) so the drop rolls off.
    const dx = d.x - DOME_CX, dy = d.y - DOME_CY, dist = Math.hypot(dx, dy) || 1e-6;
    if (dist < surf && d.y < DOME_CY) {
      const nx = dx / dist, ny = dy / dist;
      d.x = DOME_CX + nx * surf; d.y = DOME_CY + ny * surf;
      const vn = d.vx * nx + d.vy * ny;
      d.vx -= vn * nx; d.vy -= vn * ny;
      d.vx += nx * 0.06;                // nudge off the apex so it picks a side
    }
    if (d.x < 2) { d.x = 2; d.vx = Math.abs(d.vx) * 0.4; }
    if (d.x > N - 1) { d.x = N - 1; d.vx = -Math.abs(d.vx) * 0.4; }

    if (d.y >= N - 1) {                 // hit the floor -> add to the puddle
      const xi = Math.max(1, Math.min(N, Math.round(d.x)));
      puddle[xi] += 1; d.dead = true;
    }
  }
  for (let n = drops.length - 1; n >= 0; n--) if (drops[n].dead) drops.splice(n, 1);

  // Puddle levels out (shallow-water-ish smoothing) and drains slowly.
  for (let k = 0; k < 3; k++)
    for (let i = 2; i <= N - 1; i++)
      puddle[i] += 0.2 * (puddle[i - 1] + puddle[i + 1] - 2 * puddle[i]);
  for (let i = 1; i <= N; i++) puddle[i] *= 0.999;
}

// Rasterise the droplets + puddle into the density field used for rendering.
function buildDropField() {
  dens.fill(0);
  const inv = 1 / (DROP_R * DROP_R);
  for (const d of drops) {
    const ci = Math.round(d.x), cj = Math.round(d.y);
    for (let oj = -3; oj <= 3; oj++) for (let oi = -3; oi <= 3; oi++) {
      const i = ci + oi, j = cj + oj;
      if (i < 1 || i > N || j < 1 || j > N) continue;
      dens[IX(i, j)] += 3.2 * Math.exp(-(oi * oi + oj * oj) * inv); // round, smooth drop
    }
  }
  for (let i = 1; i <= N; i++) {
    const rows = Math.min(10, puddle[i] * 1.6);
    if (rows <= 0) continue;
    for (let j = N - Math.round(rows); j <= N; j++) {
      if (j < 1 || solid[IX(i, j)]) continue;
      dens[IX(i, j)] += 3.2;
    }
  }
}

// ----------------------- Colour palette (256) ------------------------
function buildPalette() {
  // dark navy -> blue -> cyan -> green -> yellow -> magenta -> white
  const stops = [
    [4, 6, 18], [20, 40, 120], [0, 170, 200], [80, 230, 150],
    [240, 225, 90], [248, 70, 130], [255, 255, 255],
  ];
  const pal = Buffer.alloc(256 * 3);
  for (let k = 0; k < 256; k++) {
    const t = k / 255 * (stops.length - 1);
    const a = Math.min(stops.length - 2, Math.floor(t)), f = t - a;
    for (let c = 0; c < 3; c++)
      pal[k*3 + c] = Math.round(stops[a][c] + (stops[a+1][c] - stops[a][c]) * f);
  }
  return pal;
}

// Map the field to a 0..255 colour index per interior cell.
function frameIndices() {
  const idx = new Uint8Array(N * N);
  for (let j = 0; j < N; j++) for (let i = 0; i < N; i++) {
    const c = IX(i+1, j+1);
    if (solid[c]) { idx[i + j*N] = 0; continue; }              // dome -> palette slot 0
    const speed = Math.hypot(u[c], v[c]);
    let val = 0.15 * Math.tanh(speed * 5) + 0.90 * Math.tanh(dens[c] * 0.5); // faint flow + water
    if (val > 1) val = 1;
    idx[i + j*N] = 2 + Math.round(val * 253);                   // 0=dome, 1=surface line
  }
  return idx;
}

// ------------------- Marching squares: liquid surface -----------------
// Trace the iso-contour dens == SURFACE_T across the grid. Each cell is one
// "square"; we classify its 4 corners against the threshold and emit the
// line segments where the surface crosses, interpolating the crossing point
// along each edge. The result is the outline of the liquid.
const SURFACE_T = 0.22;
// For each of the 16 corner cases, which edges the contour connects.
// Edges: 0=top, 1=right, 2=bottom, 3=left.
const MS_CASES = [
  [], [[3,0]], [[0,1]], [[3,1]], [[1,2]], [[3,0],[1,2]], [[0,2]], [[3,2]],
  [[2,3]], [[2,0]], [[0,1],[2,3]], [[2,1]], [[1,3]], [[1,0]], [[0,3]], [],
];
function drawLine(buf, x0, y0, x1, y1, ci) {
  x0 = Math.round(x0); y0 = Math.round(y0); x1 = Math.round(x1); y1 = Math.round(y1);
  let dx = Math.abs(x1 - x0), dy = -Math.abs(y1 - y0);
  let sx = x0 < x1 ? 1 : -1, sy = y0 < y1 ? 1 : -1, err = dx + dy;
  for (;;) {
    if (x0 >= 0 && x0 < W && y0 >= 0 && y0 < H) {
      buf[y0 * W + x0] = ci;                                    // 2px line for visibility
      if (x0 + 1 < W) buf[y0 * W + x0 + 1] = ci;
    }
    if (x0 === x1 && y0 === y1) break;
    const e2 = 2 * err;
    if (e2 >= dy) { err += dy; x0 += sx; }
    if (e2 <= dx) { err += dx; y0 += sy; }
  }
}
// Overlay the liquid surface onto an already-rendered WxH frame (index `ci`).
function overlaySurface(buf, ci) {
  const cx = (oi) => oi * CELL + CELL / 2;     // grid-cell centre -> pixel x
  const cy = (oj) => oj * CELL + CELL / 2;
  const T = SURFACE_T;
  for (let oj = 0; oj < N - 1; oj++) for (let oi = 0; oi < N - 1; oi++) {
    const v0 = dens[IX(oi + 1, oj + 1)];       // top-left
    const v1 = dens[IX(oi + 2, oj + 1)];       // top-right
    const v2 = dens[IX(oi + 2, oj + 2)];       // bottom-right
    const v3 = dens[IX(oi + 1, oj + 2)];       // bottom-left
    const c = (v0 > T ? 1 : 0) | (v1 > T ? 2 : 0) | (v2 > T ? 4 : 0) | (v3 > T ? 8 : 0);
    const segs = MS_CASES[c];
    if (!segs.length) continue;
    // Interpolated crossing point on each edge.
    const lerp = (a, b) => (T - a) / (b - a);
    const pts = [
      [cx(oi) + (cx(oi + 1) - cx(oi)) * lerp(v0, v1), cy(oj)],                 // 0 top
      [cx(oi + 1), cy(oj) + (cy(oj + 1) - cy(oj)) * lerp(v1, v2)],             // 1 right
      [cx(oi) + (cx(oi + 1) - cx(oi)) * lerp(v3, v2), cy(oj + 1)],             // 2 bottom
      [cx(oi), cy(oj) + (cy(oj + 1) - cy(oj)) * lerp(v0, v3)],                 // 3 left
    ];
    for (const [e0, e1] of segs)
      drawLine(buf, pts[e0][0], pts[e0][1], pts[e1][0], pts[e1][1], ci);
  }
}

// ----------------------------- GIF89a --------------------------------
// LSB-first bit writer + variable-width LZW, packed into 255-byte sub-blocks.
function lzwEncode(indices, minCodeSize, out) {
  const clear = 1 << minCodeSize, eoi = clear + 1;
  let codeSize = minCodeSize + 1, next = eoi + 1;
  let dict = new Map();
  const resetDict = () => { dict = new Map(); for (let i = 0; i < clear; i++) dict.set("" + i, i); next = eoi + 1; codeSize = minCodeSize + 1; };

  let bitBuf = 0, bitCnt = 0;
  const chunk = [];
  const flushBlocks = (final) => {
    while (chunk.length >= 255 || (final && chunk.length)) {
      const n = Math.min(255, chunk.length);
      out.push(n); for (let i = 0; i < n; i++) out.push(chunk[i]);
      chunk.splice(0, n);
    }
  };
  const emit = (code) => {
    bitBuf |= code << bitCnt; bitCnt += codeSize;
    while (bitCnt >= 8) { chunk.push(bitBuf & 0xff); bitBuf >>= 8; bitCnt -= 8; }
    if (chunk.length >= 255) flushBlocks(false);
  };

  resetDict();
  emit(clear);
  let prev = "" + indices[0];
  for (let p = 1; p < indices.length; p++) {
    const k = indices[p];
    const combo = prev + "," + k;
    if (dict.has(combo)) { prev = combo; }
    else {
      emit(dict.get(prev));
      dict.set(combo, next++);
      if (next > (1 << codeSize) && codeSize < 12) codeSize++;
      if (next >= 4096) { emit(clear); resetDict(); }
      prev = "" + k;
    }
  }
  emit(dict.get(prev));
  emit(eoi);
  if (bitCnt > 0) chunk.push(bitBuf & 0xff);
  flushBlocks(true);
  out.push(0); // block terminator
}

function writeGif(path, frames, w, h, palette, delayCs) {
  const out = [];
  const push = (...b) => b.forEach(x => out.push(x & 0xff));
  const u16 = (n) => push(n & 0xff, (n >> 8) & 0xff);

  // Header + Logical Screen Descriptor (256-colour global table)
  for (const ch of "GIF89a") push(ch.charCodeAt(0));
  u16(w); u16(h); push(0xf7, 0, 0);
  for (let i = 0; i < 768; i++) out.push(palette[i]);

  // Loop forever (NETSCAPE2.0)
  push(0x21, 0xff, 0x0b);
  for (const ch of "NETSCAPE2.0") push(ch.charCodeAt(0));
  push(0x03, 0x01, 0x00, 0x00, 0x00);

  for (const idx of frames) {
    push(0x21, 0xf9, 0x04, 0x00); u16(delayCs); push(0x00, 0x00); // GCE
    push(0x2c); u16(0); u16(0); u16(w); u16(h); push(0x00);        // image descriptor
    push(0x08);                                                     // LZW min code size
    lzwEncode(idx, 8, out);
  }
  push(0x3b); // trailer
  fs.writeFileSync(path, Buffer.from(out));
}

// ------------------------------- Main --------------------------------
const FRAMES = parseInt(process.argv[2] || "110", 10);
const CELL = parseInt(process.argv[3] || "5", 10);       // px per grid cell
const W = N * CELL, H = N * CELL;
const palette = buildPalette();

buildDome();
palette[0] = 122; palette[1] = 126; palette[2] = 140;   // slot 0: dome = stone gray
palette[3] = 255; palette[4] = 60;  palette[5] = 60;     // slot 1: surface line = red
const frames = [];
let peakSpeed = 0, peakMass = 0;
for (let f = 0; f < FRAMES; f++) {
  updateDrops(f);     // move droplets, slide off dome, collect in puddle
  buildDropField();   // stamp them into the density field for rendering

  // upscale interior grid -> WxH index buffer (block scaling)
  const small = frameIndices();
  const big = new Uint8Array(W * H);
  for (let j = 0; j < N; j++) for (let i = 0; i < N; i++) {
    const val = small[i + j*N];
    for (let dy = 0; dy < CELL; dy++) {
      const row = (j*CELL + dy) * W + i*CELL;
      for (let dx = 0; dx < CELL; dx++) big[row + dx] = val;
    }
  }
  overlaySurface(big, 1); // marching-squares liquid surface, palette slot 1
  frames.push(big);

  // running diagnostics: drops in flight + total water collected on the floor
  let puddleVol = 0; for (let i = 1; i <= N; i++) puddleVol += puddle[i];
  peakSpeed = Math.max(peakSpeed, drops.length); peakMass = Math.max(peakMass, puddleVol);
  if (f % 15 === 0) console.log(`frame ${f}: dropsInFlight=${drops.length} puddleVol=${puddleVol.toFixed(1)}`);
}

writeGif("cfd.gif", frames, W, H, palette, 6); // 6 = 60ms/frame (~16fps)
const kb = (fs.statSync("cfd.gif").size / 1024).toFixed(0);
console.log(`\nWrote cfd.gif  ${W}x${H}  ${FRAMES} frames  ${kb} KB`);
console.log(`max drops in flight=${peakSpeed}  total water pooled=${peakMass.toFixed(1)}`);
