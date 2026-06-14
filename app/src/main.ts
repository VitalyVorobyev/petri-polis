// Petri Polis — M1 entry point.
// Pipeline: Rust Physarum sim → (zero-copy) → R32F texture
//           → tone-map pass (EMA auto-exposure + gamma) → FBO
//           → bloom bright-pass → ping-pong Gaussian blur → additive composite
//           → present to canvas.

import {
  type BloomFBO,
  compileProgram,
  createBloomFBO,
  createFieldTexture,
  createGL,
  uploadField,
} from "./render/gl";
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
// Pass 1 — tone-map: EMA-stabilised gain + gamma → bioluminescent palette.
// Result written to an RGBA16F FBO (the "base" image for the bloom composite).
// ---------------------------------------------------------------------------
const FRAG_TONEMAP = `#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_field;
uniform float u_gain;
out vec4 o;

vec3 palette(float t) {
  t = clamp(t, 0.0, 1.0);
  vec3 c0 = vec3(0.01, 0.02, 0.05);  // near-black blue
  vec3 c1 = vec3(0.00, 0.30, 0.42);  // deep teal
  vec3 c2 = vec3(0.05, 0.80, 0.92);  // cyan
  vec3 c3 = vec3(0.90, 1.00, 1.00);  // white-hot
  if (t < 0.35) return mix(c0, c1, t / 0.35);
  if (t < 0.70) return mix(c1, c2, (t - 0.35) / 0.35);
  return mix(c2, c3, (t - 0.70) / 0.30);
}

void main() {
  float raw = texture(u_field, v_uv).r;
  // Soft-knee exposure: lift mid-tones without blowing the peaks.
  float v = raw * u_gain;
  // Gamma lift — sqrt-ish curve that opens up the dim filaments.
  float t = pow(clamp(v, 0.0, 1.0), 0.45);
  o = vec4(palette(t), 1.0);
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

/**
 * Return a fresh Float32Array view over the WASM trail field.
 * Call after spawn() or reset() as a defensive measure (memory could in theory
 * be relocated, though in practice petri-wasm pre-allocates at construction).
 */
function makeFieldView(wasm: { memory: WebAssembly.Memory }, sim: Sim): Float32Array {
  return new Float32Array(wasm.memory.buffer, sim.field_ptr(), sim.field_len());
}

async function main(): Promise<void> {
  const wasm = await init({ module_or_path: wasmUrl });
  const sim = new Sim(WIDTH, HEIGHT, 1);

  // Zero-copy view over the trail field in WASM linear memory.
  // Re-fetch with makeFieldView() after any spawn/reset call.
  const field = makeFieldView(wasm, sim);

  const canvas = document.getElementById("c") as HTMLCanvasElement;
  const hud = document.getElementById("hud") as HTMLDivElement;
  const gl = createGL(canvas);

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
  // Trail field texture (R32F, updated each frame)
  // ---------------------------------------------------------------------------
  const fieldTex = createFieldTexture(gl, WIDTH, HEIGHT);

  // ---------------------------------------------------------------------------
  // Off-screen FBOs
  // ---------------------------------------------------------------------------
  // baseFBO  — full-res tone-mapped image (input to bloom and final composite)
  // pingFBO / pongFBO — half-res, ping-pong for the Gaussian blur passes
  let baseFBO: BloomFBO;
  let pingFBO: BloomFBO;
  let pongFBO: BloomFBO;

  function allocFBOs(): void {
    baseFBO = createBloomFBO(gl, canvas.width, canvas.height);
    const bw = Math.max(1, Math.floor(canvas.width * BLOOM_SCALE));
    const bh = Math.max(1, Math.floor(canvas.height * BLOOM_SCALE));
    pingFBO = createBloomFBO(gl, bw, bh);
    pongFBO = createBloomFBO(gl, bw, bh);
  }

  function resize(): void {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.floor(window.innerWidth * dpr);
    canvas.height = Math.floor(window.innerHeight * dpr);
    gl.viewport(0, 0, canvas.width, canvas.height);
    allocFBOs();
  }
  window.addEventListener("resize", resize);
  resize();

  // ---------------------------------------------------------------------------
  // Uniform locations
  // ---------------------------------------------------------------------------
  const ulTonemap_field = gl.getUniformLocation(progTonemap, "u_field");
  const ulTonemap_gain = gl.getUniformLocation(progTonemap, "u_gain");
  const ulBright_src = gl.getUniformLocation(progBright, "u_src");
  const ulBright_thresh = gl.getUniformLocation(progBright, "u_threshold");
  const ulBlur_src = gl.getUniformLocation(progBlur, "u_src");
  const ulBlur_dir = gl.getUniformLocation(progBlur, "u_dir");
  const ulComp_base = gl.getUniformLocation(progComposite, "u_base");
  const ulComp_bloom = gl.getUniformLocation(progComposite, "u_bloom");
  const ulComp_strength = gl.getUniformLocation(progComposite, "u_bloom_strength");

  // ---------------------------------------------------------------------------
  // EMA reference for auto-exposure
  // ---------------------------------------------------------------------------
  let emaRef = 1.0;

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
    sim.tick();
    uploadField(gl, fieldTex, WIDTH, HEIGHT, field);

    // EMA-stabilised auto-exposure — avoids flicker from frame-to-frame max spikes.
    const rawMax = Math.max(1e-3, sim.field_max());
    emaRef = emaRef + EMA_ALPHA * (rawMax - emaRef);
    const gain = GAIN_TARGET / emaRef;

    gl.bindVertexArray(vao);

    // ------------------------------------------------------------------
    // Pass 1 — Tone-map → baseFBO (full resolution)
    // ------------------------------------------------------------------
    gl.bindFramebuffer(gl.FRAMEBUFFER, baseFBO.fbo);
    gl.viewport(0, 0, baseFBO.width, baseFBO.height);
    gl.useProgram(progTonemap);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, fieldTex);
    gl.uniform1i(ulTonemap_field, 0);
    gl.uniform1f(ulTonemap_gain, gain);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // ------------------------------------------------------------------
    // Pass 2 — Bright-pass → pingFBO (half resolution)
    // ------------------------------------------------------------------
    gl.bindFramebuffer(gl.FRAMEBUFFER, pingFBO.fbo);
    gl.viewport(0, 0, pingFBO.width, pingFBO.height);
    gl.useProgram(progBright);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, baseFBO.tex);
    gl.uniform1i(ulBright_src, 0);
    gl.uniform1f(ulBright_thresh, BLOOM_THRESHOLD);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // ------------------------------------------------------------------
    // Pass 3 — Separable Gaussian blur (BLUR_PASSES × horizontal+vertical)
    // pingFBO → pongFBO → pingFBO → ...  final result in pingFBO
    // ------------------------------------------------------------------
    const invW = 1.0 / pingFBO.width;
    const invH = 1.0 / pingFBO.height;
    let srcFBO = pingFBO;
    let dstFBO = pongFBO;

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
    gl.bindTexture(gl.TEXTURE_2D, baseFBO.tex);
    gl.uniform1i(ulComp_base, 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, bloomTex);
    gl.uniform1i(ulComp_bloom, 1);
    gl.uniform1f(ulComp_strength, BLOOM_STRENGTH);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    // ------------------------------------------------------------------
    // HUD
    // ------------------------------------------------------------------
    frames++;
    if (now - last >= 500) {
      fps = Math.round((frames * 1000) / (now - last));
      frames = 0;
      last = now;
      hud.textContent = `Petri Polis · M1 · ${WIDTH}×${HEIGHT} · tick ${sim.tick_count()} · ${fps} fps`;
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
