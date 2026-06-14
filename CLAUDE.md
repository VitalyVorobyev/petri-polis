# Petri Polis — agent guide

A **visually beautiful** browser toy for playing with **complex dynamics that emerge from a
few simple rules**. Core = Physarum (slime-mold) trail agents, rendered as glowing
bioluminescent networks. Fast **Rust→WASM** simulation, **WebGL2** rendering, single static
site. No backend.

> **Read `docs/DESIGN.md` for the architecture and the decisions log, `docs/ROADMAP.md` for
> the milestone arc, and `docs/BACKLOG.md` for the next concrete task.** The user-facing guide
> (concepts & algorithms) is the mdBook in `docs/guide/`. `docs/initial-spec.md` is historical —
> superseded by DESIGN.md.

## Build / run / test

> Frontend uses **bun** (not npm). Verified working with these versions (Rust 1.95, wasm-pack
> 0.15, wasm-bindgen 0.2.125, bun 1.3, Vite 8, TypeScript 6, mdBook 0.4).

```bash
# Build the Rust sim → WASM into app/src/wasm/
bash scripts/build-wasm.sh

# Run the dev frontend (Vite) — frontend uses bun, run from app/
cd app && bun install         # first time
bun run dev

# Native Rust tests (sim core is testable WITHOUT the browser)
cargo test -p petri-core

# Build/serve the guide (mdBook) — concepts & algorithms
mdbook serve docs/guide       # live at http://localhost:3000
mdbook build docs/guide       # one-off → docs/guide/book/

# Production build
bash scripts/build-wasm.sh --release && (cd app && bun run build)
```

Rust→wasm rebuilds only matter when you change *rules/structure*. Tuning **runtime knobs**
(sensor angle, deposit, decay, step) goes through `Sim::set_params` and needs **no rebuild**.

## Conventions

- **Rust sim hot loop:** struct-of-arrays (`Vec<f32>` per agent field), no per-agent objects,
  no allocation inside `tick()`. Pre-allocate agent capacity at `Sim::new`.
- **Determinism is load-bearing:** all randomness comes from a single seeded PRNG owned by the
  sim. Same seed + same wasm binary → identical run. No wall-clock in sim logic. Native test
  asserts a field checksum after N ticks for a fixed seed.
- **Zero-copy caveat:** the JS `Float32Array` view aliases WASM linear memory, which *moves if
  memory grows*. Only `spawn`/`reset` may grow it → re-fetch the view after those calls, never
  inside the per-frame loop.
- **TS render:** the renderer is the product. Field → R32F texture → colormap (bioluminescent
  LUT) → bloom → present. Keep passes small and readable; beauty before features.
- **Docs describe the present, not the process:** reference docs (`DESIGN.md`), code comments,
  and runtime/UI strings describe the system **as it is now** — no milestone labels (`M1`,
  `M2`), task IDs, "deferred to…", "(later)", or historical references ("superseded by",
  "initially…", "used to"). Milestone/task sequencing lives only in `ROADMAP.md` and
  `BACKLOG.md`; those two files are the only sanctioned home for it.

## Guardrails — the cut-list (do NOT reintroduce without an explicit decision)

- **No backend / no Axum / no server.** Single-player, local, static site. A network
  round-trip per frame is pure tax.
- **No Tauri / native shell** unless we deliberately want a desktop binary (we don't yet).
- **No formal intent-objects + conflict-resolution phase.** Use double-buffered fields and
  direct agent updates in the Rust tick.
- **No event log / deterministic-replay system / binary snapshots / MessagePack.** Determinism
  comes from the seeded PRNG; `same seed → same world` reset is enough.
- **No 13-metric history + plots** until much later (M5+).
- **Don't add intelligence (RL/LLM/learned behavior) before the ecology is interesting.**
- **Don't front-load 5 species / 9 layers / 12 params.** One agent type + one trail field
  first. Add knobs only when the current ones are mastered.

## Workflow (multi-session)

The orchestrator (this session) reads `docs/BACKLOG.md`, picks the next task, and delegates to
an implementation agent so orchestrator context stays lean:

- **`petri-rust`** (Opus) — owns `crates/petri-core` + `crates/petri-wasm` (sim, determinism).
- **`petri-render`** (Sonnet) — owns `app/` (WebGL2, shaders, bloom, Tweakpane).
- `petri-review` — added later, once there's code to guard.

Model policy: **Opus where correctness is load-bearing, Sonnet for mechanical work, escalate
when stuck.** After an agent finishes, tick the box in `BACKLOG.md` and update `ROADMAP.md`.

## Layout

```
crates/petri-core   pure Rust sim (native-testable)
crates/petri-wasm   wasm-bindgen Sim wrapper (zero-copy field handle)
app/                Vite + TS + WebGL2 renderer + Tweakpane UI
scripts/            build-wasm.sh
docs/               DESIGN.md (arch + decisions), ROADMAP.md, BACKLOG.md, initial-spec.md (historical)
docs/guide/         the mdBook guide (concepts & algorithms) — published to Pages
.claude/agents/     petri-rust.md, petri-render.md
.github/workflows/  ci.yml (fmt/clippy/test + wasm/lint/build), docs.yml (Pages: app + guide + API)
```

Pushing to `main` publishes the live toy, the guide, and the API docs to GitHub Pages via
`docs.yml`. The Pages site is `/` (landing) · `/app/` (toy) · `/guide/` (book) · `/api/` (rustdoc).
