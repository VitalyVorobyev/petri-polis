# WASM and the zero-copy field

The simulation is native Rust; the renderer is JavaScript in a browser. The `petri-wasm` crate is the
boundary between them. Its job is small but exacting: compile `petri-core` to WebAssembly and hand
JavaScript the fields **without copying them** every frame. This chapter explains how that works and
the one caveat that makes it safe.

## A thin wrapper

`petri-wasm` wraps `petri_core::Sim` and re-exports a flat, `u32`-friendly API through `wasm-bindgen`:

```rust
#[wasm_bindgen]
pub struct Sim { inner: CoreSim }

#[wasm_bindgen]
impl Sim {
    #[wasm_bindgen(constructor)]
    pub fn new(width: usize, height: usize, seed: u32) -> Sim { /* … */ }
    pub fn tick(&mut self) { self.inner.tick(); }
    // …
}
```

It is mechanical: downcast `usize`/`u64` to `u32` so JavaScript stays in plain numbers (no BigInt), and
forward each call to the core. The interesting part is the field handles.

## The zero-copy field

Instead of returning a copy of a field array, `petri-wasm` returns a **raw pointer** into WASM linear
memory:

```rust
pub fn field_ptr(&self, species: u32) -> *const f32 {
    self.inner.field(species as usize).as_ptr()
}
pub fn field_len(&self) -> usize {
    self.inner.field(0).len()
}
```

On the JavaScript side, that pointer plus the length builds a typed-array **view** directly over the
WASM heap — no allocation, no serialization:

```js
const view = new Float32Array(wasm.memory.buffer, sim.field_ptr(0), sim.field_len());
// view[i] now reads cell i of species 0's field straight from WASM memory
```

The same is done for the food field (`food_ptr` / `food_len`). Each frame, the renderer uploads these
views to GPU textures. The field never leaves WASM memory until the GPU reads it — that is the
"zero-copy" boundary, and it is why even large fields stay smooth.

## The caveat: memory growth detaches views

There is one sharp edge. A `Float32Array` view does not own its memory; it aliases the WASM heap at a
fixed offset. If the WASM heap ever **grows** (`memory.grow`, triggered by any Rust allocation that
exceeds the current heap), the entire linear memory may be relocated to a new `ArrayBuffer` — and every
existing view silently **detaches**, reading zeros or throwing.

Petri Polis avoids this by **never allocating after construction** (see
[The simulation core](./simulation-core.md)):

- Agent capacity is reserved once at `Sim::new`; `spawn` is capped at it; births/deaths reuse the
  reserved slots.
- The field and food buffers are allocated once and never resized; `reset` clears them in place.

So during steady-state ticking — the per-frame hot path — the heap never grows and the cached views
stay valid indefinitely. The native `no_allocation_after_new` test enforces this by asserting the
buffer pointers don't change across ticks, spawns, and resets.

The practical rule for the renderer: it is safe to cache the field views and reuse them every frame; you
only ever need to *re-fetch* a view after `spawn` or `reset`, and even then only defensively — the core
is written so those calls don't grow memory either.

## The exposed API

Beyond the field handles, the wrapper exposes everything the renderer needs, indexed by `species: u32`
where relevant:

- **Lifecycle:** `new`, `tick`, `reset(seed)`, `spawn(x, y, count, pattern, species)`.
- **Fields and exposure:** `field_ptr` / `field_len` / `field_max`, `food_ptr` / `food_len` /
  `food_max`.
- **Live parameters:** `set_params`, `set_diffuse_weight`, `set_ecology`, plus a getter for every knob
  (so the panel can initialize from the sim's defaults).
- **Counts and metrics:** `agent_count`, `species_population`, `trail_mass`, `food_total`,
  `food_coverage`.
- **Inspector:** `trail_at`, `food_at`, `nearest_agent`, and per-index `agent_species` /
  `agent_energy` / `agent_x` / `agent_y`.

Every one of these is a thin forward to `petri-core`, which keeps the boundary trivial to audit and
leaves all the real logic in the native-testable core. For the full signatures, see the
[API docs](https://vitalyvorobyev.github.io/petri-polis/api/).
