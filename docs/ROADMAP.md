# Petri Polis — Roadmap

Each milestone must produce something **visually rewarding** — that's the design constraint
that keeps a side project alive. A milestone is done only when its **gate** is met.

**Status: M0–M5 are shipped.** The toy has the Physarum core, live controls, the boom/bust
ecology, two coexisting species, and the inspect/measure/share instrument — and it publishes to
GitHub Pages (live demo + guide + API docs). **M6 (scale) is the only open milestone**, and is
optional — pursued only if scale is craved.

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

## M6 — Scale (optional)
`wasm-simd` + wasm threads (rayon) for the field/agent passes, or migrate the hot loop to
**WebGPU compute** for Sage-Jenson-scale millions.
**Gate:** target agent count rises ~10×+ while holding ~60 fps. Only pursued if scale is craved.
