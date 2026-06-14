# The Petri Polis Guide

Petri Polis is a browser toy where **complex global structure emerges from simple local rules**.
Thousands of agents each follow one short instruction — drop a trail, sense the trail ahead, steer
toward it — and with no central plan they organize into living, branching networks. On top of that
trail rule runs a light ecology (energy, food, reproduction, death) and two competing species, so the
world also breathes in boom/bust cycles.

This guide explains the **concepts and algorithms** behind it: what each rule is, why it produces
what it does, and how the code is shaped so it stays fast and reproducible. For the architecture and
the design *decisions* — and the alternatives that were rejected — see
[`docs/DESIGN.md`](https://github.com/VitalyVorobyev/petri-polis/blob/main/docs/DESIGN.md). For exact
signatures, see the [API docs](https://vitalyvorobyev.github.io/petri-polis/api/). To just watch it
run, open the [live demo](https://vitalyvorobyev.github.io/petri-polis/app/).

## The thesis: stigmergy

The organizing principle is **stigmergy** — coordination through the environment rather than through
direct communication. An agent leaves a mark (a trail deposit); other agents, and itself later, read
that mark and react to it. No agent knows about "the network." The network is simply what is left
over when many agents reinforce the paths they happen to share. Ants do this with pheromones; the
slime mold *Physarum polycephalum* does it with its own protoplasm; here it is a scalar **trail
field** that agents write to and read from.

This is the source of the "few simple rules → complex dynamics" effect. The rules are local and dumb,
but the shared medium couples every agent to every other agent's history, and reinforcement turns
small coincidences into stable global structure.

## Architecture at a glance

Petri Polis is one static web page with three layers:

```text
crates/petri-core   pure Rust simulation — the rules, the fields, the agents, the PRNG
crates/petri-wasm   a thin wasm-bindgen wrapper — hands JS zero-copy pointers into the fields
app/                TypeScript + WebGL2 — uploads the fields to the GPU and renders them
```

- **`petri-core`** is plain, dependency-free Rust. It runs natively, so the rules are unit-tested
  without a browser. It owns the simulation state and the seeded random generator.
- **`petri-wasm`** compiles the core to WebAssembly and exposes a small API. Its key trick is
  **zero-copy**: it hands JavaScript a raw pointer into a field, which JS reads directly as a
  `Float32Array` over WASM memory — no per-frame serialization.
- **`app/`** is the renderer. Each frame it uploads the fields as GPU textures, maps them through a
  bioluminescent colour palette, blooms the bright parts, and presents a full-bleed canvas.

The division is deliberate: **fast Rust where the work is, the GPU where the beauty is, no backend.**

## How to read this guide

The chapters build the system up one rule at a time:

- **The Physarum rule** and **Diffusion and decay** are the heart — the agent behaviour and the field
  update that together make the networks.
- **The simulation core** is how that is implemented to run fast: struct-of-arrays agents, the tick
  pipeline, and a tick that never allocates.
- **Determinism** explains the seeded PRNG and why the same seed always grows the same world.
- **Ecology and population dynamics** and **Two species** layer life and competition onto the trail
  core.
- **Parameter reference** is the knob-by-knob catalogue.
- **The rendering pipeline** and **WASM and the zero-copy field** cover how the simulation reaches the
  screen.

Throughout, the simulation lives in `crates/petri-core/src/lib.rs`, the boundary in
`crates/petri-wasm/src/lib.rs`, and the renderer in `app/src/main.ts`.
