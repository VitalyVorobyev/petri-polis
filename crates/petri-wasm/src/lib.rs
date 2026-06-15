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

    // --- World geography: obstacle mask, endpoint food sources, chemotaxis, and
    // the reachability / network-cost metric (the "Physarum solves the maze" kit).

    /// Pointer to the obstacle mask within WASM linear memory (`u8` per cell,
    /// `0` = open, `1` = wall). Pair with [`Sim::obstacle_len`] to build
    /// `new Uint8Array(memory.buffer, ptr, len)`. Stable for the run (the mask never
    /// reallocates) — re-fetch only after a reset-class call (`reset`,
    /// `load_maze_demo`), like the field/food views.
    pub fn obstacle_ptr(&self) -> *const u8 {
        self.inner.obstacles().as_ptr()
    }

    /// Length of the obstacle mask in cells (`width * height`, same as a field).
    pub fn obstacle_len(&self) -> usize {
        self.inner.obstacles().len()
    }

    /// Number of wall cells currently set. `0` means a fully open world (and the
    /// determinism fast path — every geography branch in the tick is skipped).
    pub fn obstacle_count(&self) -> u32 {
        self.inner.obstacle_count() as u32
    }

    /// Paint (`on = true`) or erase (`on = false`) a filled disk of wall cells at
    /// `(x, y)` with the given `radius` (cells, toroidal). In-place — never grows
    /// linear memory, so the field/food/obstacle views stay valid (no re-fetch).
    pub fn paint_obstacle(&mut self, x: f32, y: f32, radius: f32, on: bool) {
        self.inner.paint_obstacle(x, y, radius, on);
    }

    /// Clear every wall, returning the world to fully open. In-place (no re-fetch).
    pub fn clear_obstacles(&mut self) {
        self.inner.clear_obstacles();
    }

    /// Add a persistent endpoint food source at `(x, y)` with `radius` (cells) — a
    /// well that keeps regrowing and draws food-attracted agents, doubling as a
    /// reachability seed/target and a renderer marker. **Reset-class:** it
    /// recomputes the food caps and refills food, so re-fetch the food view after
    /// (the pointer is stable; the contents change).
    pub fn add_endpoint(&mut self, x: f32, y: f32, radius: f32) {
        self.inner.add_endpoint(x, y, radius);
    }

    /// Remove every endpoint and recompute the food caps. Reset-class (re-fetch the
    /// food view after).
    pub fn clear_endpoints(&mut self) {
        self.inner.clear_endpoints();
    }

    /// Number of endpoint food sources.
    pub fn endpoint_count(&self) -> u32 {
        self.inner.endpoint_count() as u32
    }

    /// `x` of endpoint `i` (cells), or `0.0` if out of range — for the renderer's
    /// endpoint markers.
    pub fn endpoint_x(&self, i: u32) -> f32 {
        self.inner.endpoint_x(i as usize)
    }

    /// `y` of endpoint `i` (cells), or `0.0` if out of range.
    pub fn endpoint_y(&self, i: u32) -> f32 {
        self.inner.endpoint_y(i as usize)
    }

    /// Radius of endpoint `i` (cells), or `0.0` if out of range.
    pub fn endpoint_radius(&self, i: u32) -> f32 {
        self.inner.endpoint_radius(i as usize)
    }

    /// Set `species`' food-attraction (chemotaxis) weight. `0.0` (the default) =
    /// pure self-trail Physarum; a positive value steers agents up the food gradient
    /// toward endpoints. Takes effect next tick; no rebuild.
    pub fn set_food_attraction(&mut self, species: u32, weight: f32) {
        let s = species as usize;
        let mut p = self.inner.params(s);
        p.food_attraction = weight;
        self.inner.set_params(s, p);
    }

    /// `species`' current food-attraction weight (for the parameter panel).
    pub fn food_attraction(&self, species: u32) -> f32 {
        self.inner.params(species as usize).food_attraction
    }

    /// Set the cross-species sensing weight `cross_sense[species][other]` — how
    /// strongly an agent of `species` is pulled by `other`'s trail at a sensor. A
    /// positive weight attracts it to the other species' trail, a negative one repels
    /// it. The diagonal (`species == other`) is the agent's own-trail weight (default
    /// `1.0`); off-diagonals default to `0.0` (no coupling). Takes effect next tick; no
    /// rebuild. While the matrix is the default identity, the sim runs the byte-identical
    /// own-trail fast path.
    pub fn set_cross_sense(&mut self, species: u32, other: u32, weight: f32) {
        self.inner
            .set_cross_sense(species as usize, other as usize, weight);
    }

    /// Current cross-species sensing weight `cross_sense[species][other]` (for the
    /// parameter panel). Out-of-range indices return `0.0`.
    pub fn cross_sense(&self, species: u32, other: u32) -> f32 {
        self.inner.cross_sense(species as usize, other as usize)
    }

    // --- Evolution: a single heritable trait (`sensor_distance`) per agent, mutated
    // on reproduction and selected by the food/geometry landscape. Default off →
    // byte-identical to the base rule (no extra RNG, the trait field stays zero). ---

    /// Enable or disable evolution for `species`. Enabling seeds a **uniform
    /// population** at the current `sensor_distance` (every live agent of `species`
    /// reset to the param), then births mutate it so the trait drifts under selection.
    /// While *no* species has evolution on, the sim runs the byte-identical base rule.
    /// Disabling the last evolving species clears the trait field. In-place (no
    /// re-fetch of the field/food views needed).
    pub fn set_evolution(&mut self, species: u32, enabled: bool) {
        self.inner.set_evolution(species as usize, enabled);
    }

    /// Whether evolution is enabled for `species` (for the panel). `false` for an
    /// out-of-range index.
    pub fn evolution_enabled(&self, species: u32) -> bool {
        self.inner.evolution_enabled(species as usize)
    }

    /// Set `species`' per-birth mutation strength — the std-dev (cells) of the
    /// Gaussian added to a child's inherited `sensor_distance`, clamped non-negative.
    /// Only consulted while evolution is enabled for that species. Takes effect next
    /// tick; no rebuild.
    pub fn set_mutation_strength(&mut self, species: u32, strength: f32) {
        self.inner.set_mutation_strength(species as usize, strength);
    }

    /// `species`' current per-birth mutation strength (cells). `0.0` for an
    /// out-of-range index.
    pub fn mutation_strength(&self, species: u32) -> f32 {
        self.inner.mutation_strength(species as usize)
    }

    /// Mean of the heritable `sensor_distance` trait over `species`' live agents
    /// (cells). `0.0` when the species has no live agents or is out of range.
    pub fn trait_mean(&self, species: u32) -> f32 {
        self.inner.trait_mean(species as usize)
    }

    /// Population std-dev of the `sensor_distance` trait over `species`' live agents.
    /// `0.0` when the species has fewer than two live agents or is out of range.
    pub fn trait_std(&self, species: u32) -> f32 {
        self.inner.trait_std(species as usize)
    }

    /// Smallest `sensor_distance` trait among `species`' live agents (cells). `0.0`
    /// when the species has no live agents or is out of range.
    pub fn trait_min(&self, species: u32) -> f32 {
        self.inner.trait_min(species as usize)
    }

    /// Largest `sensor_distance` trait among `species`' live agents (cells). `0.0`
    /// when the species has no live agents or is out of range.
    pub fn trait_max(&self, species: u32) -> f32 {
        self.inner.trait_max(species as usize)
    }

    /// Heritable `sensor_distance` trait of agent `i` (cells; the value it senses
    /// with when its species is evolving). `0.0` for an out-of-range index. Valid only
    /// until the next `tick`/`spawn`/`reset`, like the other `agent_*` getters.
    pub fn agent_trait(&self, i: u32) -> f32 {
        self.inner.agent_trait(i as usize)
    }

    /// Pointer to the trait field within WASM linear memory (`f32` per cell: the
    /// `sensor_distance` of the last agent to deposit there, `0.0` where none has, and
    /// everywhere while no species is evolving). Pair with [`Sim::trait_field_len`] to
    /// build `new Float32Array(memory.buffer, ptr, len)` for a trait-map render mode.
    /// Stable for the run (it never reallocates) — re-fetch only after a reset-class
    /// call, like the field/obstacle views.
    pub fn trait_field_ptr(&self) -> *const f32 {
        self.inner.trait_field().as_ptr()
    }

    /// Length of the trait field in cells (`width * height`, same as a trail field).
    pub fn trait_field_len(&self) -> usize {
        self.inner.trait_field().len()
    }

    /// Set the reachability threshold — the fraction of the current combined
    /// `field_max` a cell's combined trail must reach to count as network. Clamped to
    /// `[0, 1]`. Affects only the on-demand metric, not the sim.
    pub fn set_network_threshold(&mut self, t: f32) {
        self.inner.set_network_threshold(t);
    }

    /// Current reachability threshold (fraction of combined `field_max`).
    pub fn network_threshold(&self) -> f32 {
        self.inner.network_threshold()
    }

    /// How many endpoints (including endpoint 0) are reachable from endpoint 0 along
    /// the thresholded combined trail network (open cells only, toroidal). `0` if
    /// there are no endpoints. On-demand read-only reduction (does not tick the sim).
    pub fn endpoints_connected(&mut self) -> u32 {
        self.inner.endpoints_connected()
    }

    /// Number of cells in the reachable network from endpoint 0 — a length/cost proxy
    /// for the connecting structure. `0` if no endpoints or no above-threshold
    /// network. On-demand read-only reduction.
    pub fn network_cost(&mut self) -> u32 {
        self.inner.network_cost()
    }

    // --- Structure metrics: cheap, read-only observables over the thresholded
    // combined-trail foreground (same masking as `network_cost`: open cells whose
    // combined trail clears `network_threshold * combined_max`, 4-connected,
    // toroidal). All are computed on demand — they never tick the sim, mutate its
    // state, or draw RNG. The ones writing scratch take `&mut self`. ---

    /// Number of connected components in the thresholded combined-trail foreground —
    /// high for a scattered speckle of blobs, dropping toward `1` as the network
    /// links up. `0` when there is no foreground. Fills the component-label buffer as
    /// a side effect (see [`Sim::component_labels_ptr`]). On-demand read-only.
    pub fn component_count(&mut self) -> u32 {
        self.inner.component_count()
    }

    /// Number of independent loops (cycles) in the foreground — the first Betti
    /// number `b1 = edges - cells + components` of the 4-connected grid graph. `0`
    /// for a tree/forest, positive once the network closes a loop. Also fills the
    /// component-label buffer. On-demand read-only.
    pub fn loop_count(&mut self) -> u32 {
        self.inner.loop_count()
    }

    /// Box-counting (Minkowski–Bouligand) fractal dimension of the foreground —
    /// least-squares slope of `log(occupied boxes)` vs `log(1/box size)` over
    /// power-of-two box sizes. Near `1` for a sparse, filament-like network; toward
    /// `2` for a space-filling one. `0.0` for an empty foreground. On-demand
    /// read-only.
    pub fn fractal_dimension(&mut self) -> f32 {
        self.inner.fractal_dimension()
    }

    /// Spatial autocorrelation length of the combined trail field, in cells — the
    /// grain size, read off as the lag at which the radially-averaged autocorrelation
    /// first falls to `1/e`. Larger for a coarse field (broad blobs), smaller for a
    /// fine one. `0.0` for a flat field. On-demand read-only.
    pub fn autocorrelation_length(&self) -> f32 {
        self.inner.autocorrelation_length()
    }

    /// Pointer to the per-cell component-label buffer within WASM linear memory
    /// (`u32` per cell: `0` = background, else a `1..=component_count` component id).
    /// Pair with [`Sim::component_labels_len`] to build
    /// `new Uint32Array(memory.buffer, ptr, len)` for a component-map overlay. The
    /// buffer is filled by the most recent `component_count` / `loop_count` call
    /// (all-zero before any call). Stable for the run (it never reallocates) —
    /// re-fetch only after a reset-class call, like the field/obstacle views.
    pub fn component_labels_ptr(&self) -> *const u32 {
        self.inner.component_labels().as_ptr()
    }

    /// Length of the component-label buffer in cells (`width * height`, same as a
    /// field).
    pub fn component_labels_len(&self) -> usize {
        self.inner.component_labels().len()
    }

    /// Load the built-in "Physarum solves the maze" scenario: a wall maze, two
    /// endpoint food wells, food-attraction on, and a population seeded in the open
    /// left corridor. **Reset-class** — it rewrites obstacles, endpoints, food, and
    /// the population (all in place, no reallocation), so re-fetch the
    /// field/food/obstacle views afterward (pointers stay valid; contents change).
    pub fn load_maze_demo(&mut self) {
        self.inner.load_maze_demo();
    }
}
