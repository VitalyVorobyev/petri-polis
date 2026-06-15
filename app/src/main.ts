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
import { DEFAULT_PRESET_ID, PRESETS, presetById, type Scenario } from "./presets";
import {
  type BloomFBO,
  compileProgram,
  createBloomFBO,
  createFieldTexture,
  createGL,
  createLabelTexture,
  createObstacleTexture,
  uploadField,
  uploadLabels,
  uploadObstacle,
} from "./render/gl";
import {
  type CrossSense,
  decodeHash,
  encodeState,
  type GeometryTag,
  type SpeciesState,
} from "./urlstate";
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
uniform sampler2D u_obstacle; // wall mask (R8: 0 = open, ~0.0039 = wall)
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
  // --- Walls ---
  // R8 is normalized, so a wall (stored as 1) reads back as ~1/255; open cells
  // are exactly 0. A wall is a dim desaturated slate, kept well under the bloom
  // bright-pass threshold (~0.4) so it never glows. Skip trail + food there.
  bool isWall = texture(u_obstacle, v_uv).r > 0.0;
  if (isWall) {
    o = vec4(vec3(0.07, 0.09, 0.12), 1.0);
    return;
  }

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
// Component map — color each connected component a distinct hue.
// The label texture is R32UI (one u32 per cell, 0 = background). A cheap integer
// hash of the id → hue makes adjacent components contrast strongly. Background
// (id 0) → near-black; walls → a dim slate matching the tone-map's wall colour.
// Sampled with NEAREST so component boundaries stay crisp (no interpolation).
// ---------------------------------------------------------------------------
const FRAG_COMPONENTS = `#version 300 es
precision highp float;
precision highp int;
precision highp usampler2D;
in vec2 v_uv;
uniform usampler2D u_labels;  // R32UI: 0 = background, else component id
uniform sampler2D u_obstacle; // wall mask (R8)
out vec4 o;

vec3 hsv2rgb(vec3 c) {
  vec3 p = abs(fract(c.xxx + vec3(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
  return c.z * mix(vec3(1.0), clamp(p - 1.0, 0.0, 1.0), c.y);
}

void main() {
  if (texture(u_obstacle, v_uv).r > 0.0) {
    o = vec4(vec3(0.07, 0.09, 0.12), 1.0); // wall slate (matches tone-map)
    return;
  }
  uint id = texture(u_labels, v_uv).r;
  if (id == 0u) {
    o = vec4(0.02, 0.025, 0.04, 1.0); // background near-black
    return;
  }
  // Integer hash → hue. Mixing the id through a couple of multiplies spreads
  // consecutive ids across the wheel so neighbours rarely share a hue.
  uint h = id * 2654435761u;
  float hue = float(h & 0xffffu) / 65535.0;
  float sat = 0.65 + float((h >> 16) & 0xffu) / 255.0 * 0.30;
  o = vec4(hsv2rgb(vec3(hue, sat, 1.0)), 1.0);
}`;

// ---------------------------------------------------------------------------
// Long-exposure accumulate — fold the current tone-mapped frame into a
// persistent RGBA16F buffer: accum = max(accum * fade, current). The slow fade
// keeps a fading memory of where the network has been, so its full history is
// visible as a time-integrated glow. Output goes to the accumulator FBO; the
// shader reads the previous accumulator and the current base in one pass.
// ---------------------------------------------------------------------------
const FRAG_ACCUM = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_prev;     // previous accumulator (the running integral)
uniform sampler2D u_base;     // this frame's tone-mapped base
uniform sampler2D u_bloom;    // this frame's blurred bloom
uniform float u_strength;     // bloom blend weight (matches the Normal composite)
uniform float u_fade;         // per-frame decay of the memory (e.g. 0.985)
out vec4 o;

void main() {
  // This frame's image = the exact Normal composite (base + additive bloom).
  vec3 cur = texture(u_base, v_uv).rgb + texture(u_bloom, v_uv).rgb * u_strength;
  // Fold it into the faded running integral, keeping the brightest history.
  vec3 prev = texture(u_prev, v_uv).rgb * u_fade;
  o = vec4(max(prev, cur), 1.0);
}`;

// ---------------------------------------------------------------------------
// Endpoint markers — one glowing ring per endpoint, drawn additively over the
// final composite so they mark sources/sinks at a glance. Each endpoint is one
// gl.POINTS vertex; the vertex shader looks up its centre/size from a uniform
// array (sim coords → clip space with the same Y-flip the spawn handler uses).
// The fragment shader draws a soft ring within the point sprite.
// ---------------------------------------------------------------------------
const MAX_MARKERS = 64;

const VERT_MARKER = `#version 300 es
precision highp float;
uniform vec2 u_centers[${MAX_MARKERS}];  // clip-space centres
uniform float u_sizes[${MAX_MARKERS}];   // point sizes (px)
void main() {
  gl_Position = vec4(u_centers[gl_VertexID], 0.0, 1.0);
  gl_PointSize = u_sizes[gl_VertexID];
}`;

const FRAG_MARKER = `#version 300 es
precision highp float;
uniform vec3 u_color;
out vec4 o;
void main() {
  // gl_PointCoord is [0,1] over the sprite; centre the radial coordinate.
  vec2 p = gl_PointCoord * 2.0 - 1.0;
  float r = length(p);
  // A bright ring near the rim, fading to transparent inside and outside.
  float ring = smoothstep(0.55, 0.78, r) * (1.0 - smoothstep(0.92, 1.0, r));
  float core = (1.0 - smoothstep(0.0, 0.30, r)) * 0.5; // soft centre dot
  float a = clamp(ring + core, 0.0, 1.0);
  if (a <= 0.001) discard;
  o = vec4(u_color * a, a);
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
// Render modes + long-exposure
// ---------------------------------------------------------------------------
type RenderMode = "normal" | "components" | "long-exposure";

const LONG_EXPOSURE_FADE = 0.985; // per-frame decay of the accumulator memory

// Structure metrics are O(N) reductions (union-find, box-counting,
// autocorrelation) — far heavier than the per-frame sum reductions. Sample them
// on a throttled cadence and reuse the last value between samples so fps stays
// smooth.
const STRUCTURE_SAMPLE_INTERVAL = 20; // frames between structure-metric samples

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

function makeObstacleView(wasm: { memory: WebAssembly.Memory }, sim: Sim): Uint8Array {
  return new Uint8Array(wasm.memory.buffer, sim.obstacle_ptr(), sim.obstacle_len());
}

// The component-label buffer is a u32-per-cell view (0 = background, else a
// 1..=component_count id). It is filled as a side effect of component_count() /
// loop_count() — all-zero before any such call — but its pointer is valid from
// construction, so the view can be built eagerly here like the others.
function makeLabelsView(wasm: { memory: WebAssembly.Memory }, sim: Sim): Uint32Array {
  return new Uint32Array(
    wasm.memory.buffer,
    sim.component_labels_ptr(),
    sim.component_labels_len(),
  );
}

// Re-fetch all zero-copy views and return them. Call after every reset-class
// op (reset / spawn / load_maze_demo / add_endpoint / clear_endpoints): WASM
// memory may grow and detach every view, so all of them must be rebuilt.
function refreshViews(
  wasm: { memory: WebAssembly.Memory },
  sim: Sim,
): { trails: Float32Array[]; food: Float32Array; obstacle: Uint8Array; labels: Uint32Array } {
  const count = sim.species_count();
  const trails: Float32Array[] = [];
  for (let s = 0; s < count; s++) {
    trails.push(makeTrailView(wasm, sim, s));
  }
  return {
    trails,
    food: makeFoodView(wasm, sim),
    obstacle: makeObstacleView(wasm, sim),
    labels: makeLabelsView(wasm, sim),
  };
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
// Geometry state — drives the paint-tool dispatch in the canvas pointer handler.
// ---------------------------------------------------------------------------
type GeometryTool = "spawn" | "paint-wall" | "erase-wall" | "place-endpoint";

interface GeometryState {
  tool: GeometryTool;
  brushRadius: number; // wall brush radius (cells)
  endpointRadius: number; // endpoint well radius (cells)
}

// ---------------------------------------------------------------------------
// Chemotaxis state — per-species food-attraction (chemotaxis) weight.
// ---------------------------------------------------------------------------
interface ChemotaxisParams {
  food_attraction: number;
}

// ---------------------------------------------------------------------------
// Network (reachability metric) state — the threshold knob.
// ---------------------------------------------------------------------------
interface NetworkParams {
  network_threshold: number;
}

// ---------------------------------------------------------------------------
// Cross-species sensing state — the two off-diagonal coupling weights.
//   s01 = species 0 (cyan) senses species 1's (magenta) trail.
//   s10 = species 1 (magenta) senses species 0's (cyan) trail.
// Positive = attracted to the other's trail, negative = repelled. The own-trail
// diagonals stay at 1.0 and aren't exposed.
// ---------------------------------------------------------------------------
interface CrossSenseParams {
  s01: number;
  s10: number;
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
// Presets state — the active scenario id (drives the Presets dropdown + caption).
// ---------------------------------------------------------------------------
interface PresetsState {
  id: string;
}

// ---------------------------------------------------------------------------
// Render state — the active render mode (drives the Render mode dropdown).
//   normal        — bioluminescent tone-map + bloom (default, unchanged).
//   components    — connected-component map (each component a distinct hue).
//   long-exposure — time-integrated trail accumulated across frames.
// ---------------------------------------------------------------------------
interface RenderState {
  mode: RenderMode;
}

// ---------------------------------------------------------------------------
// Programmatic-refresh guard. When we sync the panel from the sim (after a
// preset or shared-link load) we call `pane.refresh()`, which fires the
// bindings' `change` events. Those handlers would otherwise write the widgets'
// step-quantized values back into the sim, perturbing it (e.g. nudging a
// repro_threshold of 1.12 off by a hair). The sim is authoritative during a
// programmatic sync, so we suppress binding writes while this flag is set.
// ---------------------------------------------------------------------------
let suppressBindingWrites = false;

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
  chemotaxis: ChemotaxisParams,
): void {
  const folder = pane.addFolder({ title: label, expanded: false });

  const onParams = (): void => {
    if (suppressBindingWrites) return;
    applySimParams(sim, species, params);
  };
  const onEcology = (): void => {
    if (suppressBindingWrites) return;
    applyEcologyParams(sim, species, ecology);
  };

  // Physarum parameters
  folder
    .addBinding(params, "sensor_angle", { label: "sensor angle", min: 0, max: 1.2, step: 0.01 })
    .on("change", onParams);

  folder
    .addBinding(params, "sensor_distance", { label: "sensor dist", min: 1, max: 32, step: 0.5 })
    .on("change", onParams);

  folder
    .addBinding(params, "rotation_angle", { label: "rotation angle", min: 0, max: 1.2, step: 0.01 })
    .on("change", onParams);

  folder
    .addBinding(params, "step_size", { label: "step size", min: 0.2, max: 3.0, step: 0.05 })
    .on("change", onParams);

  folder
    .addBinding(params, "deposit", { label: "deposit", min: 0.5, max: 20, step: 0.5 })
    .on("change", onParams);

  folder
    .addBinding(params, "decay", { label: "decay", min: 0.8, max: 0.99, step: 0.005 })
    .on("change", onParams);

  folder
    .addBinding(params, "diffuse_weight", { label: "blur weight", min: 0, max: 1, step: 0.01 })
    .on("change", onParams);

  // Ecology parameters
  folder
    .addBinding(ecology, "metabolism", { label: "metabolism", min: 0.001, max: 0.02, step: 0.001 })
    .on("change", onEcology);

  folder
    .addBinding(ecology, "eat_rate", { label: "eat rate", min: 0.02, max: 0.3, step: 0.01 })
    .on("change", onEcology);

  folder
    .addBinding(ecology, "repro_threshold", {
      label: "repro threshold",
      min: 0.6,
      max: 2.5,
      step: 0.05,
    })
    .on("change", onEcology);

  folder
    .addBinding(ecology, "food_regrow", {
      label: "food regrow",
      min: 0.001,
      max: 0.02,
      step: 0.0005,
    })
    .on("change", onEcology);

  folder
    .addBinding(ecology, "death_return", { label: "death return", min: 0, max: 1, step: 0.05 })
    .on("change", onEcology);

  // Chemotaxis — steers agents up the food gradient toward endpoints.
  folder
    .addBinding(chemotaxis, "food_attraction", {
      label: "food attraction",
      min: 0,
      max: 10,
      step: 0.1,
    })
    .on("change", () => {
      if (suppressBindingWrites) return;
      sim.set_food_attraction(species, chemotaxis.food_attraction);
    });
}

// ---------------------------------------------------------------------------
// Build the Tweakpane control panel
// ---------------------------------------------------------------------------
function buildPane(
  sim: Sim,
  allParams: SimParams[],
  allEcology: EcologyParams[],
  allChemotaxis: ChemotaxisParams[],
  crossSense: CrossSenseParams,
  network: NetworkParams,
  spawn: SpawnState,
  geometry: GeometryState,
  transport: Transport,
  presets: PresetsState,
  render: RenderState,
  onReset: () => void,
  onLoadMaze: () => void,
  onClearGeometry: () => void,
  onApplyPreset: (id: string) => void,
  onRenderModeChange: () => void,
  seedRef: { value: number },
  metricsBuf: MetricsBuffer,
  paneRef: { pane: Pane | null },
  captionRef: { value: string },
  geometryTagRef: { value: GeometryTag },
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

  // --- Render mode ---------------------------------------------------------
  // How the field is presented: the default bioluminescent tone-map, a
  // connected-component map (each component a distinct hue — makes the
  // components metric visible), or a long-exposure time-integration of the
  // network. Normal is byte-for-byte the default renderer.
  pane
    .addBinding(render, "mode", {
      label: "render mode",
      view: "list",
      options: [
        { text: "Normal", value: "normal" },
        { text: "Component map", value: "components" },
        { text: "Long-exposure", value: "long-exposure" },
      ],
    })
    .on("change", () => onRenderModeChange());

  // --- Presets (the lab bench) ---------------------------------------------
  // A menu of named classical complex-systems scenarios. Selecting one applies
  // it immediately (full reset to its canonical state). The caption monitor
  // names the active scenario.
  const presetsFolder = pane.addFolder({ title: "Presets", expanded: true });

  presetsFolder
    .addBinding(presets, "id", {
      label: "scenario",
      view: "list",
      options: PRESETS.map((p) => ({ text: p.name, value: p.id })),
    })
    .on("change", (ev) => onApplyPreset(ev.value));

  presetsFolder.addBinding(captionRef, "value", {
    label: "about",
    readonly: true,
    multiline: true,
    rows: 2,
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
    addSpeciesFolder(pane, speciesLabels[s], sim, s, allParams[s], allEcology[s], allChemotaxis[s]);
  }

  // --- Cross-species sensing -----------------------------------------------
  // The two off-diagonal coupling weights: how strongly each species is pulled
  // by the OTHER's trail. Positive = attracted, negative = repelled. The
  // own-trail diagonals stay at 1.0 and aren't exposed.
  const crossFolder = pane.addFolder({ title: "Cross-species sensing", expanded: false });

  crossFolder
    .addBinding(crossSense, "s01", {
      label: "cyan senses magenta",
      min: -1,
      max: 1,
      step: 0.05,
    })
    .on("change", () => {
      if (suppressBindingWrites) return;
      sim.set_cross_sense(0, 1, crossSense.s01);
    });

  crossFolder
    .addBinding(crossSense, "s10", {
      label: "magenta senses cyan",
      min: -1,
      max: 1,
      step: 0.05,
    })
    .on("change", () => {
      if (suppressBindingWrites) return;
      sim.set_cross_sense(1, 0, crossSense.s10);
    });

  // --- Geometry (walls + endpoints) ----------------------------------------
  const geomFolder = pane.addFolder({ title: "Geometry (click canvas)", expanded: false });

  geomFolder.addBinding(geometry, "tool", {
    label: "tool",
    view: "list",
    options: [
      { text: "spawn", value: "spawn" },
      { text: "paint wall", value: "paint-wall" },
      { text: "erase wall", value: "erase-wall" },
      { text: "place endpoint", value: "place-endpoint" },
    ],
  });

  geomFolder.addBinding(geometry, "brushRadius", {
    label: "brush (cells)",
    min: 1,
    max: 24,
    step: 1,
  });

  geomFolder.addBinding(geometry, "endpointRadius", {
    label: "endpoint (cells)",
    min: 2,
    max: 24,
    step: 1,
  });

  geomFolder.addButton({ title: "Load maze demo" }).on("click", onLoadMaze);
  geomFolder.addButton({ title: "Clear geometry" }).on("click", onClearGeometry);

  // --- Network (reachability metric) ---------------------------------------
  const networkFolder = pane.addFolder({ title: "Network", expanded: false });

  networkFolder
    .addBinding(network, "network_threshold", {
      label: "threshold",
      min: 0,
      max: 1,
      step: 0.005,
    })
    .on("change", () => {
      if (suppressBindingWrites) return;
      sim.set_network_threshold(network.network_threshold);
    });

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
    // Capture the full scenario: seed + per-species params/ecology/chemotaxis +
    // reachability threshold + endpoints + the built-in geometry tag (the maze's
    // wall mask is regenerated from the tag on load; hand-painted walls are not
    // serialized, by design).
    const hash = encodeState(sim, seedRef.value, geometryTagRef.value);
    location.hash = hash;
    navigator.clipboard.writeText(location.href).catch(() => {
      // clipboard may be unavailable (non-HTTPS); at least the hash is set
    });
  });
}

// ---------------------------------------------------------------------------
// Canvas pointer handler — dispatches on the active Geometry tool.
//   spawn          → sim.spawn (reset-class → re-fetch views)
//   paint/erase    → sim.paint_obstacle (in-place, supports click-drag)
//   place-endpoint → sim.add_endpoint (reset-class → re-fetch views)
// All tools share the same pointer→sim-coord mapping with the Y-flip.
// ---------------------------------------------------------------------------
function attachCanvasHandler(
  canvas: HTMLCanvasElement,
  sim: Sim,
  spawn: SpawnState,
  geometry: GeometryState,
  onRefetch: () => void,
): void {
  // Map a pointer event to sim cell coords (Y-flipped to match the renderer).
  function toSim(e: PointerEvent): { sx: number; sy: number } {
    const rect = canvas.getBoundingClientRect();
    const u = (e.clientX - rect.left) / rect.width;
    const v = (e.clientY - rect.top) / rect.height;
    // The fullscreen triangle maps uv(0,0) to the bottom-left of clip space,
    // so field row 0 renders at the bottom of the screen. Flip V to correct.
    return { sx: u * sim.width(), sy: (1 - v) * sim.height() };
  }

  let painting = false;

  function apply(e: PointerEvent, isDrag: boolean): void {
    const { sx, sy } = toSim(e);
    switch (geometry.tool) {
      case "spawn":
        if (isDrag) return; // spawn only on the initial press
        sim.spawn(sx, sy, spawn.count, spawn.pattern, spawn.species);
        onRefetch(); // reset-class
        break;
      case "paint-wall":
        sim.paint_obstacle(sx, sy, geometry.brushRadius, true); // in-place
        break;
      case "erase-wall":
        sim.paint_obstacle(sx, sy, geometry.brushRadius, false); // in-place
        break;
      case "place-endpoint":
        if (isDrag) return; // one endpoint per press
        sim.add_endpoint(sx, sy, geometry.endpointRadius);
        onRefetch(); // reset-class
        break;
    }
  }

  canvas.addEventListener("pointerdown", (e: PointerEvent) => {
    painting = true;
    canvas.setPointerCapture(e.pointerId);
    apply(e, false);
  });

  canvas.addEventListener("pointermove", (e: PointerEvent) => {
    if (!painting) return;
    // Only the wall tools paint continuously while dragging.
    if (geometry.tool === "paint-wall" || geometry.tool === "erase-wall") {
      apply(e, true);
    }
  });

  const endPaint = (e: PointerEvent): void => {
    painting = false;
    if (canvas.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
  };
  canvas.addEventListener("pointerup", endPaint);
  canvas.addEventListener("pointercancel", endPaint);
}

// ---------------------------------------------------------------------------
// FBO allocation helpers
// ---------------------------------------------------------------------------
interface FBOSet {
  base: BloomFBO;
  ping: BloomFBO;
  pong: BloomFBO;
  // Long-exposure accumulator (full resolution, RGBA16F). Ping-ponged because
  // the accumulate pass reads the previous accumulator while writing the next.
  accumA: BloomFBO;
  accumB: BloomFBO;
}

function allocFBOs(gl: WebGL2RenderingContext, w: number, h: number): FBOSet {
  const bw = Math.max(1, Math.floor(w * BLOOM_SCALE));
  const bh = Math.max(1, Math.floor(h * BLOOM_SCALE));
  return {
    base: createBloomFBO(gl, w, h),
    ping: createBloomFBO(gl, bw, bh),
    pong: createBloomFBO(gl, bw, bh),
    accumA: createBloomFBO(gl, w, h),
    accumB: createBloomFBO(gl, w, h),
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
  const progMarker = compileProgram(gl, VERT_MARKER, FRAG_MARKER);
  const progComponents = compileProgram(gl, VERT, FRAG_COMPONENTS);
  const progAccum = compileProgram(gl, VERT, FRAG_ACCUM);

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
  // Obstacle (wall) mask texture (R8, updated each frame)
  // ---------------------------------------------------------------------------
  const obstacleTex = createObstacleTexture(gl, WIDTH, HEIGHT);

  // ---------------------------------------------------------------------------
  // Component-label texture (R32UI) — uploaded only while the component-map
  // render mode is active, from the labels buffer the sim fills as a side
  // effect of component_count().
  // ---------------------------------------------------------------------------
  const labelTex = createLabelTexture(gl, WIDTH, HEIGHT);

  // ---------------------------------------------------------------------------
  // Render mode + long-exposure accumulator bookkeeping. `accumActive` tracks
  // which of the two accumulator FBOs holds the latest integral (they ping-pong
  // each frame). `accumCleared` forces the accumulator to be wiped on the next
  // long-exposure frame after a reset / preset / seed change / resize, so the
  // network history doesn't bleed across worlds.
  // ---------------------------------------------------------------------------
  const render: RenderState = { mode: "normal" };
  let accumActive = 0; // 0 → accumA holds the latest, 1 → accumB
  let accumCleared = false; // whether the active accumulator currently holds a valid integral

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
    accumCleared = false; // freshly-allocated accumulator FBOs need a clear
  }
  window.addEventListener("resize", resize);
  resize();

  // ---------------------------------------------------------------------------
  // Uniform locations — tone-map
  // ---------------------------------------------------------------------------
  const ulTonemap_field0 = gl.getUniformLocation(progTonemap, "u_field0");
  const ulTonemap_field1 = gl.getUniformLocation(progTonemap, "u_field1");
  const ulTonemap_food = gl.getUniformLocation(progTonemap, "u_food");
  const ulTonemap_obstacle = gl.getUniformLocation(progTonemap, "u_obstacle");
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
  // markers
  const ulMarker_centers = gl.getUniformLocation(progMarker, "u_centers");
  const ulMarker_sizes = gl.getUniformLocation(progMarker, "u_sizes");
  const ulMarker_color = gl.getUniformLocation(progMarker, "u_color");
  // component map
  const ulComp_labels = gl.getUniformLocation(progComponents, "u_labels");
  const ulComp_obstacle = gl.getUniformLocation(progComponents, "u_obstacle");
  // long-exposure accumulate
  const ulAccum_prev = gl.getUniformLocation(progAccum, "u_prev");
  const ulAccum_base = gl.getUniformLocation(progAccum, "u_base");
  const ulAccum_bloom = gl.getUniformLocation(progAccum, "u_bloom");
  const ulAccum_strength = gl.getUniformLocation(progAccum, "u_strength");
  const ulAccum_fade = gl.getUniformLocation(progAccum, "u_fade");

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
  const allChemotaxis: ChemotaxisParams[] = [];
  for (let s = 0; s < speciesCount; s++) {
    allParams.push(readSimParams(sim, s));
    allEcology.push(readEcologyParams(sim, s));
    allChemotaxis.push({ food_attraction: sim.food_attraction(s) });
  }

  const network: NetworkParams = { network_threshold: sim.network_threshold() };
  const crossSense: CrossSenseParams = {
    s01: sim.cross_sense(0, 1),
    s10: sim.cross_sense(1, 0),
  };
  const spawn: SpawnState = { count: 500, pattern: 1, species: 0 };
  const geometry: GeometryState = { tool: "spawn", brushRadius: 6, endpointRadius: 8 };
  const transport: Transport = { paused: false, speed: 1, stepOnce: false };
  const seedRef = { value: initialSeed };

  // Active preset + its caption, and the built-in geometry tag of the current
  // scenario (what a share link regenerates: "maze" → load_maze_demo; "none" →
  // open world). Hand-painting walls is intentionally NOT reflected in this tag.
  const presets: PresetsState = { id: DEFAULT_PRESET_ID };
  const captionRef = { value: presetById(DEFAULT_PRESET_ID)?.caption ?? "" };
  const geometryTagRef: { value: GeometryTag } = { value: "none" };

  function clearAccumulator(): void {
    accumCleared = false; // request a GPU clear on the next long-exposure frame
  }

  // ---------------------------------------------------------------------------
  // Metrics ring buffer + food ceiling (captured on reset)
  // ---------------------------------------------------------------------------
  const metricsBuf = new MetricsBuffer();
  metricsBuf.foodCeiling = sim.food_total(); // food starts full right after new()

  let latestSample: MetricSample | null = null;

  // Cached structure metrics — the four O(N) reductions are refreshed only on a
  // throttled cadence (see STRUCTURE_SAMPLE_INTERVAL) and reused in between so
  // fps stays smooth. `structureFrame` counts frames toward the next refresh.
  const structure = {
    componentCount: 0,
    loopCount: 0,
    fractalDimension: 0,
    autocorrLength: 0,
  };
  let structureFrame = STRUCTURE_SAMPLE_INTERVAL; // force a sample on the first frame

  // Refresh the cached structure metrics from the sim (heavy O(N) reductions).
  // component_count() / loop_count() also fill the component-labels buffer as a
  // side effect.
  function refreshStructureMetrics(): void {
    structure.componentCount = sim.component_count();
    structure.loopCount = sim.loop_count();
    structure.fractalDimension = sim.fractal_dimension();
    structure.autocorrLength = sim.autocorrelation_length();
  }

  function recordSample(): void {
    // Refresh the heavy structure metrics only on the throttled cadence; reuse
    // the cached values for every frame in between.
    if (structureFrame >= STRUCTURE_SAMPLE_INTERVAL) {
      refreshStructureMetrics();
      structureFrame = 0;
    }
    structureFrame++;

    const s: MetricSample = {
      tick: sim.tick_count(),
      pop: [sim.species_population(0), sim.species_population(1)],
      mass: [sim.trail_mass(0), sim.trail_mass(1)],
      foodTotal: sim.food_total(),
      foodCoverage: sim.food_coverage(),
      connected: sim.endpoints_connected(),
      endpointCount: sim.endpoint_count(),
      networkCost: sim.network_cost(),
      componentCount: structure.componentCount,
      loopCount: structure.loopCount,
      fractalDimension: structure.fractalDimension,
      autocorrLength: structure.autocorrLength,
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
    structureFrame = STRUCTURE_SAMPLE_INTERVAL; // re-sample structure next frame
    clearAccumulator();
  }

  // Sync the mutable param objects + Tweakpane from the sim's current state.
  // Called after any reset-class op so the panel mirrors the live sim. The
  // refresh fires binding `change` events; we suppress their writes so the
  // step-quantized widget values don't get pushed back into the authoritative
  // sim state.
  function syncPanelFromSim(): void {
    for (let s = 0; s < speciesCount; s++) {
      Object.assign(allParams[s], readSimParams(sim, s));
      Object.assign(allEcology[s], readEcologyParams(sim, s));
      allChemotaxis[s].food_attraction = sim.food_attraction(s);
    }
    network.network_threshold = sim.network_threshold();
    crossSense.s01 = sim.cross_sense(0, 1);
    crossSense.s10 = sim.cross_sense(1, 0);
    suppressBindingWrites = true;
    paneRef.pane?.refresh();
    suppressBindingWrites = false;
  }

  // Load the built-in maze scenario — reset-class (rewrites obstacles,
  // endpoints, food, population), so re-fetch all views. Sync the param panel
  // since food-attraction is turned on by the demo.
  function doLoadMaze(): void {
    sim.load_maze_demo();
    geometryTagRef.value = "maze";
    views = refreshViews(wasm, sim);
    for (let s = 0; s < speciesCount; s++) emaRefs[s] = 1.0;
    emaFoodRef = 1.0;
    metricsBuf.clear();
    metricsBuf.foodCeiling = sim.food_total();
    latestSample = null;
    structureFrame = STRUCTURE_SAMPLE_INTERVAL;
    clearAccumulator();
    syncPanelFromSim();
  }

  // Clear walls + endpoints. clear_obstacles is in-place; clear_endpoints is
  // reset-class (recomputes food caps), so re-fetch all views afterward.
  function doClearGeometry(): void {
    sim.clear_obstacles();
    sim.clear_endpoints();
    geometryTagRef.value = "none";
    views = refreshViews(wasm, sim);
    metricsBuf.foodCeiling = sim.food_total();
  }

  // ---------------------------------------------------------------------------
  // Apply a full scenario — the shared path for both presets and shared links.
  // Fully resets the world to the scenario's canonical state, in order:
  //   1. reset(seed)  — re-seed + respawn the default population (reset-class).
  //   2. per-species params / ecology / chemotaxis + network threshold +
  //      cross-species sensing (scalar setters; no re-fetch needed).
  //   3. rebuild geometry: clear_obstacles + clear_endpoints (so switching off
  //      the maze removes its walls), then either load_maze_demo (tag "maze")
  //      or add_endpoint(...) per the endpoint list, or nothing.
  //   4. re-fetch ALL zero-copy views (every reset-class call above may have
  //      grown/detached the views) and refresh the Tweakpane bindings.
  // The cross-sense matrix is ALWAYS set in full (off-diagonals default to 0,
  // diagonals pinned to 1), so switching to a scenario without coupling clears
  // any coupling left by the previous one — no stale coupling leaks between
  // presets.
  // ---------------------------------------------------------------------------
  function applyScenario(
    species: SpeciesState[],
    networkThreshold: number,
    geometryTag: GeometryTag,
    endpoints: { x: number; y: number; radius: number }[],
    seed: number,
    crossSenseState?: CrossSense,
  ): void {
    // 1. Re-seed + respawn the default population.
    seedRef.value = seed;
    sim.reset(seed);

    // 2. Per-species params, ecology, chemotaxis + reachability threshold.
    const n = Math.min(species.length, speciesCount);
    for (let s = 0; s < n; s++) {
      const sp = species[s];
      sim.set_params(
        s,
        sp.sensor_angle,
        sp.sensor_distance,
        sp.rotation_angle,
        sp.step_size,
        sp.deposit,
        sp.decay,
      );
      sim.set_diffuse_weight(s, sp.diffuse_weight);
      sim.set_ecology(
        s,
        sp.metabolism,
        sp.eat_rate,
        sp.repro_threshold,
        sp.food_regrow,
        sp.death_return,
      );
      sim.set_food_attraction(s, sp.food_attraction);
    }
    sim.set_network_threshold(networkThreshold);

    // Cross-species sensing — always set the full matrix so switching to a
    // scenario without coupling resets it to identity (own-trail diagonals at
    // 1.0, cross off-diagonals at 0.0). The diagonals are pinned to 1 here to
    // guard against any stale own-trail weight; the off-diagonals carry the
    // scenario's coupling (0 when omitted).
    sim.set_cross_sense(0, 0, 1.0);
    sim.set_cross_sense(1, 1, 1.0);
    sim.set_cross_sense(0, 1, crossSenseState?.s01 ?? 0);
    sim.set_cross_sense(1, 0, crossSenseState?.s10 ?? 0);

    // 3. Geometry. Always clear first so switching presets removes prior walls
    //    and endpoints, then lay down the scenario's built-in geometry.
    sim.clear_obstacles(); // in-place
    sim.clear_endpoints(); // reset-class
    if (geometryTag === "maze") {
      // The maze bakes in its own walls, endpoints, chemotaxis, and population —
      // it overrides whatever we set above, which is intentional.
      sim.load_maze_demo();
      geometryTagRef.value = "maze";
    } else {
      geometryTagRef.value = "none";
      for (const ep of endpoints) sim.add_endpoint(ep.x, ep.y, ep.radius); // reset-class
    }

    // 4. Re-fetch every view (multiple reset-class calls above) + resync panel.
    views = refreshViews(wasm, sim);
    for (let s = 0; s < speciesCount; s++) emaRefs[s] = 1.0;
    emaFoodRef = 1.0;
    metricsBuf.clear();
    metricsBuf.foodCeiling = sim.food_total();
    latestSample = null;
    structureFrame = STRUCTURE_SAMPLE_INTERVAL;
    clearAccumulator();
    syncPanelFromSim();
  }

  // Apply a preset by id (from the Presets dropdown). Pins the preset's seed if
  // it has one, otherwise keeps the current seed.
  function doApplyPreset(id: string): void {
    const preset = presetById(id);
    if (!preset) return;
    const sc: Scenario = preset.scenario;
    presets.id = id;
    captionRef.value = preset.caption;
    applyScenario(
      sc.species,
      sc.network_threshold,
      sc.geometry,
      sc.endpoints,
      sc.seed ?? seedRef.value,
      sc.crossSense,
    );
  }

  buildPane(
    sim,
    allParams,
    allEcology,
    allChemotaxis,
    crossSense,
    network,
    spawn,
    geometry,
    transport,
    presets,
    render,
    doReset,
    doLoadMaze,
    doClearGeometry,
    doApplyPreset,
    // Entering long-exposure (or switching modes back into it) starts a fresh
    // integral, so the previous mode's accumulator never bleeds through.
    () => {
      if (render.mode === "long-exposure") clearAccumulator();
    },
    seedRef,
    metricsBuf,
    paneRef,
    captionRef,
    geometryTagRef,
  );

  // ---------------------------------------------------------------------------
  // Apply shared state from URL hash (after pane is built so we can refresh it).
  // A shared link is a full scenario, applied through the same path presets use:
  // params/ecology/chemotaxis → threshold → geometry (maze tag → load_maze_demo,
  // else the endpoint list) → re-fetch views → refresh pane.
  // ---------------------------------------------------------------------------
  if (sharedState) {
    // A URL-loaded scenario is not necessarily a named preset; set a neutral
    // dropdown selection + caption before applying so the sync picks them up.
    presets.id = DEFAULT_PRESET_ID;
    captionRef.value = "Loaded from a shared link.";
    applyScenario(
      sharedState.species,
      sharedState.network_threshold,
      sharedState.geometry,
      sharedState.endpoints,
      sharedState.seed,
      sharedState.crossSense,
    );
  }

  attachCanvasHandler(canvas, sim, spawn, geometry, () => {
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

    // Upload all trail textures, food texture, and obstacle mask after all
    // ticks this frame.
    for (let s = 0; s < speciesCount; s++) {
      uploadField(gl, trailTexs[s], WIDTH, HEIGHT, views.trails[s]);
    }
    uploadField(gl, foodTex, WIDTH, HEIGHT, views.food);
    uploadObstacle(gl, obstacleTex, WIDTH, HEIGHT, views.obstacle);

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

    gl.activeTexture(gl.TEXTURE3);
    gl.bindTexture(gl.TEXTURE_2D, obstacleTex);
    gl.uniform1i(ulTonemap_obstacle, 3);

    gl.uniform1f(ulTonemap_gain0, gains[0]);
    gl.uniform1f(ulTonemap_gain1, gains[1]);
    gl.uniform1f(ulTonemap_foodGain, foodGain);

    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // ------------------------------------------------------------------
    // Mode dispatch. `drawMarkers` flags whether the endpoint rings go on top
    // (Normal + Long-exposure show them; the Component map is a pure
    // measurement view so it omits them).
    // ------------------------------------------------------------------
    let drawMarkers = true;

    if (render.mode === "components") {
      // --- Component map ---------------------------------------------------
      // The labels buffer is filled as a side effect of component_count(); call
      // it EVERY frame while this mode is active so the overlay is fresh, then
      // upload the labels and colour each component by a hash of its id.
      sim.component_count();
      uploadLabels(gl, labelTex, WIDTH, HEIGHT, views.labels);

      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.useProgram(progComponents);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, labelTex);
      gl.uniform1i(ulComp_labels, 0);
      gl.activeTexture(gl.TEXTURE1);
      gl.bindTexture(gl.TEXTURE_2D, obstacleTex);
      gl.uniform1i(ulComp_obstacle, 1);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      drawMarkers = false;
    } else {
      // --- Bloom (Normal + Long-exposure share the bright-pass + blur) -----
      // Pass 2 — Bright-pass → ping FBO (half resolution)
      gl.bindFramebuffer(gl.FRAMEBUFFER, fbos.ping.fbo);
      gl.viewport(0, 0, fbos.ping.width, fbos.ping.height);
      gl.useProgram(progBright);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, fbos.base.tex);
      gl.uniform1i(ulBright_src, 0);
      gl.uniform1f(ulBright_thresh, BLOOM_THRESHOLD);
      gl.drawArrays(gl.TRIANGLES, 0, 3);

      // Pass 3 — Separable Gaussian blur (BLUR_PASSES × horizontal+vertical)
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

      if (render.mode === "long-exposure") {
        // --- Long-exposure -------------------------------------------------
        // Fold this frame's Normal composite (base + bloom, computed inside the
        // accum shader) into a persistent accumulator with a slow fade, so the
        // network's whole history glows. The two accumulator FBOs alternate
        // roles each frame: `prev` = last frame's integral (read), `next` = the
        // new integral (written). Nothing is read and written at once.
        const prev = accumActive === 0 ? fbos.accumA : fbos.accumB;
        const next = accumActive === 0 ? fbos.accumB : fbos.accumA;

        // fade = 0 wipes the (just-allocated or invalidated) accumulator on its
        // first frame so no stale history bleeds across worlds; thereafter the
        // slow fade keeps a fading memory.
        const fade = accumCleared ? LONG_EXPOSURE_FADE : 0.0;
        accumCleared = true;

        gl.bindFramebuffer(gl.FRAMEBUFFER, next.fbo);
        gl.viewport(0, 0, next.width, next.height);
        gl.useProgram(progAccum);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, prev.tex);
        gl.uniform1i(ulAccum_prev, 0);
        gl.activeTexture(gl.TEXTURE1);
        gl.bindTexture(gl.TEXTURE_2D, fbos.base.tex);
        gl.uniform1i(ulAccum_base, 1);
        gl.activeTexture(gl.TEXTURE2);
        gl.bindTexture(gl.TEXTURE_2D, bloomTex);
        gl.uniform1i(ulAccum_bloom, 2);
        gl.uniform1f(ulAccum_strength, BLOOM_STRENGTH);
        gl.uniform1f(ulAccum_fade, fade);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        // `next` now holds the latest integral; make it the active accumulator.
        accumActive = next === fbos.accumA ? 0 : 1;

        // Present the accumulator to the screen (straight copy via the
        // composite program with bloom strength 0).
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.viewport(0, 0, canvas.width, canvas.height);
        gl.useProgram(progComposite);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, next.tex);
        gl.uniform1i(ulComp_base, 0);
        gl.activeTexture(gl.TEXTURE1);
        gl.bindTexture(gl.TEXTURE_2D, next.tex);
        gl.uniform1i(ulComp_bloom, 1);
        gl.uniform1f(ulComp_strength, 0.0); // present the integral as-is
        gl.drawArrays(gl.TRIANGLES, 0, 3);
      } else {
        // --- Normal — Pass 4: composite base + additive bloom → canvas ------
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
      }
    }

    // ------------------------------------------------------------------
    // Pass 5 — Endpoint markers: additive glowing rings over the composite.
    // sim coords → clip space matches the spawn handler's Y convention
    // (field row 0 is at the bottom, so sim y maps straight to clip y).
    // ------------------------------------------------------------------
    const epCount = drawMarkers ? Math.min(sim.endpoint_count(), MAX_MARKERS) : 0;
    if (epCount > 0) {
      const centers = new Float32Array(MAX_MARKERS * 2);
      const sizes = new Float32Array(MAX_MARKERS);
      // Diameter scale: cells → device px along x (canvas pixels per cell × 2
      // for the sprite to comfortably enclose the well, with a sane minimum).
      const pxPerCell = canvas.width / sim.width();
      for (let i = 0; i < epCount; i++) {
        const ex = sim.endpoint_x(i);
        const ey = sim.endpoint_y(i);
        const er = sim.endpoint_radius(i);
        centers[i * 2] = (ex / sim.width()) * 2.0 - 1.0;
        centers[i * 2 + 1] = (ey / sim.height()) * 2.0 - 1.0;
        sizes[i] = Math.max(14.0, er * pxPerCell * 2.4);
      }

      gl.useProgram(progMarker);
      gl.uniform2fv(ulMarker_centers, centers);
      gl.uniform1fv(ulMarker_sizes, sizes);
      gl.uniform3f(ulMarker_color, 1.0, 0.78, 0.32); // warm amber rings

      gl.enable(gl.BLEND);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE); // additive glow
      gl.drawArrays(gl.POINTS, 0, epCount);
      gl.disable(gl.BLEND);
    }

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
