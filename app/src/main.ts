// Petri Polis — entry point.
// Pipeline: Rust Physarum sim (two species) → (zero-copy) → two R32F trail textures + one food texture
//           → tone-map pass (per-species EMA auto-exposure, cyan + magenta palettes, additive composite
//             with food substrate underneath) → FBO
//           → bloom bright-pass → ping-pong Gaussian blur → additive composite
//           → present to canvas.
//
// Overlay instruments (drawn each frame onto separate canvases / DOM elements):
//   - Sparkline canvas (bottom-left): population, trail mass, food total, food coverage.
//   - Inspector readout (bottom-right): per-cell trail/food values + nearest agent.
//   - Metrics Tweakpane folder: Export CSV / Export JSON / Copy link.

import { Pane } from "tweakpane";
import { attachInspector } from "./inspector";
import { drawSparklines, exportCSV, exportJSON, type MetricSample, MetricsBuffer } from "./metrics";
import {
  type BloomFBO,
  compileProgram,
  createBloomFBO,
  createFieldTexture,
  createGL,
  uploadField,
} from "./render/gl";
import { applySharedState, decodeHash, encodeState, type SimParamObjects } from "./urlstate";
import init, { Sim } from "./wasm/petri_wasm.js";
import wasmUrl from "./wasm/petri_wasm_bg.wasm?url";

// ---------------------------------------------------------------------------
// Fullscreen triangle from gl_VertexID — no vertex buffers needed.
// ---------------------------------------------------------------------------
const VERT = `#version 300 es
out vec2 v_uv;
void main() {
  vec2 uv = vec2((gl_VertexID == 1) ? 2.0 : 0.0, (gl_VertexID == 2) ? 2.0 : 0.0);
  v_uv = uv;
  gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}`;

// ---------------------------------------------------------------------------
// Pass 1 — tone-map: two species with per-species EMA-stabilised gains.
//   Species 0: cyan ramp  (near-black → deep teal → cyan → white-hot)
//   Species 1: magenta ramp (near-black → deep purple → magenta → white-hot)
// The two coloured contributions are additively combined so overlap areas blend
// toward white — coexistence reads immediately as energetic white seams.
// A dim olive food substrate sits underneath, masked out wherever the combined
// trail is bright. Result written to RGBA16F FBO (the "base" for bloom).
// ---------------------------------------------------------------------------
const FRAG_TONEMAP = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_field0;   // species 0 trail
uniform sampler2D u_field1;   // species 1 trail
uniform sampler2D u_food;
uniform float u_gain0;
uniform float u_gain1;
uniform float u_food_gain;
out vec4 o;

// Bioluminescent cyan palette: near-black → deep teal → cyan → white-hot.
vec3 cyanPalette(float t) {
  t = clamp(t, 0.0, 1.0);
  vec3 c0 = vec3(0.01, 0.02, 0.05);   // near-black blue
  vec3 c1 = vec3(0.00, 0.30, 0.42);   // deep teal
  vec3 c2 = vec3(0.05, 0.80, 0.92);   // cyan
  vec3 c3 = vec3(0.90, 1.00, 1.00);   // white-hot
  if (t < 0.35) return mix(c0, c1, t / 0.35);
  if (t < 0.70) return mix(c1, c2, (t - 0.35) / 0.35);
  return mix(c2, c3, (t - 0.70) / 0.30);
}

// Bioluminescent magenta palette: near-black → deep purple → magenta → white-hot.
// Lifted in the mid-range so its luminance is comparable to the cyan ramp
// (pure magenta sits darker than pure cyan in perceptual space).
vec3 magentaPalette(float t) {
  t = clamp(t, 0.0, 1.0);
  vec3 m0 = vec3(0.02, 0.01, 0.04);   // near-black violet
  vec3 m1 = vec3(0.35, 0.00, 0.45);   // deep purple — lifted for luminance parity
  vec3 m2 = vec3(0.92, 0.05, 0.80);   // magenta-pink
  vec3 m3 = vec3(1.00, 0.90, 1.00);   // white-hot
  if (t < 0.35) return mix(m0, m1, t / 0.35);
  if (t < 0.70) return mix(m1, m2, (t - 0.35) / 0.35);
  return mix(m2, m3, (t - 0.70) / 0.30);
}

// Food substrate palette: near-black → deep olive → muted amber-green.
// Kept dim so only the trail bloom is the spectacle.
vec3 foodPalette(float t) {
  t = clamp(t, 0.0, 1.0);
  vec3 f0 = vec3(0.00, 0.00, 0.00);   // absent food: true black
  vec3 f1 = vec3(0.04, 0.07, 0.01);   // sparse: very dark moss
  vec3 f2 = vec3(0.10, 0.16, 0.03);   // moderate: dim olive-green
  vec3 f3 = vec3(0.18, 0.24, 0.06);   // rich: muted amber-green peak
  if (t < 0.35) return mix(f0, f1, t / 0.35);
  if (t < 0.70) return mix(f1, f2, (t - 0.35) / 0.35);
  return mix(f2, f3, (t - 0.70) / 0.30);
}

void main() {
  // --- Species 0 (cyan) ---
  float raw0 = texture(u_field0, v_uv).r;
  float t0 = pow(clamp(raw0 * u_gain0, 0.0, 1.0), 0.45);
  vec3 cyan = cyanPalette(t0);

  // --- Species 1 (magenta) ---
  float raw1 = texture(u_field1, v_uv).r;
  float t1 = pow(clamp(raw1 * u_gain1, 0.0, 1.0), 0.45);
  vec3 magenta = magentaPalette(t1);

  // Additive composite: where both species are present the result trends toward
  // white — the classic additive colour mixing signal for coexistence.
  vec3 trail = cyan + magenta;

  // --- Food substrate ---
  float rawFood = texture(u_food, v_uv).r;
  float f = clamp(rawFood * u_food_gain, 0.0, 1.0);
  float ft = pow(f, 0.65) * 0.45;
  vec3 food = foodPalette(ft);

  // Mask the food substrate where the combined trail is bright, so they don't
  // muddy each other. Use perceptual luma of the combined trail for the mask.
  float trailLuma = dot(trail, vec3(0.2126, 0.7152, 0.0722));
  float foodMask = 1.0 - smoothstep(0.02, 0.18, trailLuma);
  vec3 color = food * foodMask + trail;

  o = vec4(color, 1.0);
}`;

// ---------------------------------------------------------------------------
// Pass 2 — bright-pass: keep only pixels above a luminance threshold.
// Written at half resolution to a bloom FBO.
// ---------------------------------------------------------------------------
const FRAG_BRIGHT = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_src;
uniform float u_threshold;
out vec4 o;

float luma(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

void main() {
  vec3 col = texture(u_src, v_uv).rgb;
  float l = luma(col);
  // Soft-knee above threshold so no hard cutoff.
  float knee = 0.15;
  float w = smoothstep(u_threshold - knee, u_threshold + knee, l);
  o = vec4(col * w, 1.0);
}`;

// ---------------------------------------------------------------------------
// Pass 3 — separable Gaussian blur (horizontal / vertical shared source).
// u_dir = (1/w, 0) for horizontal, (0, 1/h) for vertical.
// 9-tap kernel (σ ≈ 2.0) for a soft wide glow.
// ---------------------------------------------------------------------------
const FRAG_BLUR = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_src;
uniform vec2 u_dir;
out vec4 o;

// 9-tap Gaussian weights (normalised) for σ≈2
const float W[5] = float[](0.227027, 0.194595, 0.121622, 0.054054, 0.016216);

void main() {
  vec4 acc = texture(u_src, v_uv) * W[0];
  for (int i = 1; i < 5; i++) {
    vec2 off = u_dir * float(i);
    acc += texture(u_src, v_uv + off) * W[i];
    acc += texture(u_src, v_uv - off) * W[i];
  }
  o = acc;
}`;

// ---------------------------------------------------------------------------
// Pass 4 — composite: base + additive bloom, present to screen.
// ---------------------------------------------------------------------------
const FRAG_COMPOSITE = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_base;
uniform sampler2D u_bloom;
uniform float u_bloom_strength;
out vec4 o;

void main() {
  vec3 base  = texture(u_base,  v_uv).rgb;
  vec3 glow  = texture(u_bloom, v_uv).rgb;
  o = vec4(base + glow * u_bloom_strength, 1.0);
}`;

// ---------------------------------------------------------------------------
// Sim dimensions
// ---------------------------------------------------------------------------
const WIDTH = 256;
const HEIGHT = 256;

// ---------------------------------------------------------------------------
// Bloom constants
// ---------------------------------------------------------------------------
const BLOOM_SCALE = 0.5; // render bloom FBOs at half resolution
const BLOOM_THRESHOLD = 0.4; // luminance threshold for bright-pass
const BLOOM_STRENGTH = 1.8; // additive blend weight
const BLUR_PASSES = 3; // number of blur iteration pairs

// ---------------------------------------------------------------------------
// EMA auto-exposure
// ---------------------------------------------------------------------------
const EMA_ALPHA = 0.05; // lerp speed — slow enough to avoid flicker
const GAIN_TARGET = 0.7; // map the EMA reference to ~70% brightness

// ---------------------------------------------------------------------------
// Zero-copy field views.
// IMPORTANT: re-fetch ALL views after every spawn()/reset() because WASM
// linear memory may relocate. Never re-create inside the per-frame loop.
// ---------------------------------------------------------------------------
function makeTrailView(
  wasm: { memory: WebAssembly.Memory },
  sim: Sim,
  species: number,
): Float32Array {
  return new Float32Array(wasm.memory.buffer, sim.field_ptr(species), sim.field_len());
}

function makeFoodView(wasm: { memory: WebAssembly.Memory }, sim: Sim): Float32Array {
  return new Float32Array(wasm.memory.buffer, sim.food_ptr(), sim.food_len());
}

// Re-fetch all zero-copy views and return them.
function refreshViews(
  wasm: { memory: WebAssembly.Memory },
  sim: Sim,
): { trails: Float32Array[]; food: Float32Array } {
  const count = sim.species_count();
  const trails: Float32Array[] = [];
  for (let s = 0; s < count; s++) {
    trails.push(makeTrailView(wasm, sim, s));
  }
  return { trails, food: makeFoodView(wasm, sim) };
}

// ---------------------------------------------------------------------------
// Sim parameter state — drives Tweakpane bindings and live set_params calls.
// ---------------------------------------------------------------------------
interface SimParams {
  sensor_angle: number;
  sensor_distance: number;
  rotation_angle: number;
  step_size: number;
  deposit: number;
  decay: number;
  diffuse_weight: number;
}

function readSimParams(sim: Sim, species: number): SimParams {
  return {
    sensor_angle: sim.sensor_angle(species),
    sensor_distance: sim.sensor_distance(species),
    rotation_angle: sim.rotation_angle(species),
    step_size: sim.step_size(species),
    deposit: sim.deposit_amount(species),
    decay: sim.decay(species),
    diffuse_weight: sim.diffuse_weight(species),
  };
}

function applySimParams(sim: Sim, species: number, p: SimParams): void {
  sim.set_params(
    species,
    p.sensor_angle,
    p.sensor_distance,
    p.rotation_angle,
    p.step_size,
    p.deposit,
    p.decay,
  );
  sim.set_diffuse_weight(species, p.diffuse_weight);
}

// ---------------------------------------------------------------------------
// Ecology parameter state — drives Tweakpane bindings and live set_ecology calls.
// ---------------------------------------------------------------------------
interface EcologyParams {
  metabolism: number;
  eat_rate: number;
  repro_threshold: number;
  food_regrow: number;
  death_return: number;
}

function readEcologyParams(sim: Sim, species: number): EcologyParams {
  return {
    metabolism: sim.metabolism(species),
    eat_rate: sim.eat_rate(species),
    repro_threshold: sim.repro_threshold(species),
    food_regrow: sim.food_regrow(species),
    death_return: sim.death_return(species),
  };
}

function applyEcologyParams(sim: Sim, species: number, e: EcologyParams): void {
  sim.set_ecology(
    species,
    e.metabolism,
    e.eat_rate,
    e.repro_threshold,
    e.food_regrow,
    e.death_return,
  );
}

// ---------------------------------------------------------------------------
// Spawn state — drives click-to-spawn and Tweakpane spawn controls.
// ---------------------------------------------------------------------------
interface SpawnState {
  count: number;
  pattern: number; // 0=point, 1=ring, 2=disk
  species: number; // 0=cyan, 1=magenta
}

// ---------------------------------------------------------------------------
// Transport state
// ---------------------------------------------------------------------------
interface Transport {
  paused: boolean;
  speed: number; // ticks per RAF frame: 1 | 2 | 5 | 10
  stepOnce: boolean;
}

// ---------------------------------------------------------------------------
// Add per-species parameter folder to a Tweakpane container.
// Collapsed by default so the panel isn't overwhelming at launch.
// ---------------------------------------------------------------------------
function addSpeciesFolder(
  pane: Pane,
  label: string,
  sim: Sim,
  species: number,
  params: SimParams,
  ecology: EcologyParams,
): void {
  const folder = pane.addFolder({ title: label, expanded: false });

  // Physarum parameters
  folder
    .addBinding(params, "sensor_angle", { label: "sensor angle", min: 0, max: 1.2, step: 0.01 })
    .on("change", () => applySimParams(sim, species, params));

  folder
    .addBinding(params, "sensor_distance", { label: "sensor dist", min: 1, max: 32, step: 0.5 })
    .on("change", () => applySimParams(sim, species, params));

  folder
    .addBinding(params, "rotation_angle", { label: "rotation angle", min: 0, max: 1.2, step: 0.01 })
    .on("change", () => applySimParams(sim, species, params));

  folder
    .addBinding(params, "step_size", { label: "step size", min: 0.2, max: 3.0, step: 0.05 })
    .on("change", () => applySimParams(sim, species, params));

  folder
    .addBinding(params, "deposit", { label: "deposit", min: 0.5, max: 20, step: 0.5 })
    .on("change", () => applySimParams(sim, species, params));

  folder
    .addBinding(params, "decay", { label: "decay", min: 0.8, max: 0.99, step: 0.005 })
    .on("change", () => applySimParams(sim, species, params));

  folder
    .addBinding(params, "diffuse_weight", { label: "blur weight", min: 0, max: 1, step: 0.01 })
    .on("change", () => applySimParams(sim, species, params));

  // Ecology parameters
  folder
    .addBinding(ecology, "metabolism", { label: "metabolism", min: 0.001, max: 0.02, step: 0.001 })
    .on("change", () => applyEcologyParams(sim, species, ecology));

  folder
    .addBinding(ecology, "eat_rate", { label: "eat rate", min: 0.02, max: 0.3, step: 0.01 })
    .on("change", () => applyEcologyParams(sim, species, ecology));

  folder
    .addBinding(ecology, "repro_threshold", {
      label: "repro threshold",
      min: 0.6,
      max: 2.5,
      step: 0.05,
    })
    .on("change", () => applyEcologyParams(sim, species, ecology));

  folder
    .addBinding(ecology, "food_regrow", {
      label: "food regrow",
      min: 0.001,
      max: 0.02,
      step: 0.0005,
    })
    .on("change", () => applyEcologyParams(sim, species, ecology));

  folder
    .addBinding(ecology, "death_return", { label: "death return", min: 0, max: 1, step: 0.05 })
    .on("change", () => applyEcologyParams(sim, species, ecology));
}

// ---------------------------------------------------------------------------
// Build the Tweakpane control panel
// ---------------------------------------------------------------------------
function buildPane(
  sim: Sim,
  allParams: SimParams[],
  allEcology: EcologyParams[],
  spawn: SpawnState,
  transport: Transport,
  onReset: () => void,
  seedRef: { value: number },
  metricsBuf: MetricsBuffer,
  paneRef: { pane: Pane | null },
): void {
  const pane = new Pane({ title: "Petri Polis" });
  paneRef.pane = pane;

  // --- Transport -----------------------------------------------------------
  const transportFolder = pane.addFolder({ title: "Transport", expanded: true });

  transportFolder.addButton({ title: "Play / Pause" }).on("click", () => {
    transport.paused = !transport.paused;
  });

  transportFolder.addButton({ title: "Step" }).on("click", () => {
    transport.stepOnce = true;
  });

  transportFolder.addBinding(transport, "speed", {
    label: "speed",
    view: "list",
    options: [
      { text: "1×", value: 1 },
      { text: "2×", value: 2 },
      { text: "5×", value: 5 },
      { text: "10×", value: 10 },
    ],
  });

  // --- Reset ---------------------------------------------------------------
  const resetFolder = pane.addFolder({ title: "Reset", expanded: false });

  resetFolder.addBinding(seedRef, "value", {
    label: "seed",
    min: 0,
    max: 0xffffffff,
    step: 1,
  });

  resetFolder.addButton({ title: "Reset" }).on("click", onReset);

  // --- Per-species parameter folders (collapsed by default) ----------------
  const speciesLabels = ["Species 0 · cyan", "Species 1 · magenta"];
  for (let s = 0; s < sim.species_count(); s++) {
    addSpeciesFolder(pane, speciesLabels[s], sim, s, allParams[s], allEcology[s]);
  }

  // --- Spawn ---------------------------------------------------------------
  const spawnFolder = pane.addFolder({ title: "Spawn (click canvas)", expanded: false });

  spawnFolder.addBinding(spawn, "count", {
    label: "count",
    min: 50,
    max: 5000,
    step: 50,
  });

  spawnFolder.addBinding(spawn, "pattern", {
    label: "pattern",
    view: "list",
    options: [
      { text: "point", value: 0 },
      { text: "ring", value: 1 },
      { text: "disk", value: 2 },
    ],
  });

  spawnFolder.addBinding(spawn, "species", {
    label: "species",
    view: "list",
    options: [
      { text: "cyan (0)", value: 0 },
      { text: "magenta (1)", value: 1 },
    ],
  });

  // --- Metrics -------------------------------------------------------------
  const metricsFolder = pane.addFolder({ title: "Metrics", expanded: false });

  metricsFolder.addButton({ title: "Export CSV" }).on("click", () => {
    exportCSV(metricsBuf, seedRef.value);
  });

  metricsFolder.addButton({ title: "Export JSON" }).on("click", () => {
    exportJSON(metricsBuf, seedRef.value);
  });

  metricsFolder.addButton({ title: "Copy link" }).on("click", () => {
    const hash = encodeState(sim, seedRef.value);
    location.hash = hash;
    navigator.clipboard.writeText(location.href).catch(() => {
      // clipboard may be unavailable (non-HTTPS); at least the hash is set
    });
  });
}

// ---------------------------------------------------------------------------
// Click-to-spawn canvas handler
// ---------------------------------------------------------------------------
function attachSpawnHandler(
  canvas: HTMLCanvasElement,
  sim: Sim,
  spawn: SpawnState,
  onSpawn: () => void,
): void {
  canvas.addEventListener("pointerdown", (e: PointerEvent) => {
    const rect = canvas.getBoundingClientRect();
    const u = (e.clientX - rect.left) / rect.width;
    const v = (e.clientY - rect.top) / rect.height;
    // The fullscreen triangle maps uv(0,0) to the bottom-left of clip space,
    // so field row 0 renders at the bottom of the screen. Flip V to correct.
    const sx = u * sim.width();
    const sy = (1 - v) * sim.height();
    sim.spawn(sx, sy, spawn.count, spawn.pattern, spawn.species);
    onSpawn();
  });
}

// ---------------------------------------------------------------------------
// FBO allocation helpers
// ---------------------------------------------------------------------------
interface FBOSet {
  base: BloomFBO;
  ping: BloomFBO;
  pong: BloomFBO;
}

function allocFBOs(gl: WebGL2RenderingContext, w: number, h: number): FBOSet {
  const bw = Math.max(1, Math.floor(w * BLOOM_SCALE));
  const bh = Math.max(1, Math.floor(h * BLOOM_SCALE));
  return {
    base: createBloomFBO(gl, w, h),
    ping: createBloomFBO(gl, bw, bh),
    pong: createBloomFBO(gl, bw, bh),
  };
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
async function main(): Promise<void> {
  // Decode URL hash before creating the sim so we use the right seed.
  const sharedState = decodeHash();
  const initialSeed = sharedState ? sharedState.seed : 1;

  const wasm = await init({ module_or_path: wasmUrl });
  const sim = new Sim(WIDTH, HEIGHT, initialSeed);

  // Zero-copy views — re-fetch after spawn/reset (defensive; WASM memory is
  // stable here since field buffers never reallocate, but we re-fetch anyway).
  let views = refreshViews(wasm, sim);

  const canvas = document.getElementById("c") as HTMLCanvasElement;
  const hud = document.getElementById("hud") as HTMLDivElement;
  const sparklinesCanvas = document.getElementById("sparklines") as HTMLCanvasElement;
  const inspectorEl = document.getElementById("inspector") as HTMLDivElement;
  const gl = createGL(canvas);

  // Sparkline 2D context — only 2D canvas API, never touched WebGL.
  const sparkCtx = sparklinesCanvas.getContext("2d")!;

  // Shared fullscreen-triangle VAO (WebGL2 needs one bound even with no attribs).
  const vao = gl.createVertexArray()!;

  // ---------------------------------------------------------------------------
  // Compile all programs
  // ---------------------------------------------------------------------------
  const progTonemap = compileProgram(gl, VERT, FRAG_TONEMAP);
  const progBright = compileProgram(gl, VERT, FRAG_BRIGHT);
  const progBlur = compileProgram(gl, VERT, FRAG_BLUR);
  const progComposite = compileProgram(gl, VERT, FRAG_COMPOSITE);

  // ---------------------------------------------------------------------------
  // Trail textures — one per species (R32F, updated each frame)
  // ---------------------------------------------------------------------------
  const speciesCount = sim.species_count();
  const trailTexs: WebGLTexture[] = [];
  for (let s = 0; s < speciesCount; s++) {
    trailTexs.push(createFieldTexture(gl, WIDTH, HEIGHT));
  }

  // ---------------------------------------------------------------------------
  // Food field texture (R32F, updated each frame)
  // ---------------------------------------------------------------------------
  const foodTex = createFieldTexture(gl, WIDTH, HEIGHT);

  // ---------------------------------------------------------------------------
  // Off-screen FBOs — reallocated on resize
  // ---------------------------------------------------------------------------
  let fbos: FBOSet = allocFBOs(gl, 1, 1);

  function resize(): void {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.floor(window.innerWidth * dpr);
    canvas.height = Math.floor(window.innerHeight * dpr);
    gl.viewport(0, 0, canvas.width, canvas.height);
    fbos = allocFBOs(gl, canvas.width, canvas.height);
  }
  window.addEventListener("resize", resize);
  resize();

  // ---------------------------------------------------------------------------
  // Uniform locations — tone-map
  // ---------------------------------------------------------------------------
  const ulTonemap_field0 = gl.getUniformLocation(progTonemap, "u_field0");
  const ulTonemap_field1 = gl.getUniformLocation(progTonemap, "u_field1");
  const ulTonemap_food = gl.getUniformLocation(progTonemap, "u_food");
  const ulTonemap_gain0 = gl.getUniformLocation(progTonemap, "u_gain0");
  const ulTonemap_gain1 = gl.getUniformLocation(progTonemap, "u_gain1");
  const ulTonemap_foodGain = gl.getUniformLocation(progTonemap, "u_food_gain");
  // bloom
  const ulBright_src = gl.getUniformLocation(progBright, "u_src");
  const ulBright_thresh = gl.getUniformLocation(progBright, "u_threshold");
  const ulBlur_src = gl.getUniformLocation(progBlur, "u_src");
  const ulBlur_dir = gl.getUniformLocation(progBlur, "u_dir");
  const ulComp_base = gl.getUniformLocation(progComposite, "u_base");
  const ulComp_bloom = gl.getUniformLocation(progComposite, "u_bloom");
  const ulComp_strength = gl.getUniformLocation(progComposite, "u_bloom_strength");

  // ---------------------------------------------------------------------------
  // Per-species EMA references for auto-exposure + shared food normalization.
  // Species 1 (magenta/coarse veins) may peak brighter than species 0;
  // independent EMAs prevent one washing out the other.
  // ---------------------------------------------------------------------------
  const emaRefs: number[] = new Array(speciesCount).fill(1.0);
  let emaFoodRef = 1.0;

  // ---------------------------------------------------------------------------
  // State objects shared with UI
  // ---------------------------------------------------------------------------
  const allParams: SimParams[] = [];
  const allEcology: EcologyParams[] = [];
  for (let s = 0; s < speciesCount; s++) {
    allParams.push(readSimParams(sim, s));
    allEcology.push(readEcologyParams(sim, s));
  }

  const spawn: SpawnState = { count: 500, pattern: 1, species: 0 };
  const transport: Transport = { paused: false, speed: 1, stepOnce: false };
  const seedRef = { value: initialSeed };

  // ---------------------------------------------------------------------------
  // Metrics ring buffer + food ceiling (captured on reset)
  // ---------------------------------------------------------------------------
  const metricsBuf = new MetricsBuffer();
  metricsBuf.foodCeiling = sim.food_total(); // food starts full right after new()

  let latestSample: MetricSample | null = null;

  function recordSample(): void {
    const s: MetricSample = {
      tick: sim.tick_count(),
      pop: [sim.species_population(0), sim.species_population(1)],
      mass: [sim.trail_mass(0), sim.trail_mass(1)],
      foodTotal: sim.food_total(),
      foodCoverage: sim.food_coverage(),
    };
    metricsBuf.push(s);
    latestSample = s;
  }

  // ---------------------------------------------------------------------------
  // Pane reference (needed to call pane.refresh() after URL-load)
  // ---------------------------------------------------------------------------
  const paneRef: { pane: Pane | null } = { pane: null };

  // ---------------------------------------------------------------------------
  // Reset handler
  // ---------------------------------------------------------------------------
  function doReset(): void {
    sim.reset(seedRef.value);
    views = refreshViews(wasm, sim);
    for (let s = 0; s < speciesCount; s++) emaRefs[s] = 1.0;
    emaFoodRef = 1.0;
    metricsBuf.clear();
    metricsBuf.foodCeiling = sim.food_total();
    latestSample = null;
  }

  buildPane(sim, allParams, allEcology, spawn, transport, doReset, seedRef, metricsBuf, paneRef);

  // ---------------------------------------------------------------------------
  // Apply shared state from URL hash (after pane is built so we can refresh it)
  // ---------------------------------------------------------------------------
  if (sharedState) {
    const paramObjects: SimParamObjects = { allParams, allEcology };
    applySharedState(sim, sharedState, paramObjects);
    // Re-read food ceiling now that seed is applied.
    metricsBuf.foodCeiling = sim.food_total();
    paneRef.pane?.refresh();
  }

  attachSpawnHandler(canvas, sim, spawn, () => {
    views = refreshViews(wasm, sim);
  });

  // ---------------------------------------------------------------------------
  // Hover inspector
  // ---------------------------------------------------------------------------
  // Wrap sim in a closure so the inspector always sees the current instance.
  attachInspector(canvas, inspectorEl, () => sim);

  // ---------------------------------------------------------------------------
  // HUD timing
  // ---------------------------------------------------------------------------
  let frames = 0;
  let last = performance.now();
  let fps = 0;

  // ---------------------------------------------------------------------------
  // Render loop
  // ---------------------------------------------------------------------------
  function frame(now: number): void {
    // --- Tick sim (respecting transport) ------------------------------------
    if (!transport.paused) {
      for (let i = 0; i < transport.speed; i++) sim.tick();
    } else if (transport.stepOnce) {
      sim.tick();
    }
    transport.stepOnce = false;

    // Sample metrics once per frame (x-axis is tick_count, reproducible).
    recordSample();

    // Upload all trail textures and food texture after all ticks this frame.
    for (let s = 0; s < speciesCount; s++) {
      uploadField(gl, trailTexs[s], WIDTH, HEIGHT, views.trails[s]);
    }
    uploadField(gl, foodTex, WIDTH, HEIGHT, views.food);

    // Per-species EMA-stabilised auto-exposure — prevents one species from
    // visually dominating the other when their trail maxima diverge.
    const gains: number[] = [];
    for (let s = 0; s < speciesCount; s++) {
      const rawMax = Math.max(1e-3, sim.field_max(s));
      emaRefs[s] = emaRefs[s] + EMA_ALPHA * (rawMax - emaRefs[s]);
      gains.push(GAIN_TARGET / emaRefs[s]);
    }

    // EMA-stabilised food normalization.
    const rawFoodMax = Math.max(0.1, sim.food_max());
    emaFoodRef = emaFoodRef + EMA_ALPHA * (rawFoodMax - emaFoodRef);
    const foodGain = 1.0 / Math.max(0.1, emaFoodRef);

    gl.bindVertexArray(vao);

    // ------------------------------------------------------------------
    // Pass 1 — Tone-map → base FBO (full resolution)
    // ------------------------------------------------------------------
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbos.base.fbo);
    gl.viewport(0, 0, fbos.base.width, fbos.base.height);
    gl.useProgram(progTonemap);

    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, trailTexs[0]);
    gl.uniform1i(ulTonemap_field0, 0);

    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, trailTexs[1]);
    gl.uniform1i(ulTonemap_field1, 1);

    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, foodTex);
    gl.uniform1i(ulTonemap_food, 2);

    gl.uniform1f(ulTonemap_gain0, gains[0]);
    gl.uniform1f(ulTonemap_gain1, gains[1]);
    gl.uniform1f(ulTonemap_foodGain, foodGain);

    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // ------------------------------------------------------------------
    // Pass 2 — Bright-pass → ping FBO (half resolution)
    // ------------------------------------------------------------------
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbos.ping.fbo);
    gl.viewport(0, 0, fbos.ping.width, fbos.ping.height);
    gl.useProgram(progBright);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, fbos.base.tex);
    gl.uniform1i(ulBright_src, 0);
    gl.uniform1f(ulBright_thresh, BLOOM_THRESHOLD);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // ------------------------------------------------------------------
    // Pass 3 — Separable Gaussian blur (BLUR_PASSES × horizontal+vertical)
    // ping → pong → ping → ...  final result in srcFBO
    // ------------------------------------------------------------------
    const invW = 1.0 / fbos.ping.width;
    const invH = 1.0 / fbos.ping.height;
    let srcFBO = fbos.ping;
    let dstFBO = fbos.pong;

    for (let i = 0; i < BLUR_PASSES; i++) {
      // Horizontal
      gl.bindFramebuffer(gl.FRAMEBUFFER, dstFBO.fbo);
      gl.viewport(0, 0, dstFBO.width, dstFBO.height);
      gl.useProgram(progBlur);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, srcFBO.tex);
      gl.uniform1i(ulBlur_src, 0);
      gl.uniform2f(ulBlur_dir, invW, 0.0);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      [srcFBO, dstFBO] = [dstFBO, srcFBO];

      // Vertical
      gl.bindFramebuffer(gl.FRAMEBUFFER, dstFBO.fbo);
      gl.viewport(0, 0, dstFBO.width, dstFBO.height);
      gl.useProgram(progBlur);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, srcFBO.tex);
      gl.uniform1i(ulBlur_src, 0);
      gl.uniform2f(ulBlur_dir, 0.0, invH);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      [srcFBO, dstFBO] = [dstFBO, srcFBO];
    }
    // srcFBO now holds the blurred bloom image.
    const bloomTex = srcFBO.tex;

    // ------------------------------------------------------------------
    // Pass 4 — Composite: base + additive bloom → canvas
    // ------------------------------------------------------------------
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.useProgram(progComposite);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, fbos.base.tex);
    gl.uniform1i(ulComp_base, 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, bloomTex);
    gl.uniform1i(ulComp_bloom, 1);
    gl.uniform1f(ulComp_strength, BLOOM_STRENGTH);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // ------------------------------------------------------------------
    // Sparklines overlay — drawn on the 2D canvas, separate from WebGL
    // ------------------------------------------------------------------
    drawSparklines(
      sparkCtx,
      sparklinesCanvas.width,
      sparklinesCanvas.height,
      metricsBuf,
      latestSample,
    );

    // ------------------------------------------------------------------
    // HUD — per-species population so competition is legible
    // ------------------------------------------------------------------
    frames++;
    if (now - last >= 500) {
      fps = Math.round((frames * 1000) / (now - last));
      frames = 0;
      last = now;
      const pop0 = sim.species_population(0);
      const pop1 = sim.species_population(1);
      const pausedTag = transport.paused ? " · paused" : "";
      hud.textContent =
        `Petri Polis · ${WIDTH}×${HEIGHT}` +
        ` · cyan ${pop0} · magenta ${pop1}` +
        ` · tick ${sim.tick_count()} · ${fps} fps${pausedTag}`;
    }

    requestAnimationFrame(frame);
  }

  requestAnimationFrame(frame);
}

main().catch((err) => {
  const hud = document.getElementById("hud");
  if (hud) hud.textContent = `error: ${err instanceof Error ? err.message : String(err)}`;
  console.error(err);
});
