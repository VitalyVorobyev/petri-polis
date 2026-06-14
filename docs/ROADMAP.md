# Petri Polis — Roadmap

Each milestone must produce something **visually rewarding** — that's the design constraint
that keeps a side project alive. A milestone is done only when its **gate** is met.

## Phase 0 — Scaffolding (docs + agents)
Cross-session machinery, lean and living: the 4 docs (CLAUDE.md, DESIGN.md, ROADMAP.md,
BACKLOG.md) and 2 implementation agents (petri-rust, petri-render).
**Gate:** a fresh session can open CLAUDE.md + BACKLOG.md and know exactly what to do next.

## M0 — Toolchain round-trip
Cargo workspace (`petri-core`, `petri-wasm`) + Vite/TS app + `wasm-pack` wired in.
**Gate:** TS imports the wasm module, calls a trivial `Sim` method, AND a WebGL2 shader draws a
test gradient — Rust↔TS↔GPU all proven in one screen.

## M1 — The hook
`petri-core` Physarum tick (sense/steer/move/deposit + diffuse/decay) + seeded PRNG;
`petri-wasm` exposes `field_ptr`/`tick`; TS zero-copy uploads the field → colormap + bloom.
**Gate:** glowing self-organizing networks form within seconds; same seed → identical pattern.
**This is the "is it fun?" decision point.**

## M2 — Make it a toy
Tweakpane live `set_params` (no rebuild), play/pause/step, speed (1×/2×/5×/10×), seed reset,
click-to-spawn, spawn patterns (point / ring / uniform).
**Gate:** dragging a slider visibly changes the emergence in real time; clicking spawns agents
that integrate into the network.

## M3 — Ecology layer
Food field + per-agent energy + reproduce + death-feeds-the-world + regrowth in `petri-core`.
**Gate:** visible boom/bust cycles — networks grow, deplete, collapse, and recover.

## M4 — Species
2–3 parameter-set species (Forager → Seeder → Builder), distinct sense/deposit + color,
blended in the render.
**Gate:** two species coexisting produce a pattern neither makes alone — and it stays legible.

## M5 — Inspect & share
Click → nearest agent/cell readout; tiny live metrics (pop, total trail); encode params+seed
into a URL so a beautiful run is shareable.
**Gate:** a shared URL reproduces the same beautiful run on another load.

## M6 — Scale (optional)
`wasm-simd` + wasm threads (rayon) for the field/agent passes, or migrate the hot loop to
**WebGPU compute** for Sage-Jenson-scale millions.
**Gate:** target agent count rises ~10×+ while holding ~60 fps. Only pursued if scale is craved.
