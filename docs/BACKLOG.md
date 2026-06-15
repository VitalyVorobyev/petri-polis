# Petri Polis — Backlog

The next-task queue. Open this first each session, do the top unchecked item, tick it, update
`ROADMAP.md` when a milestone's gate is met. Keep it lean — checkboxes, not a tracker.

## Phase 0 — Scaffolding
- [x] Write lean-4 docs (CLAUDE.md, DESIGN.md, ROADMAP.md, BACKLOG.md)
- [x] Mark `docs/initial-spec.md` historical
- [x] Define `.claude/agents/petri-rust.md` (Opus) and `.claude/agents/petri-render.md` (Sonnet)
- [x] Save 2 project memories (reframe + cut-list)

## M0 — Toolchain round-trip ✅ (gate met: animated teal field renders at ~120 fps; tick advances via WASM)
- [x] `Cargo.toml` workspace with members `crates/petri-core`, `crates/petri-wasm`
- [x] `crates/petri-core`: `Sim` skeleton (new/tick/field accessor) + native unit tests (incl. determinism)
- [x] `crates/petri-wasm`: `cdylib` + wasm-bindgen wrapper exposing `field_ptr`/`field_len`/`tick`
- [x] `scripts/build-wasm.sh` → `wasm-pack build crates/petri-wasm --target web --out-dir app/src/wasm`
- [x] `app/`: Vite + TS scaffold, full-bleed canvas, WebGL2 context
- [x] `app/src/render/gl.ts`: minimal helpers (compile program, R32F field texture)
- [x] Smoke test: TS calls `Sim` (tick + zero-copy field) + bioluminescent colormap shader draws
- [x] `.gitignore`: `target/`, `node_modules/`, `app/src/wasm/`, `dist/`
- [x] Build/run commands verified in `CLAUDE.md`

## M1 — The hook (Physarum + render) ✅ gate met — glowing networks form; deterministic
- [x] petri-core: SoA agents (`x`,`y`,`heading`), seeded PRNG (xoshiro256**), `agent_capacity` (200k)
- [x] petri-core: sense (3-point) → steer (Jones rule) → move (toroidal) → deposit
- [x] petri-core: separable blur + decay field update; buffer swap; `field_max` for auto-exposure
- [x] petri-core: native test — fixed seed → golden FNV-1a checksum after 60 ticks (7/7 tests pass)
- [x] petri-wasm: `set_params`, `set_diffuse_weight`, `spawn(x,y,n,pattern)`, `reset(seed)`, param getters
- [x] app: upload field → R32F texture (zero-copy view; `makeFieldView` re-fetch helper)
- [x] app: tone-map pass — EMA auto-exposure + gamma 0.45 → bioluminescent LUT
- [x] app: bloom (bright-pass → 3× ping-pong gaussian @ half-res → additive composite)
- [x] app: RAF loop driving `tick()` + render; FPS counter
- [x] **Gate check:** glowing networks form in seconds; same seed → identical pattern

## M2 — Make it a toy (live controls) ✅ gate met — sliders change emergence live; a click spawns into the network
- [x] app: add Tweakpane; panel bound to all 7 live params (init from getters → `set_params`/`set_diffuse_weight`, no rebuild)
- [x] app: transport — play/pause, single-step, speed (1×/2×/5×/10× = N ticks per frame, one upload+render)
- [x] app: seed reset (`reset(seed)` + re-fetch zero-copy field view + reset auto-exposure EMA)
- [x] app: click-to-spawn — pointer → sim coords (Y-flip for the bottom-left field origin) → `spawn(x,y,count,pattern)` + re-fetch view
- [x] app: spawn-pattern select (point / ring / uniform) + spawn-count control
- [x] app: HUD shows dims · agents · tick · fps
- [x] **Gate check:** verified in-browser — tick advances; a click adds 500 agents (8192→8692); panel mounts; glowing networks render

## M3 — Ecology layer (boom/bust) ✅ gate met — population booms, depletes food, crashes, and recovers
- [x] petri-core: food field + static regrowing-patch `food_cap` (~16 soft Gaussian patches, deterministic from seed)
- [x] petri-core: per-agent `energy` SoA; eat food → gain, metabolism → drain
- [x] petri-core: reproduce (split energy, child at parent) via `push`; die at zero → return nutrient to food via `swap_remove`; all within capacity (zero-alloc, field view stays valid)
- [x] petri-core: food regrows toward per-cell cap; `food_max` tracked; 5 live ecology params
- [x] petri-core: determinism re-baselined (new golden checksum) + bounds + boom/bust-recovers tests (9/9 pass)
- [x] petri-wasm: `food_ptr`/`food_len`/`food_max`, `set_ecology`, + getters
- [x] app: food uploaded as 2nd R32F texture; dim olive substrate masked under the cyan trail (EMA-normalized)
- [x] app: Ecology Tweakpane folder (5 sliders → `set_ecology`)
- [x] **Gate check:** verified in-browser — population 8192 → ~44k boom → ~1.3k crash → rebound; food substrate visibly depletes/regrows; no errors

## M4 — Species (two, competing) ✅ gate met — cyan + magenta coexist into a pattern neither makes alone
- [x] petri-core: `SPECIES=2`; per-species trail field + `Params` + `Ecology`; agent `species` tag in SoA
- [x] petri-core: each agent senses/deposits ONLY its own trail; sole coupling is the shared food field (niche partitioning)
- [x] petri-core: two tuned param-sets (sp0 fine fast mesh / sp1 coarse thick veins) that coexist; determinism re-baselined; `two_species_coexist` test (10/10 pass)
- [x] petri-wasm: species-parameterized API (`field_ptr(s)`/`field_max(s)`/`set_params(s,…)`/`set_ecology(s,…)`/getters/`spawn(…,s)`), `species_count`, `species_population(s)`
- [x] app: two trail textures + per-species auto-exposure; cyan + magenta LUTs, additive → white overlap, over the food substrate
- [x] app: per-species control folders + spawn-species selector; HUD shows `cyan N · magenta N`
- [x] **Gate check:** verified in-browser — both boom (cyan ~20k / magenta ~26k) then coexist >10k ticks (neither excluded), interwoven two-color networks, no errors

## M5 — Inspect, measure & share ✅ gate met — metric export + shareable seed/params URL, verified
- [x] petri-core: read-only metric reductions (per-species trail mass, food total, food coverage) folded into existing passes — tick unperturbed, golden checksum unchanged
- [x] petri-core: inspector accessors (`trail_at`, `food_at`, `nearest_agent` + per-index agent getters); 12/12 tests pass
- [x] petri-wasm: metric + inspector accessors exposed
- [x] app: hand-rolled canvas sparklines (population, trail mass, food) sampled vs `tick_count` — no charting dependency (D8)
- [x] app: CSV/JSON export of the metric series (`tick,pop0,pop1,mass0,mass1,food_total,food_coverage`)
- [x] app: hover inspector readout (per-species trail + food + nearest agent species/energy/dist)
- [x] app: seed + per-species params encode into the URL hash; load restores the run
- [x] **Gate check:** verified in-browser — sparklines animate, hover readout works, CSV downloads with real rows, copied link round-trips on reload; no errors

## Docs & publishing ✅ gate met — guide + API docs + live toy publish to Pages on push to main
- [x] mdBook guide in `docs/guide/` — concepts & algorithms (Physarum rule, diffusion/decay, sim core, determinism, ecology, two species, parameter reference, render pipeline, zero-copy WASM)
- [x] `docs.yml` assembles one Pages site: `/` landing · `/app/` live toy · `/guide/` book · `/api/` rustdoc
- [x] `app` Pages build with the correct base path (`vite build --base=/petri-polis/app/`)
- [x] `ci.yml` builds the book so a broken guide fails PR CI
- [x] README/ROADMAP/DESIGN/CLAUDE synced to the shipped M2–M5 features

## M6 — World geography: sources, sinks & obstacles ✅ gate met — the maze network routes around the walls and bridges both endpoints; reachability flips to 2/2
- [x] petri-core: obstacle mask (`Vec<u8>`, allocated in `new`, never grows) — sensors read walls as 0; a move into a wall is blocked + reorients via one RNG draw *only when blocked*; trail held at 0 under walls in the field pass
- [x] petri-core: `food_attraction` `Params` knob (default `0.0`) — sense step adds `food_attraction * food_sample` per sensor, guarded so the zero default is byte-identical
- [x] petri-core: endpoint food sources (`add_endpoint`/`clear_endpoints`) pinning `food_cap` high within a radius, reusing the Gaussian food-cap machinery
- [x] petri-core: reachability / network-cost metric — threshold the combined trail field, flood-fill from endpoint 0 over open cells (pre-allocated scratch, on-demand, read-only); report `endpoints_connected` + `network_cost`
- [x] petri-core: `load_maze_demo` — clears default food, lays a three-wall serpentine maze + a sealed toroidal-seam wall (so the only route threads the gaps), places two endpoints, sets `food_attraction`, and blanket-seeds the colony across all open cells (pre-grown-mold coverage)
- [x] petri-core: walls fully masked from chemotaxis (wall cells read 0 trail AND 0 food in the sensor); painting a wall zeroes the trail under it immediately (no one-tick diffusion leak)
- [x] petri-core: tests — empty-geometry golden checksum UNCHANGED (byte-identical); re-pinned maze golden; reachability disconnected→connected + obstacle-split-stays-disconnected; `maze_seam_blocks_a_straight_bridge`; `maze_demo_connects_the_endpoints`; `maze_agents_never_enter_walls`; `food_attraction_zero_is_inert` (24/24 pass)
- [x] petri-wasm: `obstacle_ptr`/`obstacle_len` (stable), `paint_obstacle`/`clear_obstacles`, `add_endpoint`/`clear_endpoints` + endpoint accessors, `load_maze_demo`, `set_food_attraction`/getter, `set_network_threshold`/getter, `endpoints_connected`/`network_cost`
- [x] app: upload the obstacle mask as an `R8` texture; render walls as a dim slate underlay in the tone-map pass, suppressing trail/food/bloom there
- [x] app: endpoint markers (additive amber rings) at endpoint positions
- [x] app: pointer tool selector — spawn / paint wall / erase wall / place endpoint (click-drag painting) — + "load maze demo" and "clear geometry" buttons; obstacle view folded into the re-fetch path
- [x] app: reachability readout in the metrics panel (connected k/n + network cost, with a network-cost sparkline)
- [x] **Gate check:** verified (native `maze_demo_connects_the_endpoints` reaches `2/2`; in-browser `load_maze_demo` + 10× spans both wells, zero console errors). The colony connects via pre-grown coverage, then the food-starved interior thins and it settles at `1/2` — a *persistent, pruned* shortest-path tube needs an adaptive flux-based model, not tuning (follow-up below)
- [ ] follow-up (deferred milestone): a *persistent, pruned* shortest-path maze solve — the full "Physarum solves the maze" result. A diffusing long-range chemoattractant was tried and reverted: it yields either a transient connection or, with enough trail persistence to hold, a saturated blanket (~95% of open cells) — not a thin pruned tube. The real result needs an adaptive **flux-based tube model** (Tero-style: edges strengthen with throughput, unused edges decay), not tuning. Deliberate future work.

## M7 — Presets: the lab bench ✅ gate met — every gallery preset loads its canonical structure; scenarios round-trip as links
- [x] app: scenario model in TS (`presets.ts`) — seed + per-species params/ecology + chemotaxis + geometry + spawn; applied through the existing live setters (no new sim/wasm needed — the M6 boundary already exposes everything)
- [x] app: Presets Tweakpane folder + `applyScenario` (reset → setters → clear+rebuild geometry → re-fetch all views → guarded panel refresh); fixed a `pane.refresh()` bug that fired binding writes and perturbed the sim
- [x] app: starter gallery (coexistence, competitive exclusion, capillary mesh, trunk roads, spirals, boom/bust oscillator, maze via `load_maze_demo`, Tokyo rail via 9 city endpoints)
- [x] app: URL codec (`urlstate.ts`) carries the full scenario — params + ecology + `food_attraction` + `network_threshold` + the endpoint list + a built-in geometry tag (`maze`); hand-painted masks intentionally not serialized
- [x] **Gate check:** verified in-browser — all 8 presets apply with distinct canonical structures, zero console errors; Tokyo / maze / competitive-exclusion links round-trip exactly

## M8 — Cross-species sensing ✅ gate met — mutual avoidance segregates the species (5× less overlap); presets + links round-trip; deterministic
- [x] petri-core: a 2×2 signed sensing-weight matrix (`cross_sense[s][o]`) — sense becomes `Σ_o cross_sense[s][o]·field[o]`, composed with chemotaxis + wall masking; default **identity** keeps the run byte-identical (gated `cross_sense_active()`; empty golden `0x8de7…` unchanged)
- [x] petri-core: new cross-sense determinism golden + a segregation test (overlap 85 identity → 17 under mutual avoidance, same seed); 27/27 tests
- [x] petri-wasm: `set_cross_sense(species, other, weight)` / `cross_sense(species, other)`
- [x] app: **Cross-species sensing** Tweakpane folder (two off-diagonal sliders, guarded writes) + **Territories** (mutual avoidance) and **Predator/prey** (asymmetric pursuit) presets; scenario URL codec carries `crossSense`
- [x] **Gate check:** verified in-browser — Territories keeps the two colors to separate domains, Predator/prey shows magenta tracking cyan; segregation measured 5× in the sim; coupling links round-trip; zero console errors

## M9 — Structure metrics ✅ gate met — a live decay sweep collapses the component count (14→4→2) while grain & fractal dimension grow: a measured phase transition
- [x] petri-core: read-only, on-demand reductions over the thresholded foreground — connected components (+ a per-cell label buffer), independent loops (`b1 = E−V+C`), box-counting fractal dimension, autocorrelation grain length; one union-find pass yields components + labels + E/V; pre-allocated scratch, no tick change → **all goldens unchanged** (32/32 tests)
- [x] petri-wasm: `component_count`/`loop_count`/`fractal_dimension`/`autocorrelation_length` + `component_labels_ptr`/`component_labels_len`
- [x] app: component-count sparkline row + loops/D/grain readout + four new CSV/JSON columns (structure metrics sampled on a throttled 20-frame cadence to protect fps); a **Render mode** selector adds a **component map** (color by label) and **long-exposure** (time-integrated) overlay
- [x] **Gate check:** verified in-browser — driving `decay` 0.75→0.985 collapses components 14→4 (then 2), autocorr 5.8→28.8 cells, fractal D 1.40→1.66; component-map overlay colors components to match the count; long-exposure integrates; zero console errors
- [ ] follow-up (deferred): skeleton-based trail length & branching (node degrees), and a Lyapunov-style seed-perturbation divergence (needs a cloneable twin `Sim`) — both read-only additions in the same vein

## M10 — Headless parameter sweep ✅ gate met — a reproducible `decay` sweep collapses the component count ~17→~2, located in a CSV + SVG phase diagram
- [x] petri-core: `src/bin/sweep.rs` — native, dependency-free (std only); varies 1–2 knobs × N values × M seeds headless, reads the M9 `component_count` order parameter (+ trail mass + coexistence), aggregates mean/std across seeds, emits CSV
- [x] petri-core: hand-rolled SVG figure (no plotting crate) — 1-D line plot with ±std error bars (viridis heatmap in 2-D mode); `--knob`/`--grid`/`--knob2` flags
- [x] petri-core: `std::thread::scope` parallelism over the independent runs (default 20×6×3000 ticks at 256² in ~21 s, ~11 cores); results scatter back by job index so the output is thread-count-independent and byte-identical per config
- [x] **Gate check:** the default `decay` sweep collapses components ~17→~2 at the consolidation threshold; CSV + SVG reproducible byte-for-byte; goldens unchanged (32 tests). Figure embedded in the guide

## M11 — Evolution: heritable traits ✅ gate met — the trait distribution drifts (µ 5.7→9.3 cells live; 7.0→4.94 in the native test) and replays identically from the seed
- [x] petri-core: per-agent heritable `sensor_distance` (SoA); on reproduction the child inherits the parent's value + a seeded-RNG Gaussian mutation (`mutation_strength`, clamped [1,32]); fixed RNG order; gated `evolution_active` so default-off is byte-identical (all baseline goldens unchanged)
- [x] petri-core: trait-distribution metrics (`trait_mean`/`std`/`min`/`max`, `agent_trait`) + a gated `trait_field` for the trait-map view; new evolution determinism golden + a drift+reproducibility test (38 tests)
- [x] petri-wasm: `set_evolution`/`evolution_enabled`, `set_mutation_strength`/`mutation_strength`, trait getters, `trait_field_ptr`/`len`
- [x] app: per-species Evolution controls (enable + mutation), a trait sparkline (mean ±σ) + readout + CSV columns, a **Trait map** render mode (short→long reach gradient), an **Evolution** preset; scenario codec carries the evolution state
- [x] **Gate check:** verified in-browser — loading the Evolution preset drifts `trait µ 5.7 ± 4.3 → 9.3 ± 13.1 cells` over a run (evolving cyan outcompetes the non-evolving magenta control), the trait map colors strategies spatially, zero console errors; reset + same seed reproduces the trajectory (native test)

## Deferred follow-ups (post-arc, not yet scheduled)
- [ ] persistent, pruned shortest-path maze solve — needs an adaptive flux-based tube model (Tero-style), not tuning (see M6 notes)
- [ ] skeleton-based trail length & branching, and a Lyapunov-style seed-perturbation divergence (needs a cloneable twin `Sim`) — read-only additions in the M9 vein
