# Petri Polis — Design

## Vision

A visually beautiful toy where **simple local rules produce complex global patterns** you can
watch and poke. The principle from the original spec still holds:

> Users create local causes. The world creates global patterns.

The first (and proven-beautiful) instance of that principle is **Physarum** — the slime-mold
trail algorithm (Jeff Jones 2010; Sage Jenson's "mold"). Agents deposit a trail, sense the
trail ahead, and steer toward it; the emergent result is self-organizing vascular networks.
Ecology (energy, food, death-feeds-world, regrowth) and additional species layer on top *after*
the trail core is mesmerizing.

## Architecture

```
[ crate: petri-core ]  pure Rust, native-testable
    field:  Vec<f32>            the trail map — the thing you see
    fieldB: Vec<f32>            double buffer for diffuse
    agents (SoA): x, y, heading: Vec<f32>     (+ energy/species/age later)
    rng:    seeded PRNG (xoshiro/PCG), explicit seed → deterministic
    tick(): per agent { sense 3 pts ahead → steer → move(wrap) → deposit }
            then diffuse (separable blur) + decay (field *= k); swap buffers
        |
[ crate: petri-wasm ]  thin wasm-bindgen layer
    Sim::new(w, h, seed, agent_capacity)
    Sim::tick()
    Sim::field_ptr() -> *const f32 ; field_len()      // zero-copy handle
    Sim::set_params(...)                               // LIVE, no rebuild
    Sim::spawn(x, y, n, pattern) ; reset(seed)
    Sim::agent_count() ; nearest_agent(x, y)          // inspector (later)
        |  JS: new Float32Array(wasm.memory.buffer, ptr, len)
[ TS app ]  WebGL2 + Tweakpane
    each frame: upload field → R32F texture
                → colormap (bioluminescent LUT)
                → bloom (bright-pass → separable gaussian → additive composite)
                → present (full-bleed canvas)
```

Single static-site deploy: fast Rust where the work is, GPU where the beauty is, no backend.

## The Physarum rule (implementable spec for petri-core)

Per tick, per agent (read the *current* field; deposits accumulate as you go):

1. **Sense** the trail field at three points at distance `sensor_distance` from the agent:
   center (`heading`), left (`heading − sensor_angle`), right (`heading + sensor_angle`).
   Nearest-sample is fine (cheaper than bilinear).
2. **Steer** (`F`,`L`,`R` = sensed values):
   - `F ≥ L` and `F ≥ R` → keep heading
   - `F < L` and `F < R` → turn `±rotation_angle` (random sign via rng)
   - `L > R` → turn left by `rotation_angle` ; `R > L` → turn right by `rotation_angle`
3. **Move:** `x += cos(heading)·step_size`, `y += sin(heading)·step_size`, wrap toroidally to
   `[0,W) × [0,H)`.
4. **Deposit:** `field[idx(x,y)] += deposit_amount`.

After all agents move, **field update:** separable blur (the glow/diffusion) into `fieldB`,
then `fieldB *= decay` (e.g. 0.9), then swap. Init: agents seeded uniformly or in a center
disk with random headings.

**Live params** (`set_params`, no rebuild): `sensor_angle`, `sensor_distance`,
`rotation_angle`, `step_size`, `deposit_amount`, `decay`, blur weight.

## Rendering (petri-render)

- Trail field is single-channel `f32` → upload as `R32F` (fallback `R16F`) texture each frame.
- **Colormap pass:** `t = clamp(intensity · gain)`; map `t` through a bioluminescent LUT
  (near-black → deep teal → cyan → white-hot core).
- **Bloom:** bright-pass threshold → separable gaussian blur (ping-pong FBOs) → additive
  composite over the base → present to a full-bleed canvas. Near-black background.
- Beauty bar: smooth gradients, soft additive glow, no hard pixel edges at the target zoom.

## Decisions log (ADR-lite)

**D1 — Rust→WASM sim in the browser (not pure TS, not a backend, not native).**
The render must reach the browser GPU every frame regardless. Rust→WASM gives native-ish CPU
speed with zero serialization (shared linear memory → zero-copy texture upload) and still ships
as one static site. *Rejected:* pure TS (great iteration but ~tens-of-thousands agent ceiling —
fine for a prototype, but the user wants a solid fast core); **Axum/backend** (a network
round-trip per frame is pure tax for a single-player local toy; solves multiplayer/persistence
we don't have); **Tauri** (IPC-per-frame + loses the shareable-URL story); **native wgpu** (no
URL sharing; only worth it for a desktop binary).

**D2 — Physarum trail core first; ecology and species layered after.**
It's the most reliably gorgeous "simple rules → complex dynamics" system and is one agent type
with ~3 params. *Rejected:* the spec's 5-species / 9-layer / 12-param big-bang → ~30
simultaneous knobs → muddy, illegible emergence and endless balancing.

**D3 — WebGL2 + bloom, bioluminescent palette.** The renderer is the product. *Rejected:*
Canvas2D "agents as dots" (low beauty ceiling; the field glow is where the wow lives).

**D4 — Determinism via a single seeded PRNG, not an event-sourcing system.**
`same seed + same wasm binary → identical run`. *Rejected:* event log + deterministic-replay +
versioned binary snapshots (product infrastructure, not a toy).

**D5 — Double-buffered fields + direct agent updates; no formal intent/conflict phase.**
Sidesteps most "conflict resolution." Genuine contention (e.g. two agents to one cell) is rare
and handled simply when it arises.

**D6 — Zero-copy field via shared linear memory; pre-allocate agent capacity.**
The JS `Float32Array` view aliases WASM memory, which relocates if it grows. Pre-allocating
capacity at `Sim::new` keeps steady-state ticks growth-free; re-fetch the view only after
`spawn`/`reset`.

**D7 — Two lean crates (`petri-core`, `petri-wasm`), not an 11-module workspace up front.**
Grow structure only when a real need appears.

## Deferred (not deleted)

Ecology (M3), multiple species (M4), inspector + shareable params/seed URL (M5), and scaling
via wasm-simd/threads or WebGPU compute (M6). See `ROADMAP.md`.
