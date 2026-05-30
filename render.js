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
// Pressure projection with interior-obstacle (dome) boundary conditions.
// At a fluid/solid face the velocity is reflected (no flow through the dome) and
// pressure uses a Neumann condition, so the solve deflects flow AROUND the dome
// instead of letting it vanish into the solid. With no solids this reduces to
// the plain Stam projection.
function project(uu, vv, p, divg) {
  const h = 1 / N;
  const S = (c) => solid[c];
  for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++) {
    const c = IX(i, j);
    if (S(c)) { divg[c] = 0; p[c] = 0; continue; }
    const uR = S(IX(i+1,j)) ? -uu[c] : uu[IX(i+1,j)];
    const uL = S(IX(i-1,j)) ? -uu[c] : uu[IX(i-1,j)];
    const vT = S(IX(i,j+1)) ? -vv[c] : vv[IX(i,j+1)];
    const vB = S(IX(i,j-1)) ? -vv[c] : vv[IX(i,j-1)];
    divg[c] = -0.5 * h * ((uR - uL) + (vT - vB));
    p[c] = 0;
  }
  setBnd(0, divg); setBnd(0, p);
  for (let k = 0; k < ITER; k++) {
    for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++) {
      const c = IX(i, j);
      if (S(c)) continue;
      let sum = 0, n = 0;
      const L = IX(i-1,j), R = IX(i+1,j), B = IX(i,j-1), T = IX(i,j+1);
      if (!S(L)) { sum += p[L]; n++; }
      if (!S(R)) { sum += p[R]; n++; }
      if (!S(B)) { sum += p[B]; n++; }
      if (!S(T)) { sum += p[T]; n++; }
      if (n > 0) p[c] = (divg[c] + sum) / n;
    }
    setBnd(0, p);
  }
  for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++) {
    const c = IX(i, j);
    if (S(c)) continue;
    const pR = S(IX(i+1,j)) ? p[c] : p[IX(i+1,j)];
    const pL = S(IX(i-1,j)) ? p[c] : p[IX(i-1,j)];
    const pT = S(IX(i,j+1)) ? p[c] : p[IX(i,j+1)];
    const pB = S(IX(i,j-1)) ? p[c] : p[IX(i,j-1)];
    uu[c] -= 0.5 * (pR - pL) / h;
    vv[c] -= 0.5 * (pT - pB) / h;
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
// Hybrid (PIC/FLIP-style) liquid. A faucet drips discrete Lagrangian droplets
// that fall ballistically (clean, controlled - a grid solver would smear a
// small drop away). On impact with the dome (or floor) a droplet's mass and
// momentum are DEPOSITED into the incompressible pressure-grid solver, which
// then makes it splash, sheet down the dome, and pool like a real fluid.
const DROP_G = 0.13;          // gravity on an in-flight droplet (cells/frame^2)
const DROP_R = 2.7;           // droplet render radius (cells)
const DRIP_PERIOD = 28;       // frames between drips: one drip = one drop
const DRIP_X = Math.round(N / 2) - 7, DRIP_Y = 5; // faucet off-apex so it sheets down a flank
const DOME_CX = N / 2, DOME_CY = N, DOME_R = N * 0.26;
const GRID_G = 4.5;           // buoyancy gravity on the deposited fluid (runs it down)
const DEP_DENS = 28.0;        // density injected when a drop lands
const GRID_VIS = 1.4;         // how strongly grid fluid contributes to the render field

const solid = new Uint8Array(SIZE);
function buildDome() {
  for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++) {
    const dx = i - DOME_CX, dy = j - DOME_CY;
    if (dx * dx + dy * dy <= DOME_R * DOME_R) solid[IX(i, j)] = 1;
  }
}
// The dome is a no-flow obstacle: zero velocity and dye inside it.
function applySolid() {
  for (let c = 0; c < SIZE; c++) if (solid[c]) { u[c] = 0; v[c] = 0; dens[c] = 0; }
}

const drops = [];             // in-flight droplets {x, y, vx, vy}

// Hand a droplet off to the grid solver: splat its mass + momentum at the impact
// point (tx,ty), which the caller places just OUTSIDE the dome (non-solid cells).
function depositToGrid(tx, ty, vx, vy) {
  const ci = Math.max(2, Math.min(N - 1, Math.round(tx)));
  const cj = Math.max(2, Math.min(N - 1, Math.round(ty)));
  // Downhill tangent of the dome at the impact point, so the splash streaks down.
  const ndx = tx - DOME_CX, ndy = ty - DOME_CY, nd = Math.hypot(ndx, ndy) || 1e-6;
  const nx = ndx / nd, ny = ndy / nd;
  let tgx = -ny, tgy = nx; if (tgy < 0) { tgx = -tgx; tgy = -tgy; } // choose downhill (+y)
  const speed = Math.hypot(vx, vy);
  for (let oj = -1; oj <= 1; oj++) for (let oi = -1; oi <= 1; oi++) {
    const i = ci + oi, j = cj + oj;
    if (i < 1 || i > N || j < 1 || j > N || solid[IX(i, j)]) continue;
    const w = (oi === 0 && oj === 0) ? 1 : 0.6;
    dens[IX(i, j)] += DEP_DENS * w;
    u[IX(i, j)] += vx * 0.3 + tgx * speed * 0.8;  // momentum + downhill splash
    v[IX(i, j)] += vy * 0.3 + tgy * speed * 0.8;
  }
}

function updateDrops(frame) {
  if (frame % DRIP_PERIOD === 0) drops.push({ x: DRIP_X, y: DRIP_Y, vx: 0, vy: 0.5 });

  const surf = DOME_R + DROP_R * 0.6;
  for (const d of drops) {
    if (d.dead) continue;
    d.vy += DROP_G;                    // ballistic fall
    d.x += d.vx; d.y += d.vy;
    const dx = d.x - DOME_CX, dy = d.y - DOME_CY, dist = Math.hypot(dx, dy) || 1e-6;
    if (dist < surf && d.y < DOME_CY) {
      // Project onto the dome surface (just outside it) so the splash lands on
      // non-solid cells, then hand momentum to the grid.
      const nx = dx / dist, ny = dy / dist, r = DOME_R + 1.5;
      depositToGrid(DOME_CX + nx * r, DOME_CY + ny * r, d.vx, d.vy);
      d.dead = true;
    } else if (d.y >= N - 1) {         // missed the dome, hit the floor
      depositToGrid(d.x, N - 1, d.vx, d.vy);
      d.dead = true;
    }
  }
  for (let n = drops.length - 1; n >= 0; n--) if (drops[n].dead) drops.splice(n, 1);
}

// One step of the incompressible solver acting on the deposited fluid.
function gridStep() {
  uPrev.fill(0); vPrev.fill(0); densPrev.fill(0);
  for (let c = 0; c < SIZE; c++) if (!solid[c]) v[c] += DT * GRID_G * dens[c]; // buoyancy
  velStep(DT); applySolid();
  densStep(DT); applySolid();
  for (let i = 1; i <= N; i++) dens[IX(i, N)] *= 0.96;  // mild floor drain (let it pool)
}

// Bilinear sample of the grid dye field at fractional grid coords.
function sampleDens(gx, gy) {
  if (gx < 1) gx = 1; else if (gx > N) gx = N;
  if (gy < 1) gy = 1; else if (gy > N) gy = N;
  const i0 = gx | 0, j0 = gy | 0, i1 = Math.min(N, i0 + 1), j1 = Math.min(N, j0 + 1);
  const fx = gx - i0, fy = gy - j0;
  const a = dens[IX(i0, j0)], b = dens[IX(i1, j0)], c = dens[IX(i0, j1)], e = dens[IX(i1, j1)];
  return (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + e * fx) * fy;
}

// ------------------------- Liquid renderer ---------------------------
// Full-resolution metaball rendering. Each droplet is a smooth radial field;
// summing them makes nearby drops merge gooily (the classic "liquid" look).
// The iso-surface F = T_SURF is the liquid boundary; shading by depth gives a
// translucent bright rim over a deep-blue body, plus a specular highlight.
const T_SURF = 0.6;           // iso-surface threshold
const DEPTH = 1.3;            // field range mapped across the water ramp
const WATER0 = 16;            // first water palette index (rim) .. 255 (deep)

function renderFrame(big, t) {
  const Rpx = DROP_R * CELL;
  const R2 = Rpx * Rpx;
  const domeCx = (DOME_CX - 0.5) * CELL, domeCy = (DOME_CY - 0.5) * CELL, domeR = DOME_R * CELL;
  // Precompute each drop's pixel centre + teardrop stretch from its velocity.
  const ds = drops.map(d => {
    const sp = Math.hypot(d.vx, d.vy);
    const k = 1 + Math.min(1.5, sp * 0.55);          // elongation along motion
    const nx = sp > 1e-4 ? d.vx / sp : 0, ny = sp > 1e-4 ? d.vy / sp : 1;
    return { px: (d.x - 0.5) * CELL, py: (d.y - 0.5) * CELL, k, nx, ny };
  });

  for (let Y = 0; Y < H; Y++) {
    for (let X = 0; X < W; X++) {
      let F = 0;
      for (const d of ds) {
        const ex = X - d.px, ey = Y - d.py;
        const along = ex * d.nx + ey * d.ny, perp = -ex * d.ny + ey * d.nx;
        const r2 = (along * along) / (d.k * d.k) + perp * perp; // anisotropic distance^2
        if (r2 < R2) { const s = 1 - r2 / R2; F += s * s; }
      }
      // Grid fluid (splashed / sheeting / pooled liquid) sampled into the field.
      F += sampleDens(X / CELL + 0.5, Y / CELL + 0.5) * GRID_VIS;

      let idx;
      if (F >= T_SURF) {
        const s = Math.min(1, (F - T_SURF) / DEPTH);
        idx = WATER0 + Math.round(s * (255 - WATER0));
      } else {
        const ddx = X - domeCx, ddy = Y - domeCy;
        idx = (ddy <= 0 && ddx * ddx + ddy * ddy <= domeR * domeR) ? 0 : 1; // dome : sky
      }
      big[Y * W + X] = idx;
    }
  }

  // Specular highlight: a small bright dot on the upper-left of each drop.
  for (const d of ds) {
    const hx = Math.round(d.px - Rpx * 0.32), hy = Math.round(d.py - Rpx * 0.36);
    const hr = Math.max(1, Math.round(Rpx * 0.22));
    for (let oy = -hr; oy <= hr; oy++) for (let ox = -hr; ox <= hr; ox++) {
      if (ox * ox + oy * oy > hr * hr) continue;
      const X = hx + ox, Y = hy + oy;
      if (X < 0 || X >= W || Y < 0 || Y >= H) continue;
      if (big[Y * W + X] >= WATER0) big[Y * W + X] = 2; // only over water
    }
  }
}

// ----------------------- Colour palette (256) ------------------------
function buildPalette() {
  // water ramp: bright translucent rim -> blue body -> deep blue core
  const stops = [
    [225, 248, 255], [150, 218, 250], [80, 170, 238],
    [45, 115, 208], [28, 75, 170], [16, 48, 122],
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
palette[0] = 120; palette[1] = 124; palette[2] = 140;   // slot 0: dome = stone gray
palette[3] = 10;  palette[4] = 14;  palette[5] = 30;     // slot 1: background sky (dark)
palette[6] = 255; palette[7] = 255; palette[8] = 255;    // slot 2: specular highlight
const frames = [];
let peakSpeed = 0, peakMass = 0;
for (let f = 0; f < FRAMES; f++) {
  updateDrops(f);                  // ballistic drops; deposit to grid on impact
  gridStep();                      // incompressible solver flows the splashed fluid
  const big = new Uint8Array(W * H);
  renderFrame(big, f);             // metaball drops + grid fluid, water-shaded
  frames.push(big);

  // running diagnostics: drops in flight + total fluid on the grid
  let gridMass = 0; for (let c = 0; c < SIZE; c++) gridMass += dens[c];
  peakSpeed = Math.max(peakSpeed, drops.length); peakMass = Math.max(peakMass, gridMass);
  if (f % 15 === 0) console.log(`frame ${f}: dropsInFlight=${drops.length} gridFluid=${gridMass.toFixed(0)}`);
}

writeGif("cfd.gif", frames, W, H, palette, 6); // 6 = 60ms/frame (~16fps)
const kb = (fs.statSync("cfd.gif").size / 1024).toFixed(0);
console.log(`\nWrote cfd.gif  ${W}x${H}  ${FRAMES} frames  ${kb} KB`);
console.log(`max drops in flight=${peakSpeed}  total water pooled=${peakMass.toFixed(1)}`);
