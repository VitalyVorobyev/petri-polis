# Petri Polis — Design

This is the architecture-and-decisions reference. For a narrative walkthrough of the concepts and
algorithms, see **the guide** in [`guide/`](guide/) (published at
`https://vitalyvorobyev.github.io/petri-polis/guide/`); this document is where the *decisions* and
their rejected alternatives live.

## Vision

A visually beautiful toy where **simple local rules produce complex global patterns** you can
watch and poke. The guiding principle:

> Users create local causes. The world creates global patterns.

The first (and proven-beautiful) instance of that principle is **Physarum** — the slime-mold
trail algorithm (Jeff Jones 2010; Sage Jenson's "mold"). Agents deposit a trail, sense the
trail ahead, and steer toward it; the emergent result is self-organizing vascular networks.
Ecology — per-agent energy, a regrowing food field, reproduction, and death-feeds-the-world —
layers on the trail core to produce boom/bust cycles. Two species share that food field, each
weaving its own trail network in its own color, so they compete into a combined picture neither
makes alone.

## Architecture

```
[ crate: petri-core ]  pure Rust, native-testable
    field:  [Vec<f32>; SPECIES] one trail map per species — the thing you see
    fieldB: Vec<f32>            shared blur scratch, reused per species
    food:   Vec<f32>            SHARED nutrient field; foodCap = static regrowing patches
    agents (SoA): x, y, heading, energy: Vec<f32> + species: Vec<u8>
    params/ecology: per-species; rng: seeded PRNG (xoshiro256**) → deterministic
    tick(): per agent (by species) { sense/deposit OWN trail → steer → move(wrap)
                        → metabolism → eat SHARED food → reproduce / die-feeds-food }
            then per species: diffuse (separable blur) + decay; food regrows to cap
        |
[ crate: petri-wasm ]  thin wasm-bindgen layer
    Sim::new(w, h, seed)
    Sim::tick()
    Sim::field_ptr(species) ; field_len() ; field_max(species)   // zero-copy per-species trail
    Sim::food_ptr() ; food_len() ; food_max()          // zero-copy shared food
    Sim::set_params(species, ...) ; set_ecology(species, ...)    // LIVE, no rebuild
    Sim::spawn(x, y, n, pattern, species) ; reset(seed)
    Sim::agent_count() ; species_population(species)   // dynamic — rises/falls with the cycle
        |  JS: new Float32Array(wasm.memory.buffer, ptr, len)
[ TS app ]  WebGL2 + Tweakpane
    each frame: upload each species' trail + food → R32F textures
                → colormap: cyan (sp0) + magenta (sp1), additive → white where they overlap,
                  over a dim food substrate; per-species auto-exposure
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

## The ecology rule (implementable spec for petri-core)

A nutrient `food` field rides alongside the trail. `food_cap` is a static map of soft Gaussian
patches (regrowing nutrient sources); `food` starts at the cap and is consumed by agents.

Per tick, after an agent senses/steers/moves/deposits:
1. **Metabolism:** `energy -= metabolism` (cost of living).
2. **Eat:** consume up to `eat_rate` of the food under the agent — `eaten = min(food[cell],
   eat_rate)`; `food[cell] -= eaten`; `energy += eaten`.
3. **Reproduce:** if `energy ≥ repro_threshold` (and below the agent-capacity cap), halve energy
   and spawn a child at the parent with a jittered heading.
4. **Die-feeds-the-world:** if `energy ≤ 0`, return `death_return` nutrient to `food` at the
   cell and remove the agent.

After the agent pass, each cell regrows toward its local ceiling: `food += food_regrow ·
(food_cap − food)`. Patchy food plus survivors in rich patches give spatially-staggered
boom/bust that recovers rather than going extinct.

**Live params** (`set_ecology`, no rebuild): `metabolism`, `eat_rate`, `repro_threshold`,
`food_regrow`, `death_return`.

## Two species (implementable spec for petri-core)

`SPECIES = 2`. Each species owns a trail field, a `Params` set, and an `Ecology` set; every
agent carries a `species` tag. An agent senses and deposits **only its own species' trail**, so
each species self-organizes its own network. The **only** coupling is the shared `food` field —
both eat from the same regrowing patches, competing spatially at their network boundaries. The
two default sets are tuned to different niches (species 0 a fine, fast mesh; species 1 coarse,
thick veins) so they coexist instead of one excluding the other. Per-species live params via
`set_params(species, …)` / `set_ecology(species, …)`.

## Rendering (petri-render)

- Each species' trail is single-channel `f32` → upload as an `R32F` texture each frame.
- **Colormap pass:** per species `t = clamp(intensity · gain)` with a per-species auto-exposure
  gain; map species 0 through a cyan LUT (near-black → teal → cyan → white) and species 1
  through a magenta LUT, then add them so overlap blends toward white.
- **Bloom:** bright-pass threshold → separable gaussian blur (ping-pong FBOs) → additive
  composite over the base → present to a full-bleed canvas. Near-black background.
- **Food substrate:** the food field uploads as a second `R32F` texture and renders as a dim,
  desaturated underlay beneath the cyan trail — depletion darkens it, regrowth re-greens it, so
  the boom/bust cycle is legible. Kept below the bloom threshold so only the trail glows.
- Beauty bar: smooth gradients, soft additive glow, no hard pixel edges at the target zoom.

## Measurement & sharing (petri-render + petri-wasm)

The toy doubles as an instrument. Read-only sim accessors expose per-species **trail mass**,
**food total**, and **food coverage** (folded into the existing field/food passes, so the tick
is unperturbed and determinism holds), plus point queries (`trail_at`, `food_at`,
`nearest_agent` + per-index agent getters). On top of them:

- **Live metrics** — sparklines (hand-rolled on a 2D canvas, no charting dependency) of
  per-species population and trail mass, food total and coverage, **sampled against
  `tick_count`** (not wall-clock) so a series is reproducible across machines.
- **Export** — the recorded series downloads as CSV/JSON for offline analysis.
- **Inspector** — hovering reads the trail/food under the cursor and the nearest agent's
  species and energy.
- **Sharing** — the seed and every per-species parameter encode into the URL hash; opening the
  link restores the run.

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

**D8 — Control/inspection UI is pure TS + Tweakpane, not React.** The product is a single
full-bleed canvas driven by a per-frame WebGL/WASM loop over a mutable typed-array view of WASM
memory — there is no component tree, routing, or data-driven DOM for React's reconciliation to
help with, and React would sit beside the hot loop, never inside it. Tweakpane is purpose-built
for parameter panels and monitors and is framework-agnostic. *Revisit if* the inspector/metrics
dashboard grows into a component-heavy, multi-view UI (plots, tabs, preset gallery) — then a
lightweight reactive layer (Preact/Solid) for the panels only becomes worth its weight; the
render loop stays vanilla regardless.

**D9 — Minimal boom/bust ecology over a patchy food field (not a rich lifecycle).**
Five live knobs (`metabolism`, `eat_rate`, `repro_threshold`, `food_regrow`, `death_return`) on
top of a regrowing-patch food map are enough for the gate dynamic: grow → deplete → collapse →
recover. *Rejected:* age/senescence, an explicit carrying-capacity term, and multiple food types
up front — more knobs than the emergence needs, and the agent-capacity cap already bounds the
population. Patchy food (not uniform) is chosen so survivors in rich patches drive recovery
instead of global extinction, and the cycle is spatially staggered — more legible and more
beautiful.

**D10 — Two species coupled only through shared food (own-trail niche partitioning).**
Each species senses/deposits only its own trail field and differs by parameter-set + color; the
sole interaction is competition for the one shared food field. Because each follows its own
trail, the species self-segregate into interwoven networks → robust coexistence and a legible
two-color picture. *Rejected:* a Forager→Seeder→Builder role-transition pipeline (a new
lifecycle mechanic, harder to keep deterministic and legible) and direct cross-species trail
sensing (predator/prey) — both add coupling the gate doesn't need yet. Start at two; the
`SPECIES` constant generalizes when a third earns its place.

## Out of current scope (not deleted)

Scaling via wasm-simd/threads or WebGPU compute (only if scale is craved). See `ROADMAP.md`.
