//! petri-wasm — the thin wasm-bindgen boundary over [`petri_core::Sim`].
//!
//! Its core job is to hand JavaScript a **zero-copy** handle to the trail field:
//! [`Sim::field_ptr`] + [`Sim::field_len`] let the renderer build a
//! `Float32Array` directly over WASM linear memory — no serialization per frame.
//!
//! Zero-copy contract (see `docs/DESIGN.md` → D6): the field buffer is allocated
//! once at [`Sim::new`] and **never reallocates**, so its pointer is stable for
//! the whole run. The caveat is *any* `Vec` growth anywhere triggers
//! `memory.grow`, which moves all of linear memory and detaches the JS view. The
//! core pre-allocates agent capacity and caps `spawn`, so steady-state ticks,
//! `spawn`, and `reset` never grow memory — the cached view stays valid.

use petri_core::{Sim as CoreSim, SpawnPattern};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Sim {
    inner: CoreSim,
}

#[wasm_bindgen]
impl Sim {
    /// Create a `width × height` world seeded by `seed`, with the default
    /// Physarum parameters and the default initial population (`≈ w*h/8` agents).
    #[wasm_bindgen(constructor)]
    pub fn new(width: usize, height: usize, seed: u32) -> Sim {
        Sim {
            inner: CoreSim::new(width, height, seed as u64),
        }
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

    /// Live agent count.
    pub fn agent_count(&self) -> u32 {
        self.inner.agent_count() as u32
    }

    /// Advance the simulation one tick (Physarum rule + field diffuse/decay).
    pub fn tick(&mut self) {
        self.inner.tick();
    }

    /// Pointer to the trail field within WASM linear memory. Pair with
    /// [`Sim::field_len`] to build `new Float32Array(memory.buffer, ptr, len)`.
    /// Stable for the run (the field buffer never reallocates).
    pub fn field_ptr(&self) -> *const f32 {
        self.inner.field().as_ptr()
    }

    pub fn field_len(&self) -> usize {
        self.inner.field().len()
    }

    /// Largest field value after the most recent tick — for renderer
    /// auto-exposure (`gain ≈ 1 / field_max`). `0.0` before the first tick.
    pub fn field_max(&self) -> f32 {
        self.inner.field_max()
    }

    /// Update all live Physarum parameters at once (no rebuild). Angles in
    /// radians, distances/steps in cells. `decay` in `(0,1)`; `deposit` in field
    /// units. The diffuse (blur) weight keeps its current value — use
    /// [`Sim::set_diffuse_weight`] for that knob.
    #[wasm_bindgen]
    pub fn set_params(
        &mut self,
        sensor_angle: f32,
        sensor_distance: f32,
        rotation_angle: f32,
        step_size: f32,
        deposit: f32,
        decay: f32,
    ) {
        let mut p = self.inner.params();
        p.sensor_angle = sensor_angle;
        p.sensor_distance = sensor_distance;
        p.rotation_angle = rotation_angle;
        p.step_size = step_size;
        p.deposit_amount = deposit;
        p.decay = decay;
        self.inner.set_params(p);
    }

    /// Set the separable-blur center-tap weight in `[0,1]` (1.0 = no diffusion).
    pub fn set_diffuse_weight(&mut self, weight: f32) {
        let mut p = self.inner.params();
        p.diffuse_weight = weight;
        self.inner.set_params(p);
    }

    /// Spawn up to `count` agents about `(x, y)`. `pattern`: `0 = point`,
    /// `1 = ring`, `2 = uniform-disk`. Total agents are capped at the agent
    /// capacity reserved at construction, which is also the `Vec`'s backing
    /// capacity — so this never pushes past it and never grows linear memory.
    /// Returns the number actually added. (The field view stays valid; the JS
    /// side need not re-fetch after `spawn`/`reset`, though it may defensively.)
    pub fn spawn(&mut self, x: f32, y: f32, count: u32, pattern: u32) -> u32 {
        let added = self
            .inner
            .spawn(x, y, count as usize, SpawnPattern::from_u32(pattern));
        added as u32
    }

    /// Re-seed and respawn the default population, reusing existing buffers
    /// (no reallocation; the field view stays valid).
    pub fn reset(&mut self, seed: u32) {
        self.inner.reset(seed as u64);
    }

    // --- read-back accessors for the renderer's parameter panel ------------

    pub fn sensor_angle(&self) -> f32 {
        self.inner.params().sensor_angle
    }
    pub fn sensor_distance(&self) -> f32 {
        self.inner.params().sensor_distance
    }
    pub fn rotation_angle(&self) -> f32 {
        self.inner.params().rotation_angle
    }
    pub fn step_size(&self) -> f32 {
        self.inner.params().step_size
    }
    pub fn deposit_amount(&self) -> f32 {
        self.inner.params().deposit_amount
    }
    pub fn decay(&self) -> f32 {
        self.inner.params().decay
    }
    pub fn diffuse_weight(&self) -> f32 {
        self.inner.params().diffuse_weight
    }
}
