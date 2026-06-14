//! petri-wasm — the thin wasm-bindgen boundary over [`petri_core::Sim`].
//!
//! Its core job is to hand JavaScript **zero-copy** handles to the scalar fields:
//! one trail field per species (selected by `species`) via
//! [`Sim::field_ptr`]/[`Sim::field_len`], plus the shared food field via
//! [`Sim::food_ptr`]/[`Sim::food_len`]. Each lets the renderer build a
//! `Float32Array` directly over WASM linear memory — no serialization per frame.
//!
//! Two species run at once. Each has its own trail field, parameter set, ecology,
//! and color (the renderer draws species 0 cyan and species 1 magenta); they
//! couple only through the shared food field. Every parameter API is indexed by a
//! `species: u32` in `0..species_count()`.
//!
//! Zero-copy contract (see `docs/DESIGN.md` → D6): the per-species trail buffers
//! and the food buffer are allocated once at [`Sim::new`] and **never reallocate**,
//! so their pointers are stable for the whole run. The caveat is *any* `Vec` growth
//! anywhere triggers `memory.grow`, which moves all of linear memory and detaches
//! the JS views. The core pre-allocates agent capacity and caps `spawn`, and
//! births/deaths reuse that reserved capacity, so steady-state ticks, `spawn`, and
//! `reset` never grow memory — the cached views stay valid.

use petri_core::{Sim as CoreSim, SpawnPattern, SPECIES};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Sim {
    inner: CoreSim,
}

#[wasm_bindgen]
impl Sim {
    /// Create a `width × height` world seeded by `seed`, with the default
    /// per-species Physarum parameters and the default initial population
    /// (`≈ w*h/8` agents) split evenly across the species.
    #[wasm_bindgen(constructor)]
    pub fn new(width: usize, height: usize, seed: u32) -> Sim {
        Sim {
            inner: CoreSim::new(width, height, seed as u64),
        }
    }

    /// Number of species the sim runs (each with its own trail field + params).
    pub fn species_count(&self) -> u32 {
        SPECIES as u32
    }

    pub fn width(&self) -> usize {
        self.inner.width()
    }

    pub fn height(&self) -> usize {
        self.inner.height()
    }

    /// Tick counter as `u32` to keep JS in plain numbers (avoids BigInt).
    pub fn tick_count(&self) -> u32 {
        self.inner.tick_count() as u32
    }

    /// Total live agent count across all species.
    pub fn agent_count(&self) -> u32 {
        self.inner.agent_count() as u32
    }

    /// Live agent count for a single `species` — for the per-species HUD.
    pub fn species_population(&self, species: u32) -> u32 {
        self.inner.species_population(species as usize) as u32
    }

    /// Advance the simulation one tick (per-species Physarum rule + field
    /// diffuse/decay, shared ecology over the food field).
    pub fn tick(&mut self) {
        self.inner.tick();
    }

    /// Pointer to `species`' trail field within WASM linear memory. Pair with
    /// [`Sim::field_len`] to build `new Float32Array(memory.buffer, ptr, len)`.
    /// Stable for the run (the field buffers never reallocate).
    pub fn field_ptr(&self, species: u32) -> *const f32 {
        self.inner.field(species as usize).as_ptr()
    }

    /// Length of a trail field in cells (`width * height`). Same for every species
    /// and for the food field.
    pub fn field_len(&self) -> usize {
        self.inner.field(0).len()
    }

    /// Largest value of `species`' field after the most recent tick — for
    /// per-species renderer auto-exposure (`gain ≈ 1 / field_max`). `0.0` before
    /// the first tick.
    pub fn field_max(&self, species: u32) -> f32 {
        self.inner.field_max(species as usize)
    }

    /// Pointer to the shared food field within WASM linear memory. Pair with
    /// [`Sim::food_len`] to build `new Float32Array(memory.buffer, ptr, len)`.
    /// Stable for the run (the food buffer never reallocates, same as the trail
    /// fields) — re-fetch only after `spawn`/`reset` if you re-fetch a field view.
    pub fn food_ptr(&self) -> *const f32 {
        self.inner.food().as_ptr()
    }

    pub fn food_len(&self) -> usize {
        self.inner.food().len()
    }

    /// Largest food value after the most recent tick — for renderer normalization
    /// (`gain ≈ 1 / food_max`). Equals the patch peak right after `new`/`reset`.
    pub fn food_max(&self) -> f32 {
        self.inner.food_max()
    }

    /// Update all live Physarum parameters for `species` at once (no rebuild).
    /// Angles in radians, distances/steps in cells. `decay` in `(0,1)`; `deposit`
    /// in field units. The diffuse (blur) weight keeps its current value — use
    /// [`Sim::set_diffuse_weight`] for that knob.
    // A flat argument list (not a struct) is the natural shape for the JS boundary:
    // the renderer's panel calls this with plain numbers.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen]
    pub fn set_params(
        &mut self,
        species: u32,
        sensor_angle: f32,
        sensor_distance: f32,
        rotation_angle: f32,
        step_size: f32,
        deposit: f32,
        decay: f32,
    ) {
        let s = species as usize;
        let mut p = self.inner.params(s);
        p.sensor_angle = sensor_angle;
        p.sensor_distance = sensor_distance;
        p.rotation_angle = rotation_angle;
        p.step_size = step_size;
        p.deposit_amount = deposit;
        p.decay = decay;
        self.inner.set_params(s, p);
    }

    /// Set the separable-blur center-tap weight for `species` in `[0,1]`
    /// (1.0 = no diffusion).
    pub fn set_diffuse_weight(&mut self, species: u32, weight: f32) {
        let s = species as usize;
        let mut p = self.inner.params(s);
        p.diffuse_weight = weight;
        self.inner.set_params(s, p);
    }

    /// Update all live ecology parameters for `species` at once (no rebuild).
    /// `metabolism`, `eat_rate`, `repro_threshold`, `death_return` are in
    /// energy/food units; `food_regrow` is a per-tick fraction in `(0,1)`. The
    /// food field is shared, so its effective regrow rate is the mean of the
    /// species' `food_regrow` values. Takes effect next tick.
    #[wasm_bindgen]
    pub fn set_ecology(
        &mut self,
        species: u32,
        metabolism: f32,
        eat_rate: f32,
        repro_threshold: f32,
        food_regrow: f32,
        death_return: f32,
    ) {
        let s = species as usize;
        let mut e = self.inner.ecology(s);
        e.metabolism = metabolism;
        e.eat_rate = eat_rate;
        e.repro_threshold = repro_threshold;
        e.food_regrow = food_regrow;
        e.death_return = death_return;
        self.inner.set_ecology(s, e);
    }

    /// Spawn up to `count` agents of `species` about `(x, y)`. `pattern`:
    /// `0 = point`, `1 = ring`, `2 = uniform-disk`. Total agents are capped at the
    /// agent capacity reserved at construction, which is also the `Vec`'s backing
    /// capacity — so this never pushes past it and never grows linear memory.
    /// Returns the number actually added. (The field views stay valid; the JS side
    /// need not re-fetch after `spawn`/`reset`, though it may defensively.)
    pub fn spawn(&mut self, x: f32, y: f32, count: u32, pattern: u32, species: u32) -> u32 {
        let added = self.inner.spawn(
            x,
            y,
            count as usize,
            SpawnPattern::from_u32(pattern),
            species as usize,
        );
        added as u32
    }

    /// Re-seed and respawn the default population, reusing existing buffers
    /// (no reallocation; the field views stay valid).
    pub fn reset(&mut self, seed: u32) {
        self.inner.reset(seed as u64);
    }

    // --- per-species Physarum read-back accessors for the parameter panel ---

    pub fn sensor_angle(&self, species: u32) -> f32 {
        self.inner.params(species as usize).sensor_angle
    }
    pub fn sensor_distance(&self, species: u32) -> f32 {
        self.inner.params(species as usize).sensor_distance
    }
    pub fn rotation_angle(&self, species: u32) -> f32 {
        self.inner.params(species as usize).rotation_angle
    }
    pub fn step_size(&self, species: u32) -> f32 {
        self.inner.params(species as usize).step_size
    }
    pub fn deposit_amount(&self, species: u32) -> f32 {
        self.inner.params(species as usize).deposit_amount
    }
    pub fn decay(&self, species: u32) -> f32 {
        self.inner.params(species as usize).decay
    }
    pub fn diffuse_weight(&self, species: u32) -> f32 {
        self.inner.params(species as usize).diffuse_weight
    }

    // --- per-species ecology read-back accessors for the parameter panel ---

    pub fn metabolism(&self, species: u32) -> f32 {
        self.inner.ecology(species as usize).metabolism
    }
    pub fn eat_rate(&self, species: u32) -> f32 {
        self.inner.ecology(species as usize).eat_rate
    }
    pub fn repro_threshold(&self, species: u32) -> f32 {
        self.inner.ecology(species as usize).repro_threshold
    }
    pub fn food_regrow(&self, species: u32) -> f32 {
        self.inner.ecology(species as usize).food_regrow
    }
    pub fn death_return(&self, species: u32) -> f32 {
        self.inner.ecology(species as usize).death_return
    }

    // --- world metrics (cheap reductions computed inside the tick passes) ---

    /// Total trail mass of `species` — the sum of every cell in its trail field
    /// after the most recent tick. `f64`, **unbounded**: it grows with deposit and
    /// population and shrinks with decay. `0.0` before the first tick.
    pub fn trail_mass(&self, species: u32) -> f64 {
        self.inner.trail_mass(species as usize)
    }

    /// Total food remaining — the sum of every cell in the shared food field after
    /// the most recent tick. `f64`, bounded above by the sum of the food caps (its
    /// value right after `new`/`reset`, when food starts full).
    pub fn food_total(&self) -> f64 {
        self.inner.food_total()
    }

    /// Fraction of cells holding more than a small epsilon of food, in `[0, 1]` —
    /// a coarse gauge of how much of the world is fed.
    pub fn food_coverage(&self) -> f32 {
        self.inner.food_coverage()
    }

    // --- read-only inspector accessors (no allocation, no stored query state) ---

    /// Value of `species`' trail field at the cell containing `(x, y)` (floored and
    /// wrapped toroidally). `0.0` for an out-of-range species.
    pub fn trail_at(&self, species: u32, x: f32, y: f32) -> f32 {
        self.inner.trail_at(species as usize, x, y)
    }

    /// Value of the shared food field at the cell containing `(x, y)` (floored and
    /// wrapped toroidally).
    pub fn food_at(&self, x: f32, y: f32) -> f32 {
        self.inner.food_at(x, y)
    }

    /// Index of the live agent nearest to `(x, y)` by **toroidal** distance, or
    /// `-1` if there are no agents. Pass the result to the `agent_*` getters to read
    /// the picked agent. The index is valid only until the next
    /// `tick`/`spawn`/`reset`, so query it and the getters together between ticks.
    pub fn nearest_agent(&self, x: f32, y: f32) -> i32 {
        self.inner.nearest_agent(x, y)
    }

    /// Species tag of agent `i` (valid until the next `tick`/`spawn`/`reset`).
    /// Returns `0` for an out-of-range index.
    pub fn agent_species(&self, i: u32) -> u32 {
        self.inner.agent_species(i as usize) as u32
    }

    /// Energy of agent `i`. Returns `0.0` for an out-of-range index.
    pub fn agent_energy(&self, i: u32) -> f32 {
        self.inner.agent_energy(i as usize)
    }

    /// `x` position (cells, in `[0, width)`) of agent `i`. `0.0` if out of range.
    pub fn agent_x(&self, i: u32) -> f32 {
        self.inner.agent_x(i as usize)
    }

    /// `y` position (cells, in `[0, height)`) of agent `i`. `0.0` if out of range.
    pub fn agent_y(&self, i: u32) -> f32 {
        self.inner.agent_y(i as usize)
    }
}
