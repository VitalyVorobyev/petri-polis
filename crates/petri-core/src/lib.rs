//! petri-core — the pure, native-testable simulation core for Petri Polis.
//!
//! The [`Sim`] runs the **Physarum** (slime-mold) trail rule
//! (Jeff Jones 2010) coupled to a lightweight **ecology**, for **two competing
//! species**. Each species owns its own trail field, its own parameter set, and
//! its own ecology timing. Agents deposit into and sense only their own species'
//! trail, so each species self-organizes its own interwoven vascular network. The
//! single point of coupling is a **shared `food` field** — both species draw
//! energy from the same regrowing nutrient patches, so they compete spatially at
//! the boundaries of their networks and through food depletion. This produces
//! niche partitioning: two distinct, legible networks (rendered cyan and magenta)
//! that interleave into a combined picture neither species makes alone.
//!
//! On top of the trail rule, agents pay a metabolic cost each tick, eat from the
//! shared cell they stand on, reproduce when well-fed (the child inherits the
//! parent's species), and die when starved, returning energy to the shared food
//! field. The result is boom/bust cycles — networks grow into rich patches,
//! deplete them, collapse, and rebound from survivors — staggered between the two
//! species by their distinct ecology timing. See `docs/DESIGN.md` → "The Physarum
//! rule" for the trail spec.
//!
//! ## Invariants this crate upholds
//! - **Determinism.** All randomness flows from one seeded [`Rng`] owned by the
//!   sim. `same seed → identical run`. No wall-clock, no map iteration order.
//! - **Hot-loop discipline.** Agents are struct-of-arrays (`Vec<f32>` per
//!   attribute, plus a `species` tag). Capacity is pre-allocated at [`Sim::new`];
//!   [`Sim::tick`] never allocates. Births `push` within the reserved capacity,
//!   deaths `swap_remove`, so the population is dynamic without growing memory.
//!   The per-species trail fields are double-buffered through one shared scratch
//!   buffer for the blur (swap, don't copy).
//! - **Zero-copy.** The per-species trail fields and the shared `food` field are
//!   fixed-size buffers allocated once at [`Sim::new`]; they never reallocate, so
//!   the `petri-wasm` pointers the JS `Float32Array` views alias stay valid for
//!   the whole run.

use std::f32::consts::TAU;

/// Number of agent species the sim simulates. Each has its own trail field,
/// parameter set, ecology, and color. Two species are tuned to coexist (cyan +
/// magenta); the structure generalizes — bumping this constant and adding default
/// param-sets is all a third species needs.
pub const SPECIES: usize = 2;

/// Default agent slots reserved at construction. The sim never allocates past
/// this; [`Sim::spawn`] caps total agents here so WASM linear memory never grows
/// during a run (which would detach the JS field view — see `docs/DESIGN.md` D6).
pub const DEFAULT_AGENT_CAPACITY: usize = 200_000;

/// Seeded PRNG — xoshiro256** (Blackman & Vigna, 2018), hand-rolled and
/// dependency-free so behavior is stable across versions and platforms.
///
/// Seeded through SplitMix64 so a single `u64` seed expands to a well-mixed
/// 256-bit state (and is never all-zero, which xoshiro forbids).
#[derive(Clone)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// Seed the generator from a single `u64`. Any seed (including 0) yields a
    /// valid, well-distributed state.
    pub fn seed(seed: u64) -> Self {
        // SplitMix64 to fill the 256-bit state from one word.
        let mut z = seed;
        let mut next = || -> u64 {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        let s = [next(), next(), next(), next()];
        // SplitMix64 cannot return all-zero for the 4-word vector with these
        // additive increments, but guard anyway: an all-zero state is invalid.
        let s = if s == [0, 0, 0, 0] { [1, 2, 3, 4] } else { s };
        Self { s }
    }

    #[inline(always)]
    fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform `f32` in `[0, 1)` (24-bit mantissa precision).
    #[inline(always)]
    pub fn next_f32(&mut self) -> f32 {
        // Top 24 bits → [0, 1); matches f32 mantissa width.
        ((self.next_u64() >> 40) as f32) * (1.0 / (1u32 << 24) as f32)
    }

    /// Uniform `f32` in `[0, max)`.
    #[inline(always)]
    pub fn range(&mut self, max: f32) -> f32 {
        self.next_f32() * max
    }
}

/// Live, runtime-tunable Physarum parameters. Defaults are Jones-style and chosen
/// to produce clean networks on a `256×256`-ish field. Units are documented per
/// field. Everything here is reachable from `set_params` (no rebuild to retune).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Angle between the center sensor and each side sensor. **Radians.**
    pub sensor_angle: f32,
    /// Distance ahead the three sensors sample the field. **Cells.**
    pub sensor_distance: f32,
    /// Amount the agent turns per tick when steering. **Radians.**
    pub rotation_angle: f32,
    /// Distance the agent advances each tick. **Cells.**
    pub step_size: f32,
    /// Trail added to the field cell under the agent after it moves. **Field units.**
    pub deposit_amount: f32,
    /// Evaporation factor applied to the whole field each tick (`field *= decay`).
    /// **Unitless multiplier**, `0 < decay < 1`; smaller = faster fade.
    pub decay: f32,
    /// Diffusion weight of the center tap in the separable 3-tap blur, in `[0, 1]`.
    /// The two neighbor taps share `(1 - diffuse_weight)`. `1.0` = no blur.
    pub diffuse_weight: f32,
    /// Chemotaxis weight: how strongly the agent steers toward food. Each of the
    /// three sensor readings gains `food_attraction * food_sample` (the food field
    /// sampled at that sensor), so a positive value biases agents up the food
    /// gradient toward endpoints. **Unitless** (food units → field-comparable
    /// units). `0.0` = pure self-trail Physarum (the default; food is ignored).
    pub food_attraction: f32,
}

impl Params {
    /// Default Physarum parameters for species `s`. The two species are tuned to
    /// build visually and behaviorally distinct networks so the combined picture
    /// reads as two interwoven systems:
    /// - **Species 0** — a finer, faster mesh: short sensors, a sharper turn, and
    ///   a lighter deposit, so it weaves a dense capillary network.
    /// - **Species 1** — coarser, thicker veins: long sensors, a gentler turn, and
    ///   a heavier deposit, so it lays down a few broad trunk routes.
    ///
    /// `s >= SPECIES` falls back to the species-0 set.
    pub fn default_for(s: usize) -> Self {
        match s {
            1 => Self {
                sensor_angle: 0.34,    // ~19.5°
                sensor_distance: 16.0, // long reach → coarse, far-apart veins
                rotation_angle: 0.28,  // ~16° — gentle turns → smooth trunks
                step_size: 1.3,        // cells/tick — moves faster along its veins
                deposit_amount: 7.0,   // heavy trail → thick, persistent veins
                decay: 0.92,           // slow fade → long-lived trunk network
                diffuse_weight: 0.55,  // center tap weight; neighbors share the rest
                food_attraction: 0.0,  // off by default → pure self-trail Physarum
            },
            // Species 0 (and any out-of-range index) — the fine, fast mesh.
            _ => Self {
                sensor_angle: 0.46,   // ~26.5° — wide sensing → bushy branching
                sensor_distance: 7.0, // short reach → fine, closely-spaced mesh
                rotation_angle: 0.46, // ~26.5° — sharp turns → tight capillaries
                step_size: 1.0,       // cells/tick
                deposit_amount: 4.0,  // light trail → delicate filaments
                decay: 0.88,          // faster fade → quick-turnover mesh
                diffuse_weight: 0.5,  // center tap weight; neighbors share the rest
                food_attraction: 0.0, // off by default → pure self-trail Physarum
            },
        }
    }
}

impl Default for Params {
    /// The species-0 parameter set.
    fn default() -> Self {
        Self::default_for(0)
    }
}

/// Live, runtime-tunable ecology parameters. Defaults are tuned so a
/// `256×256`-ish world produces visible boom/bust cycles (population grows into
/// food patches, depletes them, crashes, then rebounds from survivors). All five
/// are reachable from `set_ecology` (no rebuild to retune).
#[derive(Clone, Copy, Debug)]
pub struct Ecology {
    /// Energy an agent loses every tick — the cost of living. **Energy units/tick.**
    pub metabolism: f32,
    /// Max food an agent consumes from its current cell per tick, converted 1:1 to
    /// energy. **Food units/tick.**
    pub eat_rate: f32,
    /// Energy at or above which an agent reproduces (splitting its energy with the
    /// child). **Energy units.**
    pub repro_threshold: f32,
    /// Per-tick regrowth fraction pulling each food cell back toward its local cap:
    /// `food += food_regrow * (food_cap - food)`. **Unitless**, `0..1`.
    pub food_regrow: f32,
    /// Food deposited at an agent's cell when it dies — death feeds the world.
    /// **Food units.**
    pub death_return: f32,
}

impl Ecology {
    /// Default ecology parameters for species `s`. The two species share the food
    /// field but metabolize it on different schedules, so their boom/bust cycles
    /// desync rather than crashing in lockstep — when one is busting, the other is
    /// often booming on the freed-up food, which is what keeps both alive:
    /// - **Species 0** — lean and fast: cheaper to live, eats less per bite, splits
    ///   sooner, and lives on faster-cycling food (quick boom/bust).
    /// - **Species 1** — slower and hungrier: costs more to live, eats more per
    ///   bite, splits later, and rides a slower food-regrow cycle (longer waves).
    ///
    /// `s >= SPECIES` falls back to the species-0 set.
    pub fn default_for(s: usize) -> Self {
        match s {
            1 => Self {
                metabolism: 0.0058,    // energy/tick: cost of living
                eat_rate: 0.10,        // food/tick eaten from the current cell
                repro_threshold: 1.28, // energy needed to split — reproduces later
                food_regrow: 0.0042,   // its half of the shared regrow cycle
                death_return: 0.30,    // corpse feeds the cell it died on
            },
            // Species 0 (and any out-of-range index) — lean and fast.
            _ => Self {
                metabolism: 0.0058,    // energy/tick: cost of living
                eat_rate: 0.10,        // food/tick eaten from the current cell
                repro_threshold: 1.12, // energy needed to split — reproduces sooner
                food_regrow: 0.0042,   // its half of the shared regrow cycle
                death_return: 0.30,    // corpse feeds the cell it died on
            },
        }
    }
}

impl Default for Ecology {
    /// The species-0 ecology set.
    fn default() -> Self {
        Self::default_for(0)
    }
}

/// Number of soft Gaussian food patches generated at construction/reset. Patchy
/// food (rich pockets between near-empty space) is what lets a crashed population
/// rebound from survivors sheltering in a still-rich patch — it is the engine of
/// the boom/bust cycle, so it is a fixed structural choice, not a live knob.
const FOOD_PATCH_COUNT: usize = 16;
/// Peak food value a patch center reaches (patches are combined by `max`, so the
/// food field tops out near this and falls toward 0 between patches).
const FOOD_PATCH_PEAK: f32 = 1.0;
/// Patch radius as a fraction of the smaller field dimension — sets how broad each
/// soft Gaussian pocket is.
const FOOD_PATCH_RADIUS_FRAC_MIN: f32 = 0.06;
const FOOD_PATCH_RADIUS_FRAC_MAX: f32 = 0.14;

/// A food cell counts as "covered" for the food-coverage metric when it holds more
/// than this much food — a small threshold that ignores near-empty cells.
const FOOD_COVERAGE_EPSILON: f32 = 0.02;

/// Default reachability threshold: a cell joins the network if its combined trail
/// (`field[0] + field[1]`) is at least this fraction of the current combined
/// field max. The network-cost / connectivity reduction flood-fills over cells at
/// or above this. Live-tunable via [`Sim::set_network_threshold`].
const DEFAULT_NETWORK_THRESHOLD: f32 = 0.05;

/// A persistent endpoint food source: a circular region (toroidal) whose
/// `food_cap` is pinned high, so it keeps regrowing and draws chemotactic agents.
/// Endpoints double as the seeds/targets of the reachability metric and as the
/// markers the renderer draws. Positions are in cells; `radius` is in cells.
#[derive(Clone, Copy, Debug)]
struct Endpoint {
    x: f32,
    y: f32,
    radius: f32,
}

/// Starting energy of every initial / spawned agent, as a fraction of the
/// reproduction threshold — high enough not to starve immediately, low enough not
/// to reproduce on tick one.
const INITIAL_ENERGY_FRAC: f32 = 0.6;
/// Heading jitter (radians) applied to a child relative to its parent at birth.
const BIRTH_HEADING_JITTER: f32 = 0.5;

/// Spawn pattern selector for [`Sim::spawn`]. Numeric values match the
/// `petri-wasm` integer API so the renderer can pass a plain `u32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnPattern {
    /// All agents at exactly `(x, y)`, random headings.
    Point,
    /// Agents on a thin ring around `(x, y)` (radius derived from field size).
    Ring,
    /// Agents filling a disk around `(x, y)` (uniform area density).
    UniformDisk,
}

impl SpawnPattern {
    /// Map the wire integer to a pattern. Unknown values fall back to `Point`.
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => SpawnPattern::Ring,
            2 => SpawnPattern::UniformDisk,
            _ => SpawnPattern::Point,
        }
    }
}

/// The default cross-sensing matrix: identity (each species senses only its own
/// trail with weight 1). A run under this matrix is bit-for-bit the pre-existing rule.
fn identity_cross_sense() -> [[f32; SPECIES]; SPECIES] {
    let mut m = [[0.0f32; SPECIES]; SPECIES];
    let mut s = 0;
    while s < SPECIES {
        m[s][s] = 1.0;
        s += 1;
    }
    m
}

/// The simulation world. Owns one trail `field` per species (each row-major,
/// `width * height`) that the renderer visualises, the shared `food` field both
/// species draw from, the struct-of-arrays agent population (tagged by species),
/// and the seeded PRNG that makes every run reproducible.
pub struct Sim {
    width: usize,
    height: usize,
    tick_count: u64,

    /// One trail field per species, each row-major `width * height`. Allocated
    /// once; never reallocated. A species senses and deposits only into its own.
    field: [Vec<f32>; SPECIES],
    /// Shared scratch buffer for the separable blur, reused for each species in
    /// turn. Same fixed size as a field.
    field_b: Vec<f32>,
    /// Max value of each species' field after the most recent field pass
    /// (per-species auto-exposure).
    field_max: [f32; SPECIES],

    /// Shared food field, row-major `width * height`. Both species eat from it; it
    /// regrows toward `food_cap`. Allocated once; never reallocated (JS aliases it).
    food: Vec<f32>,
    /// Static per-cell regrowth ceiling — the soft Gaussian "patches." Same fixed
    /// size as `food`; regenerated deterministically at `new`/`reset`.
    food_cap: Vec<f32>,
    /// Max food value after the most recent tick (for renderer normalization).
    food_max: f32,

    /// Obstacle mask, row-major `width * height`: `0` = open, `1` = wall. Allocated
    /// once at `new` (all-open) and never reallocated (JS aliases it for rendering).
    /// Walls block agent moves and read as zero trail at sensors. Painted/cleared
    /// through the obstacle API; `reset` clears it back to all-open.
    obstacles: Vec<u8>,
    /// Count of wall cells in `obstacles`. When `0`, every obstacle code path is
    /// skipped entirely, so an empty mask draws **zero** extra RNG and leaves a tick
    /// byte-identical to a build without obstacles (the determinism guard).
    obstacle_count: usize,
    /// Persistent endpoint food sources (targets of the reachability metric and the
    /// renderer's markers). Empty by default. Each pins `food_cap` high within its
    /// radius; changing them recomputes the caps.
    endpoints: Vec<Endpoint>,

    /// Per-species total trail mass (Σ of that species' field) after the most
    /// recent field pass. `f64` so summing all cells of a high-deposit field keeps
    /// full precision. Folded in during the field pass — no extra full-field scan.
    trail_mass: [f64; SPECIES],
    /// Total food remaining (Σ of `food`) after the most recent food pass. `f64`
    /// for the same precision reason. Accumulated during the food pass.
    food_total: f64,
    /// Fraction of cells with `food > FOOD_COVERAGE_EPSILON`, in `[0, 1]`, after
    /// the most recent food pass. Counted during the food pass.
    food_coverage: f32,

    // Agents, struct-of-arrays. Capacity reserved at `new`; never grown after.
    // Births push within capacity, deaths swap_remove — every array (including
    // `species`) is kept in lockstep on every push/swap_remove.
    x: Vec<f32>,
    y: Vec<f32>,
    heading: Vec<f32>,
    energy: Vec<f32>,
    /// Species tag in `0..SPECIES`, selecting which `params`/`ecology`/`field`
    /// each agent uses.
    species: Vec<u8>,

    rng: Rng,
    params: [Params; SPECIES],
    ecology: [Ecology; SPECIES],

    /// Signed cross-species sensing weights: `cross_sense[s][o]` is how strongly an
    /// agent of species `s` is pulled by species `o`'s trail when sampling a sensor.
    /// The trail value an agent of species `s` reads at a sensor cell is
    /// `Σ_o cross_sense[s][o] * field[o][cell]` — a positive off-diagonal attracts it
    /// to the other species' trail, a negative one repels it. The default is the
    /// identity (each species senses only its own trail with weight 1), which is the
    /// exact pre-existing rule and stays bit-for-bit identical.
    cross_sense: [[f32; SPECIES]; SPECIES],

    /// Reachability threshold as a fraction of the current combined `field_max`.
    /// Live-tunable; defaults to [`DEFAULT_NETWORK_THRESHOLD`].
    network_threshold: f32,
    /// Flood-fill visited/label scratch, `width * height`. Pre-allocated at `new`,
    /// reused by the on-demand reachability reduction — never grows, never touched
    /// inside `tick`. A cell holds `1` once the fill has visited it, else `0`.
    visited: Vec<u32>,
    /// Flood-fill BFS-queue scratch, `width * height` (a worst-case full frontier).
    /// Pre-allocated at `new`, reused by the reachability reduction.
    bfs_queue: Vec<u32>,
}

impl Sim {
    /// Allocate a `width × height` world, reserve [`DEFAULT_AGENT_CAPACITY`] agent
    /// slots, seed the PRNG, and spawn the default population (`≈ w*h/8` agents)
    /// split across the two species and seeded uniformly with random headings.
    pub fn new(width: usize, height: usize, seed: u64) -> Self {
        Self::with_capacity(width, height, seed, DEFAULT_AGENT_CAPACITY)
    }

    /// Like [`Sim::new`] but with an explicit agent capacity (the spawn cap).
    pub fn with_capacity(width: usize, height: usize, seed: u64, capacity: usize) -> Self {
        let cells = width * height;
        // The default population: a generous-but-not-saturating density.
        let initial = (cells / 8).min(capacity);

        let params = std::array::from_fn(Params::default_for);
        let ecology = std::array::from_fn(Ecology::default_for);

        let mut sim = Self {
            width,
            height,
            tick_count: 0,
            field: std::array::from_fn(|_| vec![0.0; cells]),
            field_b: vec![0.0; cells],
            field_max: [0.0; SPECIES],
            food: vec![0.0; cells],
            food_cap: vec![0.0; cells],
            food_max: 0.0,
            obstacles: vec![0u8; cells],
            obstacle_count: 0,
            endpoints: Vec::new(),
            trail_mass: [0.0; SPECIES],
            food_total: 0.0,
            food_coverage: 0.0,
            x: Vec::with_capacity(capacity),
            y: Vec::with_capacity(capacity),
            heading: Vec::with_capacity(capacity),
            energy: Vec::with_capacity(capacity),
            species: Vec::with_capacity(capacity),
            rng: Rng::seed(seed),
            params,
            ecology,
            cross_sense: identity_cross_sense(),
            network_threshold: DEFAULT_NETWORK_THRESHOLD,
            visited: vec![0u32; cells],
            bfs_queue: vec![0u32; cells],
        };
        // RNG draw order is load-bearing for determinism: patches first, then the
        // initial population. `reset` mirrors this exact order.
        sim.generate_food_patches();
        sim.spawn_initial(initial);
        sim
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }

    /// Total number of live agents across all species.
    pub fn agent_count(&self) -> usize {
        self.x.len()
    }

    /// Number of live agents tagged with species `s`. O(n); used for the HUD.
    pub fn species_population(&self, s: usize) -> usize {
        let tag = s as u8;
        self.species.iter().filter(|&&t| t == tag).count()
    }

    /// Reserved agent capacity (the spawn cap). Never changes after construction.
    pub fn capacity(&self) -> usize {
        self.x.capacity()
    }

    /// Read-only view of species `s`'s trail field (row-major,
    /// `len == width * height`). `petri-wasm` hands JS a zero-copy pointer into
    /// this buffer.
    pub fn field(&self, s: usize) -> &[f32] {
        &self.field[s]
    }

    /// Largest value of species `s`'s field after the most recent tick (for
    /// renderer auto-exposure). Computed during the field pass — `0.0` before the
    /// first tick.
    pub fn field_max(&self, s: usize) -> f32 {
        self.field_max[s]
    }

    /// Read-only view of the food field (row-major, `len == width * height`).
    /// `petri-wasm` hands JS a zero-copy pointer into this buffer.
    pub fn food(&self) -> &[f32] {
        &self.food
    }

    /// Largest food value after the most recent tick (for renderer normalization).
    /// Equal to the patch peak right after `new`/`reset`.
    pub fn food_max(&self) -> f32 {
        self.food_max
    }

    /// Total trail mass of species `s` — the `f64` sum of every cell in that
    /// species' field after the most recent tick. Unbounded (grows with deposit and
    /// population, shrinks with decay); `0.0` before the first tick. `s >= SPECIES`
    /// returns `0.0`.
    pub fn trail_mass(&self, s: usize) -> f64 {
        if s < SPECIES {
            self.trail_mass[s]
        } else {
            0.0
        }
    }

    /// Total food remaining — the `f64` sum of every cell in the shared food field
    /// after the most recent tick. Bounded above by `Σ food_cap` (set right after
    /// `new`/`reset`, when food starts at its cap).
    pub fn food_total(&self) -> f64 {
        self.food_total
    }

    /// Fraction of cells holding more than a small epsilon of food, in `[0, 1]`,
    /// after the most recent tick. A coarse "how much of the world is fed" gauge.
    pub fn food_coverage(&self) -> f32 {
        self.food_coverage
    }

    /// Current live Physarum parameters for species `s`.
    pub fn params(&self, s: usize) -> Params {
        self.params[s]
    }

    /// Replace the live Physarum parameters for species `s`. Takes effect on the
    /// next [`Sim::tick`]; no reallocation, no rebuild.
    pub fn set_params(&mut self, s: usize, params: Params) {
        self.params[s] = params;
    }

    /// Current live ecology parameters for species `s`.
    pub fn ecology(&self, s: usize) -> Ecology {
        self.ecology[s]
    }

    /// Replace the live ecology parameters for species `s`. Takes effect on the
    /// next [`Sim::tick`]; no reallocation, no rebuild.
    pub fn set_ecology(&mut self, s: usize, ecology: Ecology) {
        self.ecology[s] = ecology;
    }

    /// Current cross-sensing weight `cross_sense[species][other]` — how strongly an
    /// agent of `species` is pulled by `other`'s trail at a sensor. Out-of-range
    /// indices return `0.0`.
    pub fn cross_sense(&self, species: usize, other: usize) -> f32 {
        if species < SPECIES && other < SPECIES {
            self.cross_sense[species][other]
        } else {
            0.0
        }
    }

    /// Set the cross-sensing weight `cross_sense[species][other]`. Positive = the
    /// agent is attracted to `other`'s trail, negative = repelled. Takes effect on the
    /// next [`Sim::tick`]; no reallocation, no rebuild. Out-of-range indices are ignored.
    pub fn set_cross_sense(&mut self, species: usize, other: usize, weight: f32) {
        if species < SPECIES && other < SPECIES {
            self.cross_sense[species][other] = weight;
        }
    }

    /// Whether the cross-sensing matrix differs from the identity. When `false` (the
    /// default), the sense path uses the single-field fast path and a run is
    /// bit-for-bit identical to the pre-existing rule. Any off-diagonal `!= 0.0` or
    /// any diagonal `!= 1.0` flips this on.
    #[inline]
    fn cross_sense_active(&self) -> bool {
        for s in 0..SPECIES {
            for o in 0..SPECIES {
                let expected = if s == o { 1.0 } else { 0.0 };
                if self.cross_sense[s][o] != expected {
                    return true;
                }
            }
        }
        false
    }

    // --- Read-only inspection: point queries and per-agent getters. These never
    // allocate, never mutate, and store no query state. ---

    /// Value of species `s`'s trail field at the cell containing `(x, y)` (the
    /// continuous position is floored and wrapped toroidally, reusing the sim's own
    /// cell mapping). `s >= SPECIES` returns `0.0`.
    pub fn trail_at(&self, s: usize, x: f32, y: f32) -> f32 {
        if s < SPECIES {
            self.field[s][self.idx(x, y)]
        } else {
            0.0
        }
    }

    /// Value of the shared food field at the cell containing `(x, y)` (floored and
    /// wrapped toroidally, same cell mapping as the sim).
    pub fn food_at(&self, x: f32, y: f32) -> f32 {
        self.food[self.idx(x, y)]
    }

    /// Index of the live agent nearest to `(x, y)`, or `-1` if there are no agents.
    /// Distance is **toroidal** (wrapping at the field edges), measured against each
    /// agent's continuous position, so the nearest pick is consistent with the
    /// wrap-around world the agents live in. O(n) scan, no allocation.
    pub fn nearest_agent(&self, x: f32, y: f32) -> i32 {
        let n = self.x.len();
        if n == 0 {
            return -1;
        }
        let w = self.width as f32;
        let h = self.height as f32;
        let mut best = i32::MAX as usize; // sentinel, overwritten on first iter
        let mut best_d2 = f32::INFINITY;
        for i in 0..n {
            // Toroidal component distances: never more than half the field span.
            let mut dx = (self.x[i] - x).abs();
            if dx > w - dx {
                dx = w - dx;
            }
            let mut dy = (self.y[i] - y).abs();
            if dy > h - dy {
                dy = h - dy;
            }
            let d2 = dx * dx + dy * dy;
            if d2 < best_d2 {
                best_d2 = d2;
                best = i;
            }
        }
        best as i32
    }

    /// Species tag of agent `i`, or `0` if `i` is out of range. Indices are valid
    /// until the next `tick`/`spawn`/`reset` (which may add or remove agents).
    pub fn agent_species(&self, i: usize) -> usize {
        if i < self.species.len() {
            self.species[i] as usize
        } else {
            0
        }
    }

    /// Energy of agent `i`, or `0.0` if `i` is out of range.
    pub fn agent_energy(&self, i: usize) -> f32 {
        if i < self.energy.len() {
            self.energy[i]
        } else {
            0.0
        }
    }

    /// `x` position of agent `i` (cells, in `[0, width)`), or `0.0` if out of range.
    pub fn agent_x(&self, i: usize) -> f32 {
        if i < self.x.len() {
            self.x[i]
        } else {
            0.0
        }
    }

    /// `y` position of agent `i` (cells, in `[0, height)`), or `0.0` if out of range.
    pub fn agent_y(&self, i: usize) -> f32 {
        if i < self.y.len() {
            self.y[i]
        } else {
            0.0
        }
    }

    /// Re-seed the PRNG, clear every trail field, and respawn the default
    /// population — reusing the existing buffers (no reallocation, so the JS field
    /// views that alias linear memory are *not* detached by this call).
    pub fn reset(&mut self, seed: u64) {
        self.rng = Rng::seed(seed);
        self.tick_count = 0;
        self.field_max = [0.0; SPECIES];
        self.food_max = 0.0;
        self.trail_mass = [0.0; SPECIES];
        self.food_total = 0.0;
        self.food_coverage = 0.0;
        for f in self.field.iter_mut() {
            for v in f.iter_mut() {
                *v = 0.0;
            }
        }
        for v in self.field_b.iter_mut() {
            *v = 0.0;
        }
        // Geometry is world state: a reset returns to an empty, all-open world.
        // Clearing the mask back to zero restores byte-identity with a fresh `new`
        // (which has no obstacles), keeping `reset` a true world-reset.
        if self.obstacle_count != 0 {
            for v in self.obstacles.iter_mut() {
                *v = 0;
            }
            self.obstacle_count = 0;
        }
        self.endpoints.clear();
        // Cross-sensing is world state: a fresh world senses only own trails, so
        // restoring the identity keeps `reset` byte-identical to a fresh `new`.
        self.cross_sense = identity_cross_sense();
        self.x.clear();
        self.y.clear();
        self.heading.clear();
        self.energy.clear();
        self.species.clear();
        // Mirror `with_capacity`'s RNG draw order exactly: patches first (which
        // also refills `food` from the fresh caps), then the initial population.
        self.generate_food_patches();
        let initial = (self.width * self.height / 8).min(self.capacity());
        self.spawn_initial(initial);
    }

    /// Spawn `count` agents of species `s` about `(cx, cy)` in the given
    /// `pattern`, capped so the total never exceeds the reserved capacity (which
    /// would grow linear memory and detach the JS field views). Headings are
    /// random. Returns how many were actually added.
    pub fn spawn(
        &mut self,
        cx: f32,
        cy: f32,
        count: usize,
        pattern: SpawnPattern,
        s: usize,
    ) -> usize {
        let room = self.capacity() - self.agent_count();
        let n = count.min(room);
        let (w, h) = (self.width as f32, self.height as f32);
        // Ring/disk radius: a fraction of the smaller dimension — visible but contained.
        let radius = 0.45 * w.min(h);

        for _ in 0..n {
            let (px, py) = match pattern {
                SpawnPattern::Point => (cx, cy),
                SpawnPattern::Ring => {
                    let a = self.rng.range(TAU);
                    (cx + radius * a.cos(), cy + radius * a.sin())
                }
                SpawnPattern::UniformDisk => {
                    // sqrt(u) gives uniform area density (no center clustering).
                    let r = radius * self.rng.next_f32().sqrt();
                    let a = self.rng.range(TAU);
                    (cx + r * a.cos(), cy + r * a.sin())
                }
            };
            self.x.push(px.rem_euclid(w));
            self.y.push(py.rem_euclid(h));
            self.heading.push(self.rng.range(TAU));
            self.energy.push(self.initial_energy(s));
            self.species.push(s as u8);
        }
        n
    }

    /// Spawn the initial / reset population, split evenly across the species and
    /// seeded uniformly over the whole field so the species interleave and compete
    /// everywhere. Caps at capacity. The RNG draw order — for each agent in turn:
    /// `x`, `y`, `heading`, with species assigned round-robin and consuming no
    /// draws — is part of the determinism contract and is mirrored by `reset`.
    fn spawn_initial(&mut self, count: usize) {
        let room = self.capacity() - self.agent_count();
        let n = count.min(room);
        let (w, h) = (self.width as f32, self.height as f32);
        for i in 0..n {
            // Round-robin species assignment: even split, no RNG draw consumed.
            let s = i % SPECIES;
            self.x.push(self.rng.range(w));
            self.y.push(self.rng.range(h));
            self.heading.push(self.rng.range(TAU));
            self.energy.push(self.initial_energy(s));
            self.species.push(s as u8);
        }
    }

    /// Starting energy for an initial / spawned agent of species `s` — a fixed
    /// fraction of that species' reproduction threshold so agents neither starve
    /// nor reproduce at once.
    #[inline(always)]
    fn initial_energy(&self, s: usize) -> f32 {
        self.ecology[s].repro_threshold * INITIAL_ENERGY_FRAC
    }

    /// Generate `food_cap` as soft Gaussian patches at RNG-chosen centers/radii
    /// (toroidal distance), combined by `max` so peaks ≈ [`FOOD_PATCH_PEAK`] and
    /// the space between patches falls toward 0. Then refill `food = food_cap`
    /// (start abundant → first boom). Draws exactly `3` RNG values per patch
    /// (cx, cy, radius); this draw order is part of the determinism contract.
    fn generate_food_patches(&mut self) {
        let w = self.width;
        let h = self.height;
        let wf = w as f32;
        let hf = h as f32;
        let rmin = FOOD_PATCH_RADIUS_FRAC_MIN * wf.min(hf);
        let rmax = FOOD_PATCH_RADIUS_FRAC_MAX * wf.min(hf);

        for v in self.food_cap.iter_mut() {
            *v = 0.0;
        }

        for _ in 0..FOOD_PATCH_COUNT {
            let cx = self.rng.range(wf);
            let cy = self.rng.range(hf);
            let radius = rmin + self.rng.next_f32() * (rmax - rmin);
            let inv_two_sigma2 = 1.0 / (2.0 * radius * radius);

            for row in 0..h {
                // Toroidal vertical distance to the patch center.
                let dy_raw = (row as f32 + 0.5 - cy).abs();
                let dy = dy_raw.min(hf - dy_raw);
                let base = row * w;
                for col in 0..w {
                    let dx_raw = (col as f32 + 0.5 - cx).abs();
                    let dx = dx_raw.min(wf - dx_raw);
                    let g = FOOD_PATCH_PEAK * (-(dx * dx + dy * dy) * inv_two_sigma2).exp();
                    let cell = &mut self.food_cap[base + col];
                    if g > *cell {
                        *cell = g;
                    }
                }
            }
        }

        // Bake endpoint sources into the caps (combined by max), so each endpoint is
        // a persistent, full-strength food well its radius wide. No RNG: endpoints
        // are explicit geometry, not random patches.
        self.bake_endpoint_caps();

        // Start abundant: food begins at its cap so the first boom can run.
        self.food.copy_from_slice(&self.food_cap);
        let mut max = 0.0f32;
        let mut total = 0.0f64;
        let mut covered = 0usize;
        for &v in self.food.iter() {
            if v > max {
                max = v;
            }
            total += v as f64;
            if v > FOOD_COVERAGE_EPSILON {
                covered += 1;
            }
        }
        self.food_max = max;
        self.food_total = total;
        self.food_coverage = covered as f32 / self.food.len() as f32;
    }

    /// Max each endpoint's source into `food_cap`: a clamped Gaussian (toroidal
    /// distance) peaking at [`FOOD_PATCH_PEAK`] at the center and flooring to that
    /// same peak out to its `radius`, so an endpoint is a solid, persistent food
    /// well that survives depletion (its cap keeps regrowing it). No RNG — endpoints
    /// are explicit geometry. A no-op when there are no endpoints.
    fn bake_endpoint_caps(&mut self) {
        if self.endpoints.is_empty() {
            return;
        }
        let w = self.width;
        let h = self.height;
        let wf = w as f32;
        let hf = h as f32;
        for ep in self.endpoints.iter() {
            let radius = ep.radius.max(1.0);
            // Soft skirt just outside the core so the well blends into the field.
            let inv_two_sigma2 = 1.0 / (2.0 * radius * radius);
            for row in 0..h {
                let dy_raw = (row as f32 + 0.5 - ep.y).abs();
                let dy = dy_raw.min(hf - dy_raw);
                let base = row * w;
                for col in 0..w {
                    let dx_raw = (col as f32 + 0.5 - ep.x).abs();
                    let dx = dx_raw.min(wf - dx_raw);
                    let d2 = dx * dx + dy * dy;
                    // Solid core out to the radius, soft Gaussian skirt beyond.
                    let g = if d2 <= radius * radius {
                        FOOD_PATCH_PEAK
                    } else {
                        FOOD_PATCH_PEAK * (-(d2) * inv_two_sigma2).exp()
                    };
                    let cell = &mut self.food_cap[base + col];
                    if g > *cell {
                        *cell = g;
                    }
                }
            }
        }
    }

    /// Recompute `food_cap` from the current patches **and** endpoints and refill
    /// `food` to the fresh caps — the cap-recompute hook every geometry edit
    /// (`add_endpoint`/`clear_endpoints`) routes through. This re-runs the RNG-driven
    /// patch generation, so it re-seeds the patches from the current PRNG state; it
    /// is a world-geometry operation (reset-class), not a per-frame one.
    fn recompute_food_caps(&mut self) {
        self.generate_food_patches();
    }

    // --- Obstacle mask: paint/clear walls. In-place writes (the mask buffer is
    // fixed-size); `obstacle_count` tracks occupancy so an empty mask skips every
    // wall code path and stays byte-identical to a build with no obstacles. ---

    /// Read-only view of the obstacle mask (row-major, `len == width * height`,
    /// `0` = open, `1` = wall). `petri-wasm` hands JS a zero-copy pointer here.
    pub fn obstacles(&self) -> &[u8] {
        &self.obstacles
    }

    /// Number of wall cells currently set. `0` means the world is fully open and
    /// every obstacle code path is skipped (the determinism fast path).
    pub fn obstacle_count(&self) -> usize {
        self.obstacle_count
    }

    /// Paint (or erase) a filled disk of obstacle cells centered at `(x, y)` with
    /// the given `radius` (cells, toroidal). `on = true` sets walls, `false` clears
    /// them. In-place — no reallocation. Updates `obstacle_count`.
    pub fn paint_obstacle(&mut self, x: f32, y: f32, radius: f32, on: bool) {
        let w = self.width;
        let h = self.height;
        let wf = w as f32;
        let hf = h as f32;
        let r = radius.max(0.0);
        let r2 = r * r;
        let val: u8 = if on { 1 } else { 0 };
        for row in 0..h {
            let dy_raw = (row as f32 + 0.5 - y).abs();
            let dy = dy_raw.min(hf - dy_raw);
            let base = row * w;
            for col in 0..w {
                let dx_raw = (col as f32 + 0.5 - x).abs();
                let dx = dx_raw.min(wf - dx_raw);
                if dx * dx + dy * dy <= r2 {
                    let idx = base + col;
                    if self.obstacles[idx] != val {
                        if on {
                            self.obstacle_count += 1;
                            // Turning an open cell into a wall: zero the stale trail
                            // under it now, before the next field pass. Otherwise
                            // `blur_field_decay` would read the old value and diffuse
                            // hidden trail out of the wall into its neighbours for one
                            // tick, seeding the far side of a freshly-painted barrier.
                            for f in self.field.iter_mut() {
                                f[idx] = 0.0;
                            }
                        } else {
                            self.obstacle_count -= 1;
                        }
                        self.obstacles[idx] = val;
                    }
                }
            }
        }
    }

    /// Paint (or erase) an axis-aligned rectangle of obstacle cells. `x0..x1` and
    /// `y0..y1` are inclusive cell-coordinate ranges (auto-ordered). No wrap — used
    /// to lay maze walls. In-place; updates `obstacle_count`.
    fn paint_obstacle_rect(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, on: bool) {
        let w = self.width;
        let h = self.height;
        let (xa, xb) = (x0.min(x1), x0.max(x1).min(w - 1));
        let (ya, yb) = (y0.min(y1), y0.max(y1).min(h - 1));
        let val: u8 = if on { 1 } else { 0 };
        for row in ya..=yb {
            let base = row * w;
            for col in xa..=xb {
                let idx = base + col;
                if self.obstacles[idx] != val {
                    if on {
                        self.obstacle_count += 1;
                        // Zero stale trail under a newly-walled cell (see
                        // `paint_obstacle`): keeps the next blur from diffusing hidden
                        // trail out of the wall for one tick.
                        for f in self.field.iter_mut() {
                            f[idx] = 0.0;
                        }
                    } else {
                        self.obstacle_count -= 1;
                    }
                    self.obstacles[idx] = val;
                }
            }
        }
    }

    /// Clear every wall, returning the world to fully open. In-place.
    pub fn clear_obstacles(&mut self) {
        if self.obstacle_count != 0 {
            for v in self.obstacles.iter_mut() {
                *v = 0;
            }
            self.obstacle_count = 0;
        }
    }

    // --- Endpoint food sources: persistent wells that double as reachability
    // seeds/targets and renderer markers. Editing them recomputes the food caps. ---

    /// Add a persistent endpoint food source at `(x, y)` with the given `radius`
    /// (cells). Recomputes `food_cap`/`food` so the well takes effect immediately.
    /// Reset-class: it refills food and re-runs patch generation, so JS should
    /// re-fetch the food view afterward (the pointer is stable, but the contents
    /// change).
    pub fn add_endpoint(&mut self, x: f32, y: f32, radius: f32) {
        let w = self.width as f32;
        let h = self.height as f32;
        self.endpoints.push(Endpoint {
            x: x.rem_euclid(w),
            y: y.rem_euclid(h),
            radius: radius.max(1.0),
        });
        self.recompute_food_caps();
    }

    /// Remove every endpoint and recompute the food caps (back to patches only).
    pub fn clear_endpoints(&mut self) {
        if !self.endpoints.is_empty() {
            self.endpoints.clear();
            self.recompute_food_caps();
        }
    }

    /// Number of endpoint food sources.
    pub fn endpoint_count(&self) -> usize {
        self.endpoints.len()
    }

    /// `x` of endpoint `i` (cells), or `0.0` if out of range.
    pub fn endpoint_x(&self, i: usize) -> f32 {
        self.endpoints.get(i).map_or(0.0, |e| e.x)
    }

    /// `y` of endpoint `i` (cells), or `0.0` if out of range.
    pub fn endpoint_y(&self, i: usize) -> f32 {
        self.endpoints.get(i).map_or(0.0, |e| e.y)
    }

    /// Radius of endpoint `i` (cells), or `0.0` if out of range.
    pub fn endpoint_radius(&self, i: usize) -> f32 {
        self.endpoints.get(i).map_or(0.0, |e| e.radius)
    }

    // --- Chemotaxis + reachability knobs ---

    /// Reachability threshold as a fraction of the current combined `field_max`.
    pub fn network_threshold(&self) -> f32 {
        self.network_threshold
    }

    /// Set the reachability threshold (fraction of combined `field_max`). Clamped to
    /// a sane `[0, 1]` window; takes effect on the next reachability query.
    pub fn set_network_threshold(&mut self, t: f32) {
        self.network_threshold = t.clamp(0.0, 1.0);
    }

    /// Build a clean, reproducible "Physarum solves the maze" scenario from the
    /// current PRNG state: clear the random food patches, lay a simple wall maze
    /// into the obstacle mask, place two endpoint food wells in opposite corridors,
    /// turn on food-attraction for both species, and respawn the population between
    /// the endpoints so it must grow a connecting network through the maze.
    ///
    /// Reset-class — it rewrites obstacles, endpoints, food, and the agent arrays
    /// (all in place, no reallocation), so JS must re-fetch its field/food/obstacle
    /// views afterward (pointers stay valid; contents and the population change).
    pub fn load_maze_demo(&mut self) {
        let w = self.width;
        let h = self.height;
        let wf = w as f32;
        let hf = h as f32;

        // --- Fresh world: open mask, no endpoints, no random patches. ---
        self.clear_obstacles();
        self.endpoints.clear();
        self.tick_count = 0;
        self.field_max = [0.0; SPECIES];
        self.food_max = 0.0;
        self.trail_mass = [0.0; SPECIES];
        self.food_total = 0.0;
        self.food_coverage = 0.0;
        for f in self.field.iter_mut() {
            for v in f.iter_mut() {
                *v = 0.0;
            }
        }
        for v in self.field_b.iter_mut() {
            *v = 0.0;
        }
        // Caps start empty: the maze relies on the two endpoint wells only, so a
        // crashed front can't shelter in a random patch and ignore the maze.
        for v in self.food_cap.iter_mut() {
            *v = 0.0;
        }

        // --- Lay a simple maze: three vertical walls with offset gaps, forcing a
        // serpentine path between the left and right corridors. Wall thickness and
        // gaps scale with the field so the layout reproduces at any size. ---
        let t = (w / 64).max(1); // wall thickness in cells
        let gap = (h / 6).max(3); // corridor gap height
        let margin = h / 8; // keep gaps off the top/bottom edges

        // Seal the toroidal x=0↔width seam with a full-height wall (thickness `t`).
        // Movement and the reachability flood-fill both wrap, and without this seal a
        // straight horizontal trail across the open seam would connect the two end
        // wells without threading the serpentine gaps — defeating the demo. Both
        // endpoints (≈0.08·w / 0.92·w) and the spawn corridor (≈0.02·w) sit well to
        // the right of an `x∈[0, t)` wall, so this leaves them clear.
        self.paint_obstacle_rect(0, 0, t.saturating_sub(1), h - 1, true);

        let wall_xs = [w / 4, w / 2, (3 * w) / 4];
        for (k, &wx) in wall_xs.iter().enumerate() {
            let x0 = wx.saturating_sub(t / 2);
            let x1 = (wx + t / 2).min(w - 1);
            // Alternate the gap between the top third and the bottom third so the
            // single open path zig-zags.
            let gap_center = if k % 2 == 0 {
                margin + gap / 2
            } else {
                h - margin - gap / 2
            };
            let g0 = gap_center.saturating_sub(gap / 2);
            let g1 = (gap_center + gap / 2).min(h - 1);
            // Full-height wall, then carve the gap back open.
            self.paint_obstacle_rect(x0, 0, x1, h - 1, true);
            self.paint_obstacle_rect(x0, g0, x1, g1, false);
        }

        // --- Two endpoint wells in the far corridors (left edge / right edge),
        // each clear of the walls. ---
        let r = (wf.min(hf) * 0.05).max(3.0);
        self.endpoints.push(Endpoint {
            x: wf * 0.08,
            y: (margin + gap / 2) as f32,
            radius: r,
        });
        self.endpoints.push(Endpoint {
            x: wf * 0.92,
            y: (margin + gap / 2) as f32,
            radius: r,
        });
        self.bake_endpoint_caps();
        self.food.copy_from_slice(&self.food_cap);
        // Refresh the food summary so the metrics are consistent before the first tick.
        let mut fmax = 0.0f32;
        let mut ftotal = 0.0f64;
        let mut fcov = 0usize;
        for &v in self.food.iter() {
            if v > fmax {
                fmax = v;
            }
            ftotal += v as f64;
            if v > FOOD_COVERAGE_EPSILON {
                fcov += 1;
            }
        }
        self.food_max = fmax;
        self.food_total = ftotal;
        self.food_coverage = fcov as f32 / self.food.len() as f32;

        // --- Chemotaxis on for both species so they navigate toward the wells. ---
        for s in 0..SPECIES {
            self.params[s].food_attraction = 6.0;
        }

        // --- Pre-grow the mold: blanket the whole maze. The canonical "Physarum
        // solves the maze" setup starts with the colony filling every open cell, so a
        // connected network already threads all the serpentine gaps; the interesting
        // dynamics are the *pruning* that follows as chemotaxis pulls trail toward the
        // two wells and the food-starved detours fade. Seeding only the left corridor
        // never connects — agents that wander into the food-less middle starve before
        // laying a trail through to the far well. Agents are seeded uniformly across
        // all open (non-wall) cells of the maze by rejection-sampling from the sim
        // PRNG in a fixed order (the determinism contract). The species alternate
        // round-robin (no RNG draw), matching `spawn_initial`. ---
        self.x.clear();
        self.y.clear();
        self.heading.clear();
        self.energy.clear();
        self.species.clear();
        let count = (w * h / 8).min(self.capacity());
        for i in 0..count {
            let s = i % SPECIES;
            // Rejection-sample an open cell anywhere in the maze. A bounded retry keeps
            // the RNG draw count finite and deterministic; the fallback is endpoint 0
            // (always open). Each attempt draws (x, y) so the draw count is fixed.
            let mut px = self.endpoint_x(0);
            let mut py = self.endpoint_y(0);
            for _ in 0..16 {
                let tx = self.rng.next_f32() * wf;
                let ty = self.rng.next_f32() * hf;
                if self.obstacles[self.idx(tx, ty)] == 0 {
                    px = tx;
                    py = ty;
                    break;
                }
            }
            self.x.push(px.rem_euclid(wf));
            self.y.push(py.rem_euclid(hf));
            self.heading.push(self.rng.range(TAU));
            self.energy.push(self.initial_energy(s));
            self.species.push(s as u8);
        }
    }

    /// Reachability reduction over the **combined** trail field. Floods (4-connected,
    /// toroidal — matching the sim's wrap) over open (non-wall) cells whose combined
    /// trail `field[0] + field[1]` is at least `network_threshold * max_combined`,
    /// seeded from the cells under endpoint 0. Fills the `visited` scratch as a side
    /// effect. Returns `(endpoints_connected, network_cost)`:
    /// - `endpoints_connected`: how many endpoints (including endpoint 0) have at
    ///   least one cell inside their radius in the visited set; `0` if no endpoints.
    /// - `network_cost`: number of visited cells (a length/cost proxy).
    ///
    /// Read-only: no RNG, no state mutation beyond the pre-allocated scratch, never
    /// called inside `tick`. Allocation-free (reuses `visited` + `bfs_queue`).
    fn reachability(&mut self) -> (u32, u32) {
        let w = self.width;
        let h = self.height;
        let n = w * h;

        // No endpoints → nothing to connect.
        if self.endpoints.is_empty() {
            return (0, 0);
        }

        // Combined field max → absolute threshold.
        let mut max_combined = 0.0f32;
        for k in 0..n {
            let v = self.field[0][k] + self.field[1][k];
            if v > max_combined {
                max_combined = v;
            }
        }
        // An empty (or all-zero) field has no network at all — bail before the fill
        // so a zero threshold doesn't sweep in every open cell.
        if max_combined <= 0.0 {
            for v in self.visited.iter_mut() {
                *v = 0;
            }
            return (0, 0);
        }
        // Strictly-positive floor so a literal-0 threshold still excludes empty cells.
        let thresh = (self.network_threshold * max_combined).max(f32::MIN_POSITIVE);

        // Reset visited scratch.
        for v in self.visited.iter_mut() {
            *v = 0;
        }

        // Seed the queue from open, above-threshold cells under endpoint 0.
        let mut head = 0usize;
        let mut tail = 0usize;
        {
            let ep = self.endpoints[0];
            let r2 = ep.radius * ep.radius;
            let wf = w as f32;
            let hf = h as f32;
            for row in 0..h {
                let dy_raw = (row as f32 + 0.5 - ep.y).abs();
                let dy = dy_raw.min(hf - dy_raw);
                let base = row * w;
                for col in 0..w {
                    let dx_raw = (col as f32 + 0.5 - ep.x).abs();
                    let dx = dx_raw.min(wf - dx_raw);
                    if dx * dx + dy * dy > r2 {
                        continue;
                    }
                    let idx = base + col;
                    if self.obstacles[idx] != 0 {
                        continue;
                    }
                    if self.field[0][idx] + self.field[1][idx] < thresh {
                        continue;
                    }
                    if self.visited[idx] == 0 {
                        self.visited[idx] = 1;
                        self.bfs_queue[tail] = idx as u32;
                        tail += 1;
                    }
                }
            }
        }

        // 4-connected toroidal flood fill over open, above-threshold cells.
        let mut cost = tail as u32;
        while head < tail {
            let idx = self.bfs_queue[head] as usize;
            head += 1;
            let col = idx % w;
            let row = idx / w;
            // Toroidal 4-neighborhood.
            let left = if col == 0 { w - 1 } else { col - 1 };
            let right = if col == w - 1 { 0 } else { col + 1 };
            let up = if row == 0 { h - 1 } else { row - 1 };
            let down = if row == h - 1 { 0 } else { row + 1 };
            let neighbors = [
                row * w + left,
                row * w + right,
                up * w + col,
                down * w + col,
            ];
            for &nidx in neighbors.iter() {
                if self.visited[nidx] != 0 {
                    continue;
                }
                if self.obstacles[nidx] != 0 {
                    continue;
                }
                if self.field[0][nidx] + self.field[1][nidx] < thresh {
                    continue;
                }
                self.visited[nidx] = 1;
                self.bfs_queue[tail] = nidx as u32;
                tail += 1;
                cost += 1;
            }
        }

        // Count how many endpoints have a visited cell inside their radius.
        let wf = w as f32;
        let hf = h as f32;
        let mut connected = 0u32;
        for ep in self.endpoints.iter() {
            let r2 = ep.radius * ep.radius;
            let mut hit = false;
            'scan: for row in 0..h {
                let dy_raw = (row as f32 + 0.5 - ep.y).abs();
                let dy = dy_raw.min(hf - dy_raw);
                let base = row * w;
                for col in 0..w {
                    let dx_raw = (col as f32 + 0.5 - ep.x).abs();
                    let dx = dx_raw.min(wf - dx_raw);
                    if dx * dx + dy * dy <= r2 && self.visited[base + col] != 0 {
                        hit = true;
                        break 'scan;
                    }
                }
            }
            if hit {
                connected += 1;
            }
        }
        (connected, cost)
    }

    /// How many endpoints (including endpoint 0) are reachable from endpoint 0 along
    /// the thresholded combined trail network (open cells only, toroidal). `0` if
    /// there are no endpoints. Backed by the private `reachability` reduction. On-demand,
    /// read-only.
    pub fn endpoints_connected(&mut self) -> u32 {
        self.reachability().0
    }

    /// Number of cells in the reachable network from endpoint 0 — a length/cost
    /// proxy for the connecting structure. `0` if there are no endpoints or no
    /// network above threshold. On-demand, read-only.
    pub fn network_cost(&mut self) -> u32 {
        self.reachability().1
    }

    /// Advance the simulation one tick: every agent senses, steers, moves
    /// (toroidal wrap), eats, deposits, and pays its metabolic cost; well-fed
    /// agents reproduce and starved agents die (returning food); then the trail
    /// field diffuses (separable blur) and decays, and the food field regrows
    /// toward its caps. No allocation occurs in here.
    pub fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        self.agent_pass();
        self.field_pass();
        self.food_pass();
    }

    /// Per-agent ecology + Physarum pass, in three phases over a snapshot of the
    /// pre-tick population `n`. Each agent uses its own species' `params`/`ecology`,
    /// senses and deposits only into its own species' `field`, and eats the
    /// **shared** `food` — the sole coupling between species.
    /// 1. Move/sense/steer/deposit (Physarum) + metabolism + eat.
    /// 2. Reproduce (well-fed agents split; the child inherits the species).
    /// 3. Death sweep (starved agents return food and are `swap_remove`d).
    #[inline]
    fn agent_pass(&mut self) {
        let w = self.width as f32;
        let h = self.height as f32;
        let n = self.x.len();
        // Geometry / cross-sensing fast-path flags: when all are off, every branch
        // below collapses to exactly today's arithmetic and RNG draw order
        // (byte-identity guard). `cross` is a whole-tick constant — the matrix can't
        // change mid-pass — so hoist it out of the per-agent loop.
        let has_walls = self.obstacle_count != 0;
        let cross = self.cross_sense_active();

        // --- Phase 1: sense → steer → move → eat → deposit, pay metabolism. ---
        for i in 0..n {
            let s = self.species[i] as usize;
            let p = self.params[s];
            let e = self.ecology[s];
            let heading = self.heading[i];
            let cx = self.x[i];
            let cy = self.y[i];

            // 1. Sense this species' own field at three points at sensor_distance:
            //    center, left, right. With obstacles present, a sensor over a wall
            //    reads 0 (no trail). With food-attraction on, each reading gains
            //    `food_attraction * food` at the sensor (steer up the food gradient).
            let attract = p.food_attraction;
            let (f, l, r) = if !has_walls && attract == 0.0 && !cross {
                // Default path — bit-for-bit identical to the pre-geography rule.
                (
                    self.sense(s, cx, cy, heading, p.sensor_distance),
                    self.sense(s, cx, cy, heading - p.sensor_angle, p.sensor_distance),
                    self.sense(s, cx, cy, heading + p.sensor_angle, p.sensor_distance),
                )
            } else if cross {
                // Cross-sensing path: each sensor reads the weighted sum of every
                // species' trail (the signed `cross_sense` row), plus the food term.
                (
                    self.sense_cross(s, cx, cy, heading, p.sensor_distance, attract, has_walls),
                    self.sense_cross(
                        s,
                        cx,
                        cy,
                        heading - p.sensor_angle,
                        p.sensor_distance,
                        attract,
                        has_walls,
                    ),
                    self.sense_cross(
                        s,
                        cx,
                        cy,
                        heading + p.sensor_angle,
                        p.sensor_distance,
                        attract,
                        has_walls,
                    ),
                )
            } else {
                (
                    self.sense_full(s, cx, cy, heading, p.sensor_distance, attract, has_walls),
                    self.sense_full(
                        s,
                        cx,
                        cy,
                        heading - p.sensor_angle,
                        p.sensor_distance,
                        attract,
                        has_walls,
                    ),
                    self.sense_full(
                        s,
                        cx,
                        cy,
                        heading + p.sensor_angle,
                        p.sensor_distance,
                        attract,
                        has_walls,
                    ),
                )
            };

            // 2. Steer (Jones rule, exactly as in docs/DESIGN.md).
            let new_heading = if f >= l && f >= r {
                heading // forward is best → keep heading
            } else if f < l && f < r {
                // forward worst → turn a random direction
                if self.rng.next_f32() < 0.5 {
                    heading - p.rotation_angle
                } else {
                    heading + p.rotation_angle
                }
            } else if l > r {
                heading - p.rotation_angle // turn left
            } else {
                heading + p.rotation_angle // turn right (r > l)
            };

            // 3. Move along the (new) heading, wrap toroidally to [0,W)×[0,H).
            let mut nx = cx + new_heading.cos() * p.step_size;
            let mut ny = cy + new_heading.sin() * p.step_size;
            nx = nx.rem_euclid(w);
            ny = ny.rem_euclid(h);

            // With walls present: a move into a wall cell is blocked — the agent
            // stays put and reorients via ONE RNG draw. The RNG is drawn ONLY when
            // actually blocked, so an empty mask consumes zero extra draws.
            if has_walls && self.obstacles[self.idx(nx, ny)] != 0 {
                nx = cx;
                ny = cy;
                let turn = if self.rng.next_f32() < 0.5 {
                    -p.rotation_angle
                } else {
                    p.rotation_angle
                };
                self.heading[i] = new_heading + turn;
            } else {
                self.heading[i] = new_heading;
            }
            self.x[i] = nx;
            self.y[i] = ny;

            let idx = self.idx(nx, ny);

            // 4. Pay the cost of living, then eat from the shared cell (1:1 food→energy).
            let eaten = self.food[idx].min(e.eat_rate);
            self.food[idx] -= eaten;
            self.energy[i] = self.energy[i] - e.metabolism + eaten;

            // 5. Deposit trail into this species' own field at the agent's new cell.
            self.field[s][idx] += p.deposit_amount;
        }

        // --- Phase 2: reproduce. Children land at indices ≥ n (not processed
        // this tick) and inherit the parent's species. Capacity is reserved at
        // `new`, so `push` never reallocates. ---
        let cap = self.capacity();
        for i in 0..n {
            let s = self.species[i] as usize;
            if self.energy[i] >= self.ecology[s].repro_threshold && self.x.len() < cap {
                let child_energy = self.energy[i] * 0.5;
                self.energy[i] = child_energy;
                let jitter = (self.rng.next_f32() - 0.5) * 2.0 * BIRTH_HEADING_JITTER;
                self.x.push(self.x[i]);
                self.y.push(self.y[i]);
                self.heading.push(self.heading[i] + jitter);
                self.energy.push(child_energy);
                self.species.push(s as u8);
            }
        }

        // --- Phase 3: death sweep. Starved agents return their species'
        // `death_return` to the shared food cell and are removed by `swap_remove`
        // (O(1), realloc-free) from all five arrays. Don't advance `i` after a
        // removal — the swapped-in tail agent now sits there. ---
        let mut i = 0;
        while i < self.x.len() {
            if self.energy[i] <= 0.0 {
                let s = self.species[i] as usize;
                let idx = self.idx(self.x[i], self.y[i]);
                self.food[idx] += self.ecology[s].death_return;
                self.x.swap_remove(i);
                self.y.swap_remove(i);
                self.heading.swap_remove(i);
                self.energy.swap_remove(i);
                self.species.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Food field regrowth: each cell relaxes toward its static cap by the shared
    /// regrow rate. The food field is shared, so the effective rate is the mean of
    /// the species' `food_regrow` knobs — each species still influences the shared
    /// cycle period, and the renderer can tune either one live. O(cells), no
    /// allocation. Tracks `food_max` plus the `food_total` / `food_coverage`
    /// metrics, all folded into this single cell loop.
    #[inline]
    fn food_pass(&mut self) {
        let mut sum = 0.0f32;
        for e in self.ecology.iter() {
            sum += e.food_regrow;
        }
        let regrow = sum / SPECIES as f32;
        let mut max = 0.0f32;
        let mut total = 0.0f64;
        let mut covered = 0usize;
        for k in 0..self.food.len() {
            let v = self.food[k] + regrow * (self.food_cap[k] - self.food[k]);
            self.food[k] = v;
            if v > max {
                max = v;
            }
            total += v as f64;
            if v > FOOD_COVERAGE_EPSILON {
                covered += 1;
            }
        }
        self.food_max = max;
        self.food_total = total;
        self.food_coverage = covered as f32 / self.food.len() as f32;
    }

    /// Sample species `s`'s field at `distance` cells ahead along `angle` from
    /// `(x, y)`, with toroidal wrap. Nearest-sample (cheaper than bilinear; the
    /// spec allows it). This is the pure self-trail sensor; the geography-aware
    /// variant is [`Sim::sense_full`].
    #[inline(always)]
    fn sense(&self, s: usize, x: f32, y: f32, angle: f32, distance: f32) -> f32 {
        let sx = x + angle.cos() * distance;
        let sy = y + angle.sin() * distance;
        self.field[s][self.idx(sx, sy)]
    }

    /// Geography-aware sensor: like [`Sim::sense`], but a sample over a wall cell
    /// reads `0` (fully masked-out geography — neither trail nor food, when
    /// `has_walls`), and the food field at the sensor adds `attract * food` to the
    /// reading (when `attract != 0`) so the agent steers up the food gradient. Only
    /// called on the obstacle/attraction path; the default path uses [`Sim::sense`]
    /// for byte-identity.
    // A flat argument list mirrors the hot-loop call site (no struct churn per agent).
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn sense_full(
        &self,
        s: usize,
        x: f32,
        y: f32,
        angle: f32,
        distance: f32,
        attract: f32,
        has_walls: bool,
    ) -> f32 {
        let sx = x + angle.cos() * distance;
        let sy = y + angle.sin() * distance;
        let idx = self.idx(sx, sy);
        // A wall cell is blocked geography: it emits no signal at all, so neither the
        // trail nor the food there should pull an agent toward it (a random food patch
        // or an endpoint skirt can overlap a wall). Mask the whole sample to 0.
        if has_walls && self.obstacles[idx] != 0 {
            return 0.0;
        }
        let trail = self.field[s][idx];
        if attract != 0.0 {
            trail + attract * self.food[idx]
        } else {
            trail
        }
    }

    /// Geography- and cross-species-aware sensor for species `s`. Samples the cell
    /// `distance` ahead along `angle`. A wall cell (when `has_walls`) reads `0` (masked
    /// geography — neither trail nor food). Otherwise the reading is the weighted sum of
    /// every species' trail at that cell, `Σ_o cross_sense[s][o] * field[o][cell]`, plus
    /// the food-attraction term `attract * food[cell]` (when `attract != 0`). Used only
    /// when the cross-sensing matrix is non-identity; the no-cross paths use
    /// [`Sim::sense`] / [`Sim::sense_full`] for byte-identity.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn sense_cross(
        &self,
        s: usize,
        x: f32,
        y: f32,
        angle: f32,
        distance: f32,
        attract: f32,
        has_walls: bool,
    ) -> f32 {
        let sx = x + angle.cos() * distance;
        let sy = y + angle.sin() * distance;
        let idx = self.idx(sx, sy);
        if has_walls && self.obstacles[idx] != 0 {
            return 0.0;
        }
        let mut trail = 0.0f32;
        for o in 0..SPECIES {
            let w = self.cross_sense[s][o];
            if w != 0.0 {
                trail += w * self.field[o][idx];
            }
        }
        if attract != 0.0 {
            trail + attract * self.food[idx]
        } else {
            trail
        }
    }

    /// Map a continuous `(x, y)` to a wrapped, row-major field index.
    #[inline(always)]
    fn idx(&self, x: f32, y: f32) -> usize {
        let w = self.width;
        let h = self.height;
        // floor + wrap into [0, w) / [0, h).
        let mut ix = (x.floor() as isize).rem_euclid(w as isize) as usize;
        let mut iy = (y.floor() as isize).rem_euclid(h as isize) as usize;
        // Guard against the f32→isize edge where x == w exactly after wrap.
        if ix >= w {
            ix = w - 1;
        }
        if iy >= h {
            iy = h - 1;
        }
        iy * w + ix
    }

    /// Field update for every species: separable blur (horizontal then vertical,
    /// toroidal) of each species' field through the shared scratch buffer, multiply
    /// by that species' `decay`, and record its `field_max` plus its total
    /// `trail_mass`. The shared `field_b` is reused sequentially across species
    /// (each species is fully blurred before the next). Double-buffered — the
    /// species' own field is written in place from the scratch buffer.
    #[inline]
    fn field_pass(&mut self) {
        let w = self.width;
        let h = self.height;
        let has_walls = self.obstacle_count != 0;
        for s in 0..SPECIES {
            let center = self.params[s].diffuse_weight;
            let decay = self.params[s].decay;
            // Split-borrow the species' field and the shared scratch as disjoint
            // mutable/immutable slices for the helper.
            let field = &mut self.field[s];
            let scratch = &mut self.field_b;
            let (max, mass) = blur_field_decay(field, scratch, w, h, center, decay);
            if has_walls {
                // Force trail under walls to 0 so the blur can't bleed signal into
                // (or read trail out of) wall cells, then recompute max/mass over the
                // masked field. Only runs when obstacles exist — the empty-mask path
                // keeps the original `(max, mass)` untouched (byte-identity guard).
                let (mmax, mmass) = mask_walls(field, &self.obstacles);
                self.field_max[s] = mmax;
                self.trail_mass[s] = mmass;
            } else {
                self.field_max[s] = max;
                self.trail_mass[s] = mass;
            }
        }
    }
}

/// Zero every `field` cell whose `obstacles` cell is a wall, then return the
/// `(max, total)` of the masked field. Only invoked when obstacles are present.
/// `obstacles` must be the same length as `field`. Pure and allocation-free.
#[inline]
fn mask_walls(field: &mut [f32], obstacles: &[u8]) -> (f32, f64) {
    let mut max = 0.0f32;
    let mut total = 0.0f64;
    for (k, v) in field.iter_mut().enumerate() {
        if obstacles[k] != 0 {
            *v = 0.0;
        } else {
            let val = *v;
            if val > max {
                max = val;
            }
            total += val as f64;
        }
    }
    (max, total)
}

/// Separable 3-tap blur of `field` (horizontal then vertical, toroidal) through
/// `scratch`, multiplied by `decay`, written back in place into `field`. Returns
/// `(max, total)` of the post-pass field — the largest cell and the `f64`-summed
/// trail mass, both computed in the final vertical pass. `scratch` must be the same
/// length as `field` (`w * h`); its contents are overwritten. Pure and
/// allocation-free.
#[inline]
fn blur_field_decay(
    field: &mut [f32],
    scratch: &mut [f32],
    w: usize,
    h: usize,
    center: f32,
    decay: f32,
) -> (f32, f64) {
    let side = (1.0 - center) * 0.5;

    // Horizontal blur: field -> scratch (row-wise, wrap at column edges).
    for row in 0..h {
        let base = row * w;
        for col in 0..w {
            let left = if col == 0 { w - 1 } else { col - 1 };
            let right = if col == w - 1 { 0 } else { col + 1 };
            scratch[base + col] =
                field[base + left] * side + field[base + col] * center + field[base + right] * side;
        }
    }

    // Vertical blur: scratch -> field (column-wise, wrap at row edges), apply
    // decay in the same pass, and track the max plus the f64-summed total.
    let mut max = 0.0f32;
    let mut total = 0.0f64;
    for row in 0..h {
        let up = if row == 0 { h - 1 } else { row - 1 };
        let down = if row == h - 1 { 0 } else { row + 1 };
        let base = row * w;
        let base_up = up * w;
        let base_down = down * w;
        for col in 0..w {
            let v = (scratch[base_up + col] * side
                + scratch[base + col] * center
                + scratch[base_down + col] * side)
                * decay;
            field[base + col] = v;
            if v > max {
                max = v;
            }
            total += v as f64;
        }
    }
    (max, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cheap, order-sensitive checksum of a field for determinism guards.
    fn checksum(field: &[f32]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
        for &v in field {
            h ^= v.to_bits() as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
        }
        h
    }

    /// Combined checksum over every species' trail field — folds each field's
    /// checksum into a running FNV hash so a change in either field is detected.
    fn checksum_all(sim: &Sim) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for s in 0..SPECIES {
            h ^= checksum(sim.field(s));
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = Rng::seed(12345);
        let mut b = Rng::seed(12345);
        for _ in 0..1000 {
            let va = a.next_f32();
            assert_eq!(va, b.next_f32());
            assert!((0.0..1.0).contains(&va), "rng out of range: {va}");
        }
        // Different seeds diverge.
        let mut c = Rng::seed(54321);
        assert_ne!(Rng::seed(12345).next_u64(), c.next_u64());
    }

    #[test]
    fn new_spawns_default_population() {
        let sim = Sim::new(256, 256, 1);
        let total = 256 * 256 / 8;
        for s in 0..SPECIES {
            assert_eq!(sim.field(s).len(), 256 * 256);
            assert_eq!(sim.field_max(s), 0.0); // no tick yet
        }
        assert_eq!(sim.agent_count(), total);
        // The population is split evenly across the species (round-robin).
        let per = sim.species_population(0);
        assert!(
            per == total / SPECIES || per == total / SPECIES + 1,
            "species 0 population {per} not ~half of {total}"
        );
        let sum: usize = (0..SPECIES).map(|s| sim.species_population(s)).sum();
        assert_eq!(sum, total, "species populations must sum to the total");
    }

    #[test]
    fn agents_and_fields_stay_in_bounds() {
        let mut sim = Sim::new(128, 96, 7);
        for _ in 0..50 {
            sim.tick();
        }
        let (w, h) = (sim.width() as f32, sim.height() as f32);
        for i in 0..sim.agent_count() {
            assert!(
                (0.0..w).contains(&sim.x[i]),
                "x out of bounds: {}",
                sim.x[i]
            );
            assert!(
                (0.0..h).contains(&sim.y[i]),
                "y out of bounds: {}",
                sim.y[i]
            );
            assert!(
                (sim.species[i] as usize) < SPECIES,
                "species tag out of range: {}",
                sim.species[i]
            );
            assert!(sim.energy[i].is_finite(), "energy not finite");
        }
        for s in 0..SPECIES {
            for &v in sim.field(s) {
                assert!(v.is_finite() && v >= 0.0, "bad field value: {v}");
            }
            assert!(
                sim.field_max(s) > 0.0,
                "species {s} field should have signal after ticks"
            );
        }
    }

    #[test]
    fn spawn_caps_at_capacity() {
        let mut sim = Sim::with_capacity(64, 64, 3, 1000);
        // Default population already filled it (64*64/8 = 512), 488 slots left.
        let before = sim.agent_count();
        let added = sim.spawn(32.0, 32.0, 100_000, SpawnPattern::UniformDisk, 1);
        assert_eq!(sim.agent_count(), sim.capacity());
        assert_eq!(added, sim.capacity() - before);
        // A second spawn adds nothing — already full.
        assert_eq!(sim.spawn(0.0, 0.0, 10, SpawnPattern::Point, 0), 0);
    }

    #[test]
    fn no_allocation_after_new() {
        // Capacity (and therefore the field/food/agent pointers) must not change
        // across ticks, spawns, or resets — this is the zero-copy invariant.
        let mut sim = Sim::new(96, 96, 99);
        let cap = sim.capacity();
        let energy_cap = sim.energy.capacity();
        let species_cap = sim.species.capacity();
        let field_ptrs: [_; SPECIES] = std::array::from_fn(|s| sim.field(s).as_ptr());
        let food_ptr = sim.food().as_ptr();
        for _ in 0..30 {
            sim.tick();
        }
        sim.spawn(48.0, 48.0, 500, SpawnPattern::Ring, 0);
        sim.spawn(48.0, 48.0, 500, SpawnPattern::Ring, 1);
        sim.reset(99);
        for _ in 0..30 {
            sim.tick();
        }
        assert_eq!(sim.capacity(), cap, "agent capacity must not change");
        assert_eq!(
            sim.energy.capacity(),
            energy_cap,
            "energy capacity must not change"
        );
        assert_eq!(
            sim.species.capacity(),
            species_cap,
            "species capacity must not change"
        );
        for (s, &ptr) in field_ptrs.iter().enumerate() {
            assert_eq!(
                sim.field(s).as_ptr(),
                ptr,
                "species {s} field buffer must not reallocate"
            );
        }
        assert_eq!(
            sim.food().as_ptr(),
            food_ptr,
            "food buffer must not reallocate"
        );
    }

    #[test]
    fn ecology_values_stay_bounded() {
        let mut sim = Sim::new(128, 128, 42);
        let cap = sim.food_max(); // patch peak at start
        for _ in 0..1500 {
            sim.tick();
        }
        let (w, h) = (sim.width() as f32, sim.height() as f32);
        for i in 0..sim.agent_count() {
            assert!(sim.energy[i].is_finite(), "energy not finite");
            assert!(
                (0.0..w).contains(&sim.x[i]),
                "x out of bounds: {}",
                sim.x[i]
            );
            assert!(
                (0.0..h).contains(&sim.y[i]),
                "y out of bounds: {}",
                sim.y[i]
            );
        }
        // Food cannot exceed its cap by more than a death-return deposit, and is
        // never negative or non-finite. `death_return` can briefly push a single
        // cell above its cap, so allow a small slack above the patch peak (use the
        // larger of the species' death-return deposits).
        let max_death_return = (0..SPECIES)
            .map(|s| sim.ecology(s).death_return)
            .fold(0.0f32, f32::max);
        let upper = cap + max_death_return + 1.0;
        for &v in sim.food() {
            assert!(v.is_finite(), "food not finite: {v}");
            assert!(
                (0.0..=upper).contains(&v),
                "food out of range: {v} (cap≈{cap})"
            );
        }
        assert!(sim.food_max().is_finite() && sim.food_max() >= 0.0);
    }

    #[test]
    fn boom_bust_cycle_recovers() {
        // A real boom/bust: population rises above its start, later crashes well
        // below a prior peak, and never goes extinct (survivors in rich patches
        // rebound). Asserted on the default ecology params.
        let mut sim = Sim::new(256, 256, 7);
        let start = sim.agent_count();
        let mut peak = start;
        let mut crash_after_peak = usize::MAX;
        let mut min_alltime = start;
        for _ in 0..3000 {
            sim.tick();
            let n = sim.agent_count();
            if n > peak {
                peak = n;
                crash_after_peak = n; // reset trough tracking once we set a new peak
            } else if n < crash_after_peak {
                crash_after_peak = n;
            }
            if n < min_alltime {
                min_alltime = n;
            }
            assert!(n > 0, "population went extinct (no rebound possible)");
        }
        assert!(peak > start, "no boom: peak {peak} not above start {start}");
        // The crash dropped to well below the peak (a genuine bust, not a plateau).
        assert!(
            (crash_after_peak as f32) < 0.6 * (peak as f32),
            "no bust: trough {crash_after_peak} not well below peak {peak}"
        );
        assert!(min_alltime > 0, "population must stay positive throughout");
    }

    /// The determinism guard: a fixed seed must reproduce identical trail fields
    /// (combined checksum over both species) after N ticks, both across two
    /// independent sims and against a pinned golden value. If the rule changes,
    /// update `GOLDEN_CHECKSUM` deliberately.
    #[test]
    fn tick_is_deterministic() {
        const GOLDEN_SEED: u64 = 0xABCD_1234;
        const TICKS: usize = 60;

        let run = || {
            let mut sim = Sim::new(128, 128, GOLDEN_SEED);
            for _ in 0..TICKS {
                sim.tick();
            }
            let maxes: [u32; SPECIES] = std::array::from_fn(|s| sim.field_max(s).to_bits());
            (checksum_all(&sim), maxes)
        };

        let (sum_a, max_a) = run();
        let (sum_b, max_b) = run();
        assert_eq!(sum_a, sum_b, "two runs of the same seed must match");
        assert_eq!(max_a, max_b, "per-species field_max must match across runs");

        // Pinned golden checksum: this guards against accidental rule drift.
        // It is implementation-defined, not externally meaningful — if you change
        // the rule on purpose, run the test once and paste the new value here.
        const GOLDEN_CHECKSUM: u64 = 0x8de7_8e52_803c_7618;
        assert_eq!(
            sum_a, GOLDEN_CHECKSUM,
            "combined field checksum drifted from golden"
        );
    }

    #[test]
    fn two_species_coexist() {
        // Over a long run, neither species is driven extinct and each reaches a
        // real, network-sized population. The shared food field plus the two
        // species' distinct niches (sensor scale + ecology timing) keep both alive.
        let mut sim = Sim::new(256, 256, 7);
        // A "network-sized" floor: a few thousand agents is a visible network on
        // a 256×256 field. The starting per-species count is 256*256/8/2 ≈ 4096.
        const FLOOR: usize = 600;

        let mut min_pop = [usize::MAX; SPECIES];
        for _ in 0..3000 {
            sim.tick();
            for (s, m) in min_pop.iter_mut().enumerate() {
                let p = sim.species_population(s);
                if p < *m {
                    *m = p;
                }
                assert!(p > 0, "species {s} went extinct (competitive exclusion)");
            }
        }
        for (s, &m) in min_pop.iter().enumerate() {
            assert!(
                m >= FLOOR,
                "species {s} dropped to {m} (< floor {FLOOR}): niche separation too weak"
            );
        }
        // Each species ends the run at a real, network-sized population.
        for s in 0..SPECIES {
            let p = sim.species_population(s);
            assert!(
                p >= FLOOR,
                "species {s} ended at {p} (< floor {FLOOR}): not a real network"
            );
        }
    }

    #[test]
    fn metrics_match_direct_reductions() {
        let mut sim = Sim::new(128, 128, 13);
        for _ in 0..40 {
            sim.tick();
        }

        // Per-species trail mass ≈ the direct f64 sum of that species' field.
        for s in 0..SPECIES {
            let direct: f64 = sim.field(s).iter().map(|&v| v as f64).sum();
            let metric = sim.trail_mass(s);
            let tol = (direct.abs() * 1e-4).max(1e-3);
            assert!(
                (metric - direct).abs() <= tol,
                "trail_mass({s}) {metric} != direct {direct}"
            );
        }
        // Out-of-range species reads as zero.
        assert_eq!(sim.trail_mass(SPECIES), 0.0);

        // Food total ≈ the direct f64 sum of the food field.
        let food_direct: f64 = sim.food().iter().map(|&v| v as f64).sum();
        let food_metric = sim.food_total();
        let tol = (food_direct.abs() * 1e-4).max(1e-3);
        assert!(
            (food_metric - food_direct).abs() <= tol,
            "food_total {food_metric} != direct {food_direct}"
        );

        // Food coverage is a valid fraction and matches a direct count.
        let cov = sim.food_coverage();
        assert!(
            (0.0..=1.0).contains(&cov),
            "food_coverage {cov} out of [0,1]"
        );
        let covered = sim.food().iter().filter(|&&v| v > 0.02).count();
        let cov_direct = covered as f32 / sim.food().len() as f32;
        assert!(
            (cov - cov_direct).abs() <= 1e-6,
            "food_coverage {cov} != direct {cov_direct}"
        );
    }

    #[test]
    fn inspector_accessors_read_back_state() {
        let mut sim = Sim::new(96, 96, 21);
        for _ in 0..30 {
            sim.tick();
        }

        // trail_at / food_at match direct indexing through the same cell mapping.
        // Probe a handful of agent positions (live cells with real signal).
        let n = sim.agent_count();
        assert!(n > 0);
        for &i in &[0usize, n / 3, n / 2, n - 1] {
            let (px, py) = (sim.agent_x(i), sim.agent_y(i));
            let idx = sim.idx(px, py);
            for s in 0..SPECIES {
                assert_eq!(sim.trail_at(s, px, py), sim.field(s)[idx]);
            }
            assert_eq!(sim.food_at(px, py), sim.food()[idx]);
        }
        // Out-of-range species trail reads as zero.
        assert_eq!(sim.trail_at(SPECIES, 1.0, 1.0), 0.0);

        // nearest_agent returns a valid in-range index whose distance is minimal.
        let (qx, qy) = (40.0f32, 55.0f32);
        let near = sim.nearest_agent(qx, qy);
        assert!(near >= 0 && (near as usize) < n);
        let near = near as usize;
        let (w, h) = (sim.width() as f32, sim.height() as f32);
        let torus_d2 = |ax: f32, ay: f32| {
            let mut dx = (ax - qx).abs();
            if dx > w - dx {
                dx = w - dx;
            }
            let mut dy = (ay - qy).abs();
            if dy > h - dy {
                dy = h - dy;
            }
            dx * dx + dy * dy
        };
        let best = torus_d2(sim.agent_x(near), sim.agent_y(near));
        for i in 0..n {
            assert!(
                torus_d2(sim.agent_x(i), sim.agent_y(i)) >= best - 1e-3,
                "nearest_agent {near} is not actually closest (agent {i} is nearer)"
            );
        }

        // Per-agent getters: species in range, energy finite, position in bounds.
        assert!(sim.agent_species(near) < SPECIES);
        assert!(sim.agent_energy(near).is_finite());
        assert!((0.0..w).contains(&sim.agent_x(near)));
        assert!((0.0..h).contains(&sim.agent_y(near)));

        // Out-of-range agent getters return safe defaults.
        let oob = sim.agent_count() + 10;
        assert_eq!(sim.agent_species(oob), 0);
        assert_eq!(sim.agent_energy(oob), 0.0);
        assert_eq!(sim.agent_x(oob), 0.0);
        assert_eq!(sim.agent_y(oob), 0.0);

        // Empty population → no nearest agent.
        let mut empty = Sim::with_capacity(32, 32, 1, 0);
        assert_eq!(empty.agent_count(), 0);
        assert_eq!(empty.nearest_agent(5.0, 5.0), -1);
        // Don't leave `empty` unused beyond this; touch it to silence warnings.
        empty.tick();
        assert_eq!(empty.nearest_agent(5.0, 5.0), -1);
    }

    #[test]
    fn reset_restores_identical_state() {
        let mut sim = Sim::new(100, 100, 5);
        for _ in 0..20 {
            sim.tick();
        }
        let after_first = checksum_all(&sim);

        sim.reset(5);
        for _ in 0..20 {
            sim.tick();
        }
        assert_eq!(
            checksum_all(&sim),
            after_first,
            "reset(seed) + same ticks must reproduce both fields"
        );
    }

    // --- M6 world-geography tests ---

    /// The `food_attraction == 0` guard: with the default params (attraction off,
    /// no obstacles, no endpoints), a run that never touches the food/obstacle code
    /// must reproduce the *baseline* golden checksum exactly. This proves the new
    /// geography code is fully inert by default — the same proof as
    /// `tick_is_deterministic`, restated to make the guard explicit.
    #[test]
    fn food_attraction_zero_is_inert() {
        const GOLDEN_SEED: u64 = 0xABCD_1234;
        const TICKS: usize = 60;
        const GOLDEN_CHECKSUM: u64 = 0x8de7_8e52_803c_7618;

        let mut sim = Sim::new(128, 128, GOLDEN_SEED);
        // Defaults: attraction is 0 on both species, no walls, no endpoints.
        for s in 0..SPECIES {
            assert_eq!(sim.params(s).food_attraction, 0.0);
        }
        assert_eq!(sim.obstacle_count(), 0);
        assert_eq!(sim.endpoint_count(), 0);
        for _ in 0..TICKS {
            sim.tick();
        }
        assert_eq!(
            checksum_all(&sim),
            GOLDEN_CHECKSUM,
            "zero-attraction / empty-geometry run drifted from the baseline golden"
        );
    }

    /// `load_maze_demo` is deterministic: a fixed seed reproduces identical fields
    /// across two independent sims and matches a pinned golden value. If the maze
    /// layout or the rule changes on purpose, run once and paste the new value.
    #[test]
    fn maze_demo_is_deterministic() {
        const SEED: u64 = 0x00C0_FFEE;
        const TICKS: usize = 80;

        let run = || {
            let mut sim = Sim::new(128, 128, SEED);
            sim.load_maze_demo();
            for _ in 0..TICKS {
                sim.tick();
            }
            checksum_all(&sim)
        };
        let a = run();
        let b = run();
        assert_eq!(a, b, "two maze runs of the same seed must match");

        const MAZE_GOLDEN_CHECKSUM: u64 = 0xb5c4_42c3_0c1f_271b;
        assert_eq!(
            a, MAZE_GOLDEN_CHECKSUM,
            "maze-demo combined field checksum drifted from golden"
        );
    }

    /// `load_maze_demo` builds a sane scenario: walls present, exactly two endpoints,
    /// food-attraction on for both species, a real population, and the agent/field
    /// buffers never reallocate (zero-copy preserved across the reset-class call).
    #[test]
    fn maze_demo_builds_expected_world() {
        let mut sim = Sim::new(128, 128, 1);
        let field_ptrs: [_; SPECIES] = std::array::from_fn(|s| sim.field(s).as_ptr());
        let food_ptr = sim.food().as_ptr();
        let obstacle_ptr = sim.obstacles().as_ptr();
        let cap = sim.capacity();

        sim.load_maze_demo();

        assert!(sim.obstacle_count() > 0, "maze must lay down walls");
        assert_eq!(sim.endpoint_count(), 2, "maze places two endpoints");
        for s in 0..SPECIES {
            assert!(
                sim.params(s).food_attraction > 0.0,
                "species {s} must be food-attracted in the maze"
            );
        }
        assert!(sim.agent_count() > 0, "maze must spawn a population");
        assert_eq!(sim.tick_count(), 0, "maze resets the tick counter");

        // Zero-copy preserved: no buffer reallocated despite rewriting world state.
        for (s, &ptr) in field_ptrs.iter().enumerate() {
            assert_eq!(sim.field(s).as_ptr(), ptr, "field {s} reallocated");
        }
        assert_eq!(sim.food().as_ptr(), food_ptr, "food reallocated");
        assert_eq!(
            sim.obstacles().as_ptr(),
            obstacle_ptr,
            "obstacles reallocated"
        );
        assert_eq!(sim.capacity(), cap, "agent capacity changed");

        // Mask is binary and counts match.
        let walls = sim.obstacles().iter().filter(|&&v| v != 0).count();
        assert_eq!(
            walls,
            sim.obstacle_count(),
            "obstacle_count must match the mask"
        );
        assert!(sim.obstacles().iter().all(|&v| v == 0 || v == 1));

        // The toroidal x=0 seam is sealed full-height (otherwise a straight trail
        // across the open seam would bypass the serpentine).
        let h = sim.height();
        let w = sim.width();
        for row in 0..h {
            assert_eq!(
                sim.obstacles()[row * w],
                1,
                "x=0 seam must be walled at every row (row {row})"
            );
        }
        // ...and the endpoints + spawn corridor stay clear of that seam wall.
        for i in 0..sim.endpoint_count() {
            let idx = sim.idx(sim.endpoint_x(i), sim.endpoint_y(i));
            assert_eq!(
                sim.obstacles()[idx],
                0,
                "endpoint {i} must sit in an open cell, not the seam wall"
            );
        }

        // A maze run stays bounded and never reallocates the trail buffers.
        for _ in 0..200 {
            sim.tick();
        }
        for (s, &ptr) in field_ptrs.iter().enumerate() {
            assert_eq!(sim.field(s).as_ptr(), ptr, "field {s} reallocated mid-run");
        }
        // Trail never accumulates inside a wall cell (the field pass masks it).
        for s in 0..SPECIES {
            for (k, &v) in sim.field(s).iter().enumerate() {
                if sim.obstacles()[k] != 0 {
                    assert_eq!(v, 0.0, "trail leaked into a wall cell");
                }
            }
        }
    }

    /// The seam seal works: with the maze walls in place, a single straight
    /// horizontal band of trail at the endpoints' shared y does NOT connect the two
    /// wells — the band is blocked by the interior walls (whose gaps are at other
    /// rows) and can no longer slip around the toroidal x=0 seam. The only way to
    /// register `endpoints_connected == 2` is to thread the serpentine gaps.
    #[test]
    fn maze_seam_blocks_a_straight_bridge() {
        let mut sim = Sim::new(128, 128, 3);
        sim.load_maze_demo();

        // Lay a strong, full-width horizontal band of trail at the endpoints' row.
        // (Both endpoints share the same y in the demo, so a single band covers both.)
        let w = sim.width();
        let h = sim.height();
        let row = (sim.endpoint_y(0) as usize).min(h - 1);
        for r in row.saturating_sub(1)..=(row + 1).min(h - 1) {
            for col in 0..w {
                sim.field[0][r * w + col] = 10.0;
            }
        }

        // Endpoint 0 is in the network; endpoint 1 is walled off from a straight
        // band, so it must NOT count as connected through the seam.
        assert_eq!(
            sim.endpoints_connected(),
            1,
            "a straight horizontal band must not bypass the serpentine via the seam"
        );
    }

    /// A reachability scenario hand-built on the trail field: two endpoints in an
    /// open world start disconnected (no trail between them) and become connected
    /// once a band of trail bridges them. Asserts `endpoints_connected` goes 1 → 2
    /// and `network_cost > 0`.
    #[test]
    fn reachability_detects_a_growing_bridge() {
        // Small open world (no obstacles), two endpoints far apart on the same row.
        let mut sim = Sim::with_capacity(64, 64, 1, 0);
        // Drop the random food patches so they don't interfere (irrelevant to the
        // metric, which only reads the trail field, but keeps the test legible).
        sim.clear_obstacles();
        sim.add_endpoint(8.0, 32.0, 3.0);
        sim.add_endpoint(56.0, 32.0, 3.0);
        assert_eq!(sim.endpoint_count(), 2);

        // No trail yet → endpoint 0 sees only itself; cost is whatever lies under
        // endpoint 0 (zero, since the field is empty) → 1 connected at most.
        assert_eq!(sim.endpoints_connected(), 0); // empty field: nothing above thresh

        // Paint a strong trail blob over endpoint 0 only.
        for x in 5..12 {
            for y in 29..36 {
                sim.field[0][y * 64 + x] = 10.0;
            }
        }
        // Endpoint 0 is now in the network; endpoint 1 is not → exactly 1 connected.
        assert_eq!(sim.endpoints_connected(), 1);
        assert!(sim.network_cost() > 0);

        // Lay a continuous trail band across the middle row, bridging both endpoints.
        for x in 0..64 {
            for y in 30..35 {
                sim.field[0][y * 64 + x] = 10.0;
            }
        }
        assert_eq!(
            sim.endpoints_connected(),
            2,
            "both endpoints should now be connected by the bridge"
        );
        assert!(
            sim.network_cost() >= 64,
            "the bridging band should span the world"
        );
    }

    /// A wall dividing the world keeps two endpoints on opposite sides disconnected
    /// even with a strong trail filling the whole field — the flood-fill respects
    /// walls (it can't cross, and the toroidal wrap is blocked by a full-height wall).
    #[test]
    fn reachability_respects_a_dividing_wall() {
        let mut sim = Sim::with_capacity(64, 64, 1, 0);
        // Two full-height walls split the torus into two sealed halves (one wall
        // alone would leave the toroidal wrap-around path open).
        sim.paint_obstacle_rect(31, 0, 31, 63, true);
        sim.paint_obstacle_rect(63, 0, 63, 63, true);
        sim.add_endpoint(16.0, 32.0, 3.0); // left half
        sim.add_endpoint(47.0, 32.0, 3.0); // right half

        // Fill every open cell with trail — connectivity is purely a wall question.
        for k in 0..(64 * 64) {
            sim.field[0][k] = 10.0;
        }
        assert_eq!(
            sim.endpoints_connected(),
            1,
            "a sealed wall must keep the far endpoint unreachable"
        );
        // The cost equals the open cells in endpoint 0's half (not the whole world).
        let cost = sim.network_cost();
        assert!(
            cost > 0 && cost < 64 * 64,
            "cost {cost} should be half-world"
        );
    }

    /// Painting and clearing obstacles keeps `obstacle_count` exact, and `reset`
    /// returns the world to fully open with endpoints cleared.
    #[test]
    fn obstacle_paint_clear_and_reset() {
        let mut sim = Sim::new(64, 64, 1);
        assert_eq!(sim.obstacle_count(), 0);
        sim.paint_obstacle(32.0, 32.0, 5.0, true);
        let painted = sim.obstacle_count();
        assert!(painted > 0);
        assert_eq!(
            sim.obstacles().iter().filter(|&&v| v != 0).count(),
            painted,
            "count must match the mask after paint"
        );
        // Painting the same disk off clears exactly those cells.
        sim.paint_obstacle(32.0, 32.0, 5.0, false);
        assert_eq!(sim.obstacle_count(), 0);

        // Endpoints + obstacles are cleared by reset.
        sim.paint_obstacle(10.0, 10.0, 4.0, true);
        sim.add_endpoint(20.0, 20.0, 3.0);
        assert!(sim.obstacle_count() > 0 && sim.endpoint_count() == 1);
        sim.reset(1);
        assert_eq!(sim.obstacle_count(), 0, "reset clears obstacles");
        assert_eq!(sim.endpoint_count(), 0, "reset clears endpoints");
        assert!(sim.obstacles().iter().all(|&v| v == 0));
    }

    /// An endpoint pins its food cap high and persistent: the cell under an endpoint
    /// holds full food and stays fed across ticks (it keeps regrowing).
    #[test]
    fn endpoint_pins_persistent_food() {
        let mut sim = Sim::with_capacity(64, 64, 1, 0);
        sim.add_endpoint(32.0, 32.0, 4.0);
        let f0 = sim.food_at(32.0, 32.0);
        assert!(
            f0 >= FOOD_PATCH_PEAK - 1e-6,
            "endpoint cell should start at full food, got {f0}"
        );
        // With no agents to eat it, the endpoint cell stays full across ticks.
        for _ in 0..50 {
            sim.tick();
        }
        let f1 = sim.food_at(32.0, 32.0);
        assert!(
            f1 >= FOOD_PATCH_PEAK - 1e-3,
            "endpoint food drained, got {f1}"
        );
        // Accessors read back the endpoint geometry.
        assert_eq!(sim.endpoint_count(), 1);
        assert!((sim.endpoint_x(0) - 32.0).abs() < 1e-3);
        assert!((sim.endpoint_y(0) - 32.0).abs() < 1e-3);
        assert!((sim.endpoint_radius(0) - 4.0).abs() < 1e-3);
        // Out-of-range accessors are safe.
        assert_eq!(sim.endpoint_x(9), 0.0);
    }

    /// The network threshold knob clamps to `[0, 1]` and round-trips.
    #[test]
    fn network_threshold_knob() {
        let mut sim = Sim::new(32, 32, 1);
        assert_eq!(sim.network_threshold(), DEFAULT_NETWORK_THRESHOLD);
        sim.set_network_threshold(0.25);
        assert_eq!(sim.network_threshold(), 0.25);
        sim.set_network_threshold(5.0);
        assert_eq!(sim.network_threshold(), 1.0);
        sim.set_network_threshold(-1.0);
        assert_eq!(sim.network_threshold(), 0.0);
    }

    /// Walls block moves into them: an agent in an open cell, aimed straight at a
    /// wall, never penetrates it. Built with a single agent (no default population,
    /// no births to confound the count) on a world split by a full-height wall.
    #[test]
    fn walls_block_moves_into_them() {
        let mut sim = Sim::with_capacity(64, 64, 5, 16);
        // Strip the default population: this test wants exactly one tracked agent.
        sim.x.clear();
        sim.y.clear();
        sim.heading.clear();
        sim.energy.clear();
        sim.species.clear();
        sim.clear_obstacles();
        // A full-height wall at column 32. Make the ecology benign so the lone agent
        // neither starves nor reproduces during the test.
        sim.paint_obstacle_rect(30, 0, 34, 63, true);
        for s in 0..SPECIES {
            let mut e = sim.ecology(s);
            e.metabolism = 0.0;
            e.repro_threshold = 1e9;
            sim.set_ecology(s, e);
        }
        // Place one agent just left of the wall, heading straight at it (+x).
        sim.x.push(28.0);
        sim.y.push(32.0);
        sim.heading.push(0.0);
        sim.energy.push(1.0);
        sim.species.push(0);
        for _ in 0..200 {
            sim.tick();
            assert_eq!(sim.agent_count(), 1, "the lone agent must persist");
            let idx = sim.idx(sim.agent_x(0), sim.agent_y(0));
            assert_eq!(sim.obstacles()[idx], 0, "agent penetrated the wall");
        }
    }

    /// The load-bearing guarantee: the maze demo spawns only in open space, so no
    /// agent is ever inside a wall across a full run (covers both spawn and move).
    #[test]
    fn maze_agents_never_enter_walls() {
        let mut maze = Sim::new(128, 128, 9);
        maze.load_maze_demo();
        for _ in 0..250 {
            maze.tick();
            for i in 0..maze.agent_count() {
                let idx = maze.idx(maze.agent_x(i), maze.agent_y(i));
                assert_eq!(
                    maze.obstacles()[idx],
                    0,
                    "maze agent {i} stands inside a wall cell (spawn or move bug)"
                );
            }
        }
    }

    /// The M6 gate, proven forward: run the maze demo and confirm the network
    /// actually routes around the walls and connects the two endpoint wells —
    /// `endpoints_connected()` reaches `2` at some tick. This is the throttle-free,
    /// in-sim proof that the demo solves the maze without the seam cheat: the
    /// pre-grown colony blankets every open cell, so a connected network threads all
    /// the serpentine gaps from the first ticks; `maze_seam_blocks_a_straight_bridge`
    /// separately proves that connection cannot be a straight bridge across the seam.
    #[test]
    fn maze_demo_connects_the_endpoints() {
        const SEED: u64 = 0x00C0_FFEE;
        // A generous budget: the blanketed network connects within a few ticks, but
        // ticking out to a few thousand keeps the proof robust and still runs in well
        // under a second natively.
        const BUDGET: usize = 4000;

        let mut sim = Sim::new(128, 128, SEED);
        sim.load_maze_demo();
        // Both wells start disconnected — no trail has been laid yet.
        assert_eq!(
            sim.endpoints_connected(),
            0,
            "no network exists before the first tick"
        );

        let mut connected_at: Option<usize> = None;
        for t in 1..=BUDGET {
            sim.tick();
            if sim.endpoints_connected() == 2 {
                connected_at = Some(t);
                break;
            }
        }
        assert!(
            connected_at.is_some(),
            "maze demo never connected both endpoints within {BUDGET} ticks \
             (the network must route through the serpentine gaps)"
        );
    }

    // --- Cross-species sensing tests ---

    /// The cross-sensing matrix defaults to the identity, round-trips through the
    /// accessors, and `reset` restores it. A default-matrix run is byte-identical to
    /// the baseline golden (the own-trail fast path is taken).
    #[test]
    fn cross_sense_defaults_to_identity_and_is_inert() {
        const GOLDEN_SEED: u64 = 0xABCD_1234;
        const TICKS: usize = 60;
        const GOLDEN_CHECKSUM: u64 = 0x8de7_8e52_803c_7618;

        let mut sim = Sim::new(128, 128, GOLDEN_SEED);
        // Default is identity: diagonal 1, off-diagonal 0.
        for s in 0..SPECIES {
            for o in 0..SPECIES {
                let expected = if s == o { 1.0 } else { 0.0 };
                assert_eq!(sim.cross_sense(s, o), expected);
            }
        }
        // Out-of-range reads as 0 and writes are ignored.
        assert_eq!(sim.cross_sense(SPECIES, 0), 0.0);
        sim.set_cross_sense(SPECIES, 0, 5.0); // no-op
                                              // Round-trip a write, then restore identity by hand and confirm inertness.
        sim.set_cross_sense(0, 1, -0.6);
        assert_eq!(sim.cross_sense(0, 1), -0.6);
        sim.set_cross_sense(0, 1, 0.0);

        for _ in 0..TICKS {
            sim.tick();
        }
        assert_eq!(
            checksum_all(&sim),
            GOLDEN_CHECKSUM,
            "identity cross-sense run drifted from the baseline golden"
        );

        // reset restores identity.
        sim.set_cross_sense(0, 1, 0.9);
        sim.reset(GOLDEN_SEED);
        for s in 0..SPECIES {
            for o in 0..SPECIES {
                let expected = if s == o { 1.0 } else { 0.0 };
                assert_eq!(
                    sim.cross_sense(s, o),
                    expected,
                    "reset must restore identity"
                );
            }
        }
    }

    /// Cross-sensing is deterministic and couples the species: under mutual avoidance
    /// a fixed seed reproduces identical fields across two runs and matches a pinned
    /// golden. Distinct from the identity golden (proves the matrix actually changes
    /// the dynamics). If the rule changes on purpose, run once and paste the new value.
    #[test]
    fn cross_sense_is_deterministic() {
        const SEED: u64 = 0xABCD_1234;
        const TICKS: usize = 60;

        let run = || {
            let mut sim = Sim::new(128, 128, SEED);
            sim.set_cross_sense(0, 1, -0.6);
            sim.set_cross_sense(1, 0, -0.6);
            for _ in 0..TICKS {
                sim.tick();
            }
            checksum_all(&sim)
        };
        let a = run();
        let b = run();
        assert_eq!(a, b, "two cross-sense runs of the same seed must match");

        const CROSS_GOLDEN_CHECKSUM: u64 = 0x3d9b_7b01_f419_0a81;
        assert_eq!(
            a, CROSS_GOLDEN_CHECKSUM,
            "cross-sense checksum drifted from golden"
        );

        // The avoidance matrix must actually change the outcome vs. identity.
        const IDENTITY_GOLDEN: u64 = 0x8de7_8e52_803c_7618;
        assert_ne!(
            a, IDENTITY_GOLDEN,
            "avoidance must differ from the identity run"
        );
    }

    /// Cross-sensing couples the species: under mutual avoidance their trail fields
    /// segregate — the spatial overlap (cells where BOTH species hold significant
    /// trail) is strictly lower than under the default identity matrix, from the same
    /// seed after the same ticks. Overlap uses a per-field-relative threshold so it
    /// compares structure, not absolute magnitude.
    #[test]
    fn mutual_avoidance_segregates_species() {
        const SEED: u64 = 7;
        const TICKS: usize = 400;
        // A cell "holds" a species if its trail is at least this fraction of that
        // species' own field max — adapts to each run's exposure.
        const FRAC: f32 = 0.15;

        let overlap = |sim: &Sim| -> usize {
            let t0 = FRAC * sim.field_max(0);
            let t1 = FRAC * sim.field_max(1);
            let f0 = sim.field(0);
            let f1 = sim.field(1);
            let mut c = 0usize;
            for k in 0..f0.len() {
                if f0[k] > t0 && f1[k] > t1 {
                    c += 1;
                }
            }
            c
        };

        // Baseline: identity matrix (default).
        let mut base = Sim::new(256, 256, SEED);
        for _ in 0..TICKS {
            base.tick();
        }
        let overlap_base = overlap(&base);

        // Avoidance: each species repelled by the other's trail.
        let mut avoid = Sim::new(256, 256, SEED);
        avoid.set_cross_sense(0, 1, -0.6);
        avoid.set_cross_sense(1, 0, -0.6);
        for _ in 0..TICKS {
            avoid.tick();
        }
        let overlap_avoid = overlap(&avoid);

        assert!(
            overlap_avoid < overlap_base,
            "mutual avoidance should reduce species overlap: avoid {overlap_avoid} >= base {overlap_base}"
        );
    }
}
