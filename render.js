"use strict";
/*
 * Headless CFD renderer -> animated GIF.
 *
 * Runs the same stable-fluids Navier-Stokes solver as index.html, but with a
 * scripted scene (inflow jets + a central vortex) so the flow evolves on its
 * own. Each frame is mapped through a 256-colour gradient palette straight to
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
  for (let i = 0; i < SIZE; i++) dens[i] *= 0.985;
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
// A steady jet from the left, an angled jet from the bottom, and an initial
// central vortex. Dye is injected at the jet mouths so structures are visible.
function seedVortex() {
  const cx = N/2, cy = N/2;
  for (let j = 1; j <= N; j++) for (let i = 1; i <= N; i++) {
    const dx = i-cx, dy = j-cy, r = Math.hypot(dx, dy)+0.001;
    if (r < N*0.4) { const s = (1 - r/(N*0.4))*1.6; u[IX(i,j)] += -dy/r*s; v[IX(i,j)] += dx/r*s; }
  }
}
function forces(frame) {
  // left jet, centred vertically, with a slow vertical wobble
  const jy = Math.round(N*0.5 + Math.sin(frame*0.06)*N*0.18);
  for (let oj = -3; oj <= 3; oj++) {
    const j = jy + oj; if (j < 1 || j > N) continue;
    u[IX(2, j)] += 9; dens[IX(2, j)] += 16;
  }
  // bottom-left diagonal puffs, pulsing on/off
  if (Math.floor(frame/14) % 2 === 0) {
    const bx = Math.round(N*0.25);
    for (let oi = -2; oi <= 2; oi++) {
      const i = bx + oi; if (i < 1 || i > N) continue;
      v[IX(i, 2)] += 7; u[IX(i, 2)] += 2.5; dens[IX(i, 2)] += 13;
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
    const speed = Math.hypot(u[c], v[c]);
    let val = 0.45 * Math.tanh(speed * 6) + 0.75 * Math.tanh(dens[c] * 0.22); // flow glow + dye
    if (val > 1) val = 1;
    idx[i + j*N] = Math.round(val * 255);
  }
  return idx;
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

seedVortex();
const frames = [];
let peakSpeed = 0, peakMass = 0;
for (let f = 0; f < FRAMES; f++) {
  uPrev.fill(0); vPrev.fill(0); densPrev.fill(0);
  forces(f);
  velStep(DT); densStep(DT);

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
  frames.push(big);

  // running diagnostics (the "sums")
  let e = 0, m = 0, sp = 0;
  for (let c = 0; c < SIZE; c++) { e += 0.5*(u[c]*u[c]+v[c]*v[c]); m += dens[c]; const s = Math.hypot(u[c], v[c]); if (s > sp) sp = s; }
  peakSpeed = Math.max(peakSpeed, sp); peakMass = Math.max(peakMass, m);
  if (f % 20 === 0) console.log(`frame ${f}: ΣKE=${e.toFixed(0)} Σmass=${m.toFixed(0)} maxSpeed=${sp.toFixed(2)}`);
}

writeGif("cfd.gif", frames, W, H, palette, 6); // 6 = 60ms/frame (~16fps)
const kb = (fs.statSync("cfd.gif").size / 1024).toFixed(0);
console.log(`\nWrote cfd.gif  ${W}x${H}  ${FRAMES} frames  ${kb} KB`);
console.log(`peak ΣKE-driven maxSpeed=${peakSpeed.toFixed(2)}  peak Σmass=${peakMass.toFixed(0)}`);
