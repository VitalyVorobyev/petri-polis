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
    food:   Vec<f32>            SHARED nutrient field; foodCap = patches + endpoint wells
    obstacles: Vec<u8>          0/1 wall mask the trail can't enter (default all-0)
    endpoints: Vec<Endpoint>    persistent food wells to connect (sources/sinks)
    visited/queue: Vec<u32>     pre-allocated scratch for the on-demand reachability BFS
    agents (SoA): x, y, heading, energy: Vec<f32> + species: Vec<u8>
    params/ecology: per-species; rng: seeded PRNG (xoshiro256**) → deterministic
    tick(): per agent (by species) { sense OWN trail (+ food·food_attraction; walls→0)
                        → steer → move(wrap; refused by walls) → deposit
                        → metabolism → eat SHARED food → reproduce / die-feeds-food }
            then per species: diffuse (separable blur) + decay (held 0 under walls);
                              food regrows to cap
        |
[ crate: petri-wasm ]  thin wasm-bindgen layer
    Sim::new(w, h, seed)
    Sim::tick()
    Sim::field_ptr(species) ; field_len() ; field_max(species)   // zero-copy per-species trail
    Sim::food_ptr() ; food_len() ; food_max()          // zero-copy shared food
    Sim::set_params(species, ...) ; set_ecology(species, ...)    // LIVE, no rebuild
    Sim::spawn(x, y, n, pattern, species) ; reset(seed)
    Sim::agent_count() ; species_population(species)   // dynamic — rises/falls with the cycle
    Sim::obstacle_ptr() ; obstacle_len()               // zero-copy 0/1 wall mask (u8)
    Sim::paint_obstacle(x,y,r,on) ; clear_obstacles()  // edit walls in place
    Sim::add_endpoint(x,y,r) ; clear_endpoints() ; endpoint_count()/_x/_y/_radius
    Sim::load_maze_demo()                              // reset-class built-in scenario
    Sim::set_food_attraction(species,w) ; set_network_threshold(t)
    Sim::endpoints_connected() ; network_cost()        // on-demand reachability read
        |  JS: new Float32Array(buffer, ptr, len) for fields; Uint8Array(...) for the mask
[ TS app ]  WebGL2 + Tweakpane
    each frame: upload each species' trail + food → R32F, obstacle mask → R8
                → walls render as a dim slate underlay (below the bloom threshold, no glow)
                → colormap: cyan (sp0) + magenta (sp1), additive → white where they overlap,
                  over a dim food substrate; per-species auto-exposure
                → bloom (bright-pass → separable gaussian → additive composite)
                → additive amber rings mark the endpoints → present (full-bleed canvas)
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
agent carries a `species` tag. An agent deposits **only into its own species' trail**, and by
default senses only its own trail, so each species self-organizes its own network. By default the
**only** coupling is the shared `food` field — both eat from the same regrowing patches,
competing spatially at their network boundaries. The two default sets are tuned to different
niches (species 0 a fine, fast mesh; species 1 coarse, thick veins) so they coexist instead of
one excluding the other. Per-species live params via `set_params(species, …)` /
`set_ecology(species, …)`. An optional signed **2×2 cross-sensing matrix** (default identity)
lets a species also read the other's trail — positive weight to chase it, negative to avoid it —
which adds territories and predator/prey on top of the shared-food coupling.

## World geography (sources, sinks & obstacles)

The toroidal arena gains optional structure. An `obstacles: Vec<u8>` mask (0 open / 1 wall,
allocated once and never grown) makes cells impassable: a sensor over a wall reads 0, a move that
would enter a wall is refused (the agent stays and turns once), and the diffuse/decay pass holds
wall cells at 0 so the glow can't seep through. `endpoints` are persistent food wells —
`add_endpoint` bakes a high, flat-topped source into `food_cap` (combined with the random patches
by `max`), so a well stays fed however hard it is grazed. A per-species `food_attraction` adds the
food under each sensor to the trail it reads (`sensed = trail + food_attraction · food`), turning
the steer-toward-the-strongest rule into chemotaxis up the food gradient — the mechanism that lets
the colony find and connect the wells. `load_maze_demo` assembles a reproducible serpentine-wall
scenario with two wells and chemotaxis on.

All of it is additive: with no walls, no endpoints, and `food_attraction = 0` (the defaults) the
new branches are never entered — no extra PRNG draw, no food term — so a run is bit-for-bit
identical to one without geography and the golden-checksum test is unchanged.

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
- **Geography:** the obstacle mask uploads as an `R8` texture; wall cells render as a dim slate
  underlay kept below the bloom threshold (no glow), and each endpoint draws an additive amber
  ring so the sources/sinks read at a glance.
- **Render modes:** beyond the default tone-map, a *component map* (each connected component a
  distinct hue from the labels buffer, an `R32UI` texture) and *long-exposure* (a time-integrated
  `RGBA16F` accumulator showing the network's history) turn the renderer into a measurement
  display. Default is the tone-map; the others are opt-in.
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
- **Reachability** — on demand (never in `tick`), the combined trail field is thresholded at
  `network_threshold` and flood-filled (4-connected, toroidal) from endpoint 0 over pre-allocated
  scratch; `endpoints_connected` (how many wells the network spans) and `network_cost` (cells in
  the connecting structure) read out whether — and how cheaply — the maze is solved.
- **Structure** — the same on-demand, read-only pattern quantifies *form*, not just quantity,
  over the thresholded foreground: connected components (with a per-cell label buffer feeding the
  component-map render mode), independent loops (the grid graph's first Betti number
  `b1 = E − V + C`), box-counting fractal dimension, and the autocorrelation grain length. A slow
  `decay` sweep collapses the component count and grows the grain — a measured phase transition.
- **Inspector** — hovering reads the trail/food under the cursor and the nearest agent's
  species and energy.
- **Sharing & presets** — a named **preset gallery** loads classical scenarios (capillary mesh,
  trunk roads, spirals, boom/bust, competitive exclusion, coexistence, the maze, Tokyo rail) in
  one click. The seed, both species' parameters and ecology, the chemotaxis and reachability
  knobs, the endpoint wells, and a tag for any built-in procedural geometry encode into the URL
  hash, so a scenario — preset or hand-tuned — restores from a single link.

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

**D10 — Two species, coupled through shared food by default (own-trail niche partitioning).**
Each species deposits only into its own trail field and differs by parameter-set + color; by
default the interaction is competition for the one shared food field. Because each follows its
own trail, the species self-segregate into interwoven networks → robust coexistence and a legible
two-color picture. *Rejected:* a Forager→Seeder→Builder role-transition pipeline (a new lifecycle
mechanic, harder to keep deterministic and legible). Start at two; the `SPECIES` constant
generalizes when a third earns its place. (Direct cross-species trail sensing, once out of scope,
is now an opt-in matrix — see D15.)

**D15 — Cross-species coupling is an opt-in signed sensing matrix, off by default.**
Each species can read the other's trail through a 2×2 weight matrix (positive = attract toward
it, negative = avoid it); the default is the identity matrix, so an out-of-the-box run is
byte-identical and the only coupling is still shared food. A few lines of weighted-sum arithmetic
in the sense step buy territories (mutual avoidance) and predator/prey (asymmetric pursuit), and
the effect is measurable (trail overlap drops sharply under mutual avoidance). *Rejected:*
hard-wiring a predator/prey rule (the signed matrix subsumes it and stays tunable); making
cross-sensing always-on (it would change the default dynamics and tax every sense for a feature
most runs leave off).

**D11 — World geography is additive and inert by default (obstacles + chemotaxis).** A `u8` wall
mask (held at 0 under walls, sensed as 0, refusing moves) plus a per-species `food_attraction`
chemotaxis term give the colony walls to route around and food wells to seek — the ingredients of
the Physarum maze demo. Every new path is gated on geometry/attraction actually being present, so
the empty-world run stays bit-for-bit identical and the golden checksum is unchanged. *Rejected:* a
separate repulsion field for walls (a second full field for a boolean fact); always-on food sensing
(shifts the baseline and taxes every tick for a feature most runs don't use).

**D12 — Endpoints are persistent food wells baked into the food cap, not a new injection path.**
`add_endpoint` raises `food_cap` (a flat core + Gaussian skirt, combined by `max`) and the existing
regrow-toward-cap rule keeps the well full, so a source needs no new per-tick code and stays
deterministic. *Rejected:* a separate constant-injection term per endpoint.

**D13 — Reachability is an on-demand, read-only BFS over pre-allocated scratch — not a tick cost.**
A thresholded flood-fill (4-connected, toroidal) from endpoint 0 answers "are the wells connected,
and how large is the connecting network" only when asked — the renderer samples it at the sparkline
cadence — so the hot loop and the no-alloc/determinism invariants are untouched. One reachability
number reads out the maze demo; richer topology metrics are not computed in the tick. *Rejected:*
folding a graph search into every `tick` (heavy, and the answer is needed at UI cadence, not tick
cadence).

**D14 — Presets are in-code TS scenario data applied through the live setters; the share codec
carries geometry by endpoint-list + a built-in tag, not raw masks.** The whole gallery is plain
TS data plus a single `applyScenario` (reset → setters → rebuild geometry → re-fetch views), so
no preset machinery leaks into the sim core — the geometry boundary already exposes everything a
scenario needs. The URL hash encodes the endpoint list and a compact tag for procedural walls
(the maze regenerates from its tag) rather than the full obstacle mask, keeping links short;
hand-painted masks are deliberately not shareable. *Rejected:* a Rust-side scenario format and a
second (de)serialization path across the wasm boundary (the app already owns the parameter
surface and the URL codec); packing the raw mask into the URL (too large).

**D16 — Structure metrics are read-only on-demand reductions, like reachability; no tick cost.**
Components, loops, fractal dimension, and grain length are computed only when sampled (the
renderer throttles them off the per-frame budget), over pre-allocated scratch, never inside
`tick` — so the hot loop, the no-alloc invariant, and every golden checksum are untouched. One
union-find pass yields the components, the per-cell labels, and the edge/vertex counts for the
loop Betti number. *Rejected:* folding them into the tick (heavy, and they're wanted at UI
cadence, not tick cadence); a sensitivity (Lyapunov) divergence for now — it needs a cloneable
twin `Sim` and is tracked as a follow-up.

## Out of current scope (not deleted)

Scaling via wasm-simd/threads or WebGPU compute (only if scale is craved). See `ROADMAP.md`.
