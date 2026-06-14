# Petri Polis — Backlog

The next-task queue. Open this first each session, do the top unchecked item, tick it, update
`ROADMAP.md` when a milestone's gate is met. Keep it lean — checkboxes, not a tracker.

## Phase 0 — Scaffolding
- [x] Write lean-4 docs (CLAUDE.md, DESIGN.md, ROADMAP.md, BACKLOG.md)
- [x] Mark `docs/initial-spec.md` historical
- [ ] Define `.claude/agents/petri-rust.md` (Opus) and `.claude/agents/petri-render.md` (Sonnet)
- [ ] Save 2 project memories (reframe + cut-list)

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

> Note for M2: the default regime is a few thick veins (decay=0.9 is aggressive). Density/
> lushness is now a live-tuning question — exactly what M2's Tweakpane controls unlock.

## Later (see ROADMAP.md)
M2 Tweakpane/controls · M3 ecology · M4 species · M5 inspect+share · M6 scale.
