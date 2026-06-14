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

## Later (see ROADMAP.md)
M6 scale (optional — only if scale is craved).
