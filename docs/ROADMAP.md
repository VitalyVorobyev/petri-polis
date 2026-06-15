# Petri Polis — Roadmap

Each milestone must produce something **visually rewarding** — that's the design constraint
that keeps a side project alive. A milestone is done only when its **gate** is met.

**Status: M0–M9 are shipped.** The toy has the Physarum core, live controls, the boom/bust
ecology, two coexisting species, and the inspect/measure/share instrument; M6 added obstacles,
endpoint food sources, chemotaxis, and the reachability metric — the Physarum-solves-the-maze
demo; M7 added a one-click gallery of classical scenarios that round-trip as shareable links; M8
added cross-species sensing (a signed 2×2 matrix) for territories and predator/prey; M9 added
structure metrics (components, loops, fractal dimension, grain) and made them visible as
measured phase transitions; M10 added a headless parameter sweep that emits reproducible
phase-diagram figures. It all publishes to GitHub Pages (live demo + guide + API docs).
**M11 is the last remaining lab milestone.**

**From toy to lab.** The toy proves emergence is fun and reproducible; the lab proves the toy
*computes* — it reproduces classical complex-systems demos as one-click presets — and lets you
*measure* what it produces, via structure metrics and headless parameter sweeps that yield
phase diagrams. One experiment threads through every lab milestone: **sweep one knob slowly and
watch a regime shift.** Drag `decay` from 0.7 → 0.99 and speckle snaps into a connected network
at a threshold. That phase transition is the thing the lab makes visible and quantifiable.

## Phase 0 — Scaffolding (docs + agents)
Cross-session machinery, lean and living: the 4 docs (CLAUDE.md, DESIGN.md, ROADMAP.md,
BACKLOG.md) and 2 implementation agents (petri-rust, petri-render).
**Gate:** a fresh session can open CLAUDE.md + BACKLOG.md and know exactly what to do next.

## M0 — Toolchain round-trip ✅ done
Cargo workspace (`petri-core`, `petri-wasm`) + Vite/TS app + `wasm-pack` wired in.
**Gate:** TS imports the wasm module, calls a trivial `Sim` method, AND a WebGL2 shader draws a
test gradient — Rust↔TS↔GPU all proven in one screen.

## M1 — The hook ✅ done
`petri-core` Physarum tick (sense/steer/move/deposit + diffuse/decay) + seeded PRNG;
`petri-wasm` exposes `field_ptr`/`tick`; TS zero-copy uploads the field → colormap + bloom.
**Gate:** glowing self-organizing networks form within seconds; same seed → identical pattern.
**This is the "is it fun?" decision point.**

## M2 — Make it a toy ✅ done
Tweakpane live `set_params` (no rebuild), play/pause/step, speed (1×/2×/5×/10×), seed reset,
click-to-spawn, spawn patterns (point / ring / uniform).
**Gate:** dragging a slider visibly changes the emergence in real time; clicking spawns agents
that integrate into the network.

## M3 — Ecology layer ✅ done
Food field + per-agent energy + reproduce + death-feeds-the-world + regrowth in `petri-core`.
**Gate:** visible boom/bust cycles — networks grow, deplete, collapse, and recover.

## M4 — Species ✅ done
2–3 parameter-set species (Forager → Seeder → Builder), distinct sense/deposit + color,
blended in the render.
**Gate:** two species coexisting produce a pattern neither makes alone — and it stays legible.

## M5 — Inspect, measure & share ✅ done
Turn the toy into an instrument so runs are usable as experiments, not only watched.
- **Metrics from the sim**, computed cheaply in the field pass alongside `field_max`: total
  trail mass (Σ field), coverage fraction (cells above a threshold), mean/variance, and
  change-rate (mass Δ per tick) for steady-state detection. Exposed as WASM getters.
- **Recorded and plotted against `tick_count`, never wall-clock** — determinism is tick-level,
  so a measured run is reproducible across machines only when its series is keyed to ticks.
- **Live sparklines + CSV/JSON export** of the metric series, so a run can be analysed offline.
- **Cell/agent inspector** — hover/click readout via cheap accessors (`field_at(x,y)`,
  `nearest_agent(x,y)`).
- **Reproducibility/sharing** — current seed + params shown and copyable, encoded into a URL.

If this dashboard outgrows Tweakpane (plots, tabs, preset gallery), reconsider a lightweight
reactive layer for the panels only; the render loop stays vanilla regardless (see DESIGN D8).
**Gate:** a shared URL reproduces the same run on another load, and a run's metric series can
be exported and re-plotted.

## M6 — World geography: sources, sinks & obstacles ✅ done
The world gets a geography. An obstacle mask the trail can't enter, designated endpoint food
sources, food-attraction chemotaxis that steers agents toward those endpoints, and a
reachability / network-cost metric — the renderer marks endpoints, draws walls, and lets you
paint both.
**Gate:** the network routes around a wall and connects two endpoints; the reachability readout
flips to "connected." **This is the change that turns "pretty" into "it computes."**

## M7 — Presets: the lab bench ✅ done
Named, shareable scenarios bundling params + ecology + geometry + spawn, with a menu and a
starter gallery of classical demos (maze, Tokyo rail, capillary mesh, trunk roads, spiral
cells, boom/bust oscillator, competitive exclusion, coexistence). Built on the existing URL
codec, extended to carry geometry.
**Gate:** pick a preset and the canonical structure appears in seconds; every preset round-trips
as a shareable link.

## M8 — Cross-species sensing: ecological coupling ✅ done
Each species senses the other's trail with a signed attract/avoid weight (a 2×2 sensing
matrix) — territories, predator/prey, chasing fronts, from a handful of lines in the agent loop.
**Gate:** two species form a territory boundary or a chase that neither produces alone, and it
stays deterministic.

## M9 — Structure metrics: the render becomes a measurement ✅ done
Read-only, on-demand observables that quantify *form*: connected components & independent loops
(union-find + the grid graph's Betti number), box-counting fractal dimension, and the
autocorrelation grain length — plotted in the sparklines and CSV, and made visible by two render
modes (a component map and a long-exposure integrator). (Skeleton length/branching and a
Lyapunov-style divergence are tracked as follow-ups.)
**Gate:** a slow `decay` sweep shows the component count collapse at a threshold — a measured
phase transition, on screen and in the exported CSV.

## M10 — Headless parameter sweep: phase diagrams ✅ done
A native, dependency-free `petri-core` binary (`src/bin/sweep.rs`): vary one or two knobs across
N values × M seeds, record an order parameter (component count, with trail mass + coexistence
companions), and emit a CSV plus a hand-rolled SVG phase diagram (a heatmap in 2-D);
`std::thread` parallelizes the independent runs. Same config → byte-identical output.
**Gate:** a reproducible phase-diagram CSV — and a plotted figure — locating a regime boundary;
a `decay` sweep collapses the component count from ~17 to ~2 at the consolidation threshold.

## M11 — Evolution: heritable traits
On reproduction, copy the parent's params with a small mutation instead of the species default,
so strategies evolve under selection by the food and geometry landscape — no RL or learned
behavior. Trait-distribution metric + trait/age coloring.
**Gate:** under a fixed landscape a trait distribution measurably drifts over a run, and reset +
same seed reproduces the evolutionary trajectory.

---
*Scale, if a sweep or a live run ever needs it (`wasm-simd`, wasm threads, or a WebGPU compute
hot loop), is pulled in by the milestone that demands it — M10 most likely — not pursued as a
goal of its own.*
