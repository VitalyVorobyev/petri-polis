//! petri-core — the pure, native-testable simulation core for Petri Polis.
//!
//! M1 status: the [`Sim`] runs the **Physarum** (slime-mold) trail rule
//! (Jeff Jones 2010). Agents deposit a scalar trail, sense the trail ahead, and
//! steer toward it; the emergent result is self-organizing vascular networks.
//! See `docs/DESIGN.md` → "The Physarum rule" for the spec implemented here.
//!
//! ## Invariants this crate upholds
//! - **Determinism.** All randomness flows from one seeded [`Rng`] owned by the
//!   sim. `same seed → identical run`. No wall-clock, no map iteration order.
//! - **Hot-loop discipline.** Agents are struct-of-arrays (`Vec<f32>` per
//!   attribute). Capacity is pre-allocated at [`Sim::new`]; [`Sim::tick`] never
//!   allocates. Fields are double-buffered for the blur (swap, don't copy).
//! - **Zero-copy.** The trail `field` is a fixed-size buffer allocated once at
//!   [`Sim::new`]; it never reallocates, so the `petri-wasm` pointer the JS
//!   `Float32Array` view aliases stays valid for the whole run.

use std::f32::consts::TAU;

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
}

impl Default for Params {
    fn default() -> Self {
        Self {
            sensor_angle: 0.39,   // ~22.5°
            sensor_distance: 9.0, // cells
            rotation_angle: 0.39, // ~22.5°
            step_size: 1.0,       // cells/tick
            deposit_amount: 5.0,  // field units/tick
            decay: 0.9,           // 10% evaporation/tick
            diffuse_weight: 0.5,  // center tap weight; neighbors share the rest
        }
    }
}

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

/// The simulation world. Owns the single-channel trail `field` (row-major,
/// `width * height`) that the renderer visualises, the struct-of-arrays agent
/// population, and the seeded PRNG that makes every run reproducible.
pub struct Sim {
    width: usize,
    height: usize,
    tick_count: u64,

    /// Trail field, row-major `width * height`. Allocated once; never reallocated.
    field: Vec<f32>,
    /// Scratch buffer for the separable blur. Same fixed size as `field`.
    field_b: Vec<f32>,
    /// Max field value after the most recent tick's field pass (auto-exposure).
    field_max: f32,

    // Agents, struct-of-arrays. Capacity reserved at `new`; never grown after.
    x: Vec<f32>,
    y: Vec<f32>,
    heading: Vec<f32>,

    rng: Rng,
    params: Params,
}

impl Sim {
    /// Allocate a `width × height` world, reserve [`DEFAULT_AGENT_CAPACITY`] agent
    /// slots, seed the PRNG, and spawn the default population (`≈ w*h/8` agents)
    /// uniformly with random headings. Uses [`Params::default`].
    pub fn new(width: usize, height: usize, seed: u64) -> Self {
        Self::with_capacity(width, height, seed, DEFAULT_AGENT_CAPACITY)
    }

    /// Like [`Sim::new`] but with an explicit agent capacity (the spawn cap).
    pub fn with_capacity(width: usize, height: usize, seed: u64, capacity: usize) -> Self {
        let cells = width * height;
        // The default population: a generous-but-not-saturating density.
        let initial = (cells / 8).min(capacity);

        let mut sim = Self {
            width,
            height,
            tick_count: 0,
            field: vec![0.0; cells],
            field_b: vec![0.0; cells],
            field_max: 0.0,
            x: Vec::with_capacity(capacity),
            y: Vec::with_capacity(capacity),
            heading: Vec::with_capacity(capacity),
            rng: Rng::seed(seed),
            params: Params::default(),
        };
        sim.spawn_uniform(initial);
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

    /// Number of live agents.
    pub fn agent_count(&self) -> usize {
        self.x.len()
    }

    /// Reserved agent capacity (the spawn cap). Never changes after construction.
    pub fn capacity(&self) -> usize {
        self.x.capacity()
    }

    /// Read-only view of the trail field (row-major, `len == width * height`).
    /// `petri-wasm` hands JS a zero-copy pointer into this buffer.
    pub fn field(&self) -> &[f32] {
        &self.field
    }

    /// Largest field value after the most recent tick (for renderer auto-exposure).
    /// Computed cheaply during the field pass — `0.0` before the first tick.
    pub fn field_max(&self) -> f32 {
        self.field_max
    }

    /// Current live parameters.
    pub fn params(&self) -> Params {
        self.params
    }

    /// Replace the live parameters. Takes effect on the next [`Sim::tick`]; no
    /// reallocation, no rebuild.
    pub fn set_params(&mut self, params: Params) {
        self.params = params;
    }

    /// Re-seed the PRNG, clear the field, and respawn the default population —
    /// reusing the existing buffers (no reallocation, so the JS field view that
    /// aliases linear memory is *not* detached by this call).
    pub fn reset(&mut self, seed: u64) {
        self.rng = Rng::seed(seed);
        self.tick_count = 0;
        self.field_max = 0.0;
        for v in self.field.iter_mut() {
            *v = 0.0;
        }
        for v in self.field_b.iter_mut() {
            *v = 0.0;
        }
        self.x.clear();
        self.y.clear();
        self.heading.clear();
        let initial = (self.width * self.height / 8).min(self.capacity());
        self.spawn_uniform(initial);
    }

    /// Spawn `count` agents about `(cx, cy)` in the given `pattern`, capped so the
    /// total never exceeds the reserved capacity (which would grow linear memory
    /// and detach the JS field view). Headings are random. Returns how many were
    /// actually added.
    pub fn spawn(&mut self, cx: f32, cy: f32, count: usize, pattern: SpawnPattern) -> usize {
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
        }
        n
    }

    /// Spawn `count` agents uniformly over the whole field with random headings.
    /// Used for the initial / reset population. Caps at capacity.
    fn spawn_uniform(&mut self, count: usize) {
        let room = self.capacity() - self.agent_count();
        let n = count.min(room);
        let (w, h) = (self.width as f32, self.height as f32);
        for _ in 0..n {
            self.x.push(self.rng.range(w));
            self.y.push(self.rng.range(h));
            self.heading.push(self.rng.range(TAU));
        }
    }

    /// Advance the simulation one tick: every agent senses, steers, moves
    /// (toroidal wrap), and deposits; then the field diffuses (separable blur)
    /// and decays. No allocation occurs in here.
    pub fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        self.agent_pass();
        self.field_pass();
    }

    /// Per-agent: sense 3 points ahead, steer by the Jones rule, move with
    /// toroidal wrap, deposit into the current field.
    #[inline]
    fn agent_pass(&mut self) {
        let w = self.width as f32;
        let h = self.height as f32;
        let p = self.params;
        let n = self.x.len();

        for i in 0..n {
            let heading = self.heading[i];
            let cx = self.x[i];
            let cy = self.y[i];

            // 1. Sense at three points at sensor_distance: center, left, right.
            let f = self.sense(cx, cy, heading, p.sensor_distance);
            let l = self.sense(cx, cy, heading - p.sensor_angle, p.sensor_distance);
            let r = self.sense(cx, cy, heading + p.sensor_angle, p.sensor_distance);

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

            self.heading[i] = new_heading;
            self.x[i] = nx;
            self.y[i] = ny;

            // 4. Deposit into the current field at the agent's new cell.
            let idx = self.idx(nx, ny);
            self.field[idx] += p.deposit_amount;
        }
    }

    /// Sample the field at `distance` cells ahead along `angle` from `(x, y)`,
    /// with toroidal wrap. Nearest-sample (cheaper than bilinear; spec allows it).
    #[inline(always)]
    fn sense(&self, x: f32, y: f32, angle: f32, distance: f32) -> f32 {
        let sx = x + angle.cos() * distance;
        let sy = y + angle.sin() * distance;
        self.field[self.idx(sx, sy)]
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

    /// Field update: separable blur (horizontal then vertical, toroidal) into the
    /// scratch buffer, multiply by `decay`, then swap. Records `field_max` cheaply
    /// in the final vertical pass. Double-buffered — swap, never copy.
    #[inline]
    fn field_pass(&mut self) {
        let w = self.width;
        let h = self.height;
        let center = self.params.diffuse_weight;
        let side = (1.0 - center) * 0.5;
        let decay = self.params.decay;

        // Horizontal blur: field -> field_b (row-wise, wrap at column edges).
        for row in 0..h {
            let base = row * w;
            for col in 0..w {
                let left = if col == 0 { w - 1 } else { col - 1 };
                let right = if col == w - 1 { 0 } else { col + 1 };
                self.field_b[base + col] = self.field[base + left] * side
                    + self.field[base + col] * center
                    + self.field[base + right] * side;
            }
        }

        // Vertical blur: field_b -> field (column-wise, wrap at row edges), apply
        // decay in the same pass, and track the max.
        let mut max = 0.0f32;
        for row in 0..h {
            let up = if row == 0 { h - 1 } else { row - 1 };
            let down = if row == h - 1 { 0 } else { row + 1 };
            let base = row * w;
            let base_up = up * w;
            let base_down = down * w;
            for col in 0..w {
                let v = (self.field_b[base_up + col] * side
                    + self.field_b[base + col] * center
                    + self.field_b[base_down + col] * side)
                    * decay;
                self.field[base + col] = v;
                if v > max {
                    max = v;
                }
            }
        }
        self.field_max = max;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cheap, order-sensitive checksum of the field for determinism guards.
    fn checksum(field: &[f32]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
        for &v in field {
            h ^= v.to_bits() as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
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
        assert_eq!(sim.field().len(), 256 * 256);
        assert_eq!(sim.agent_count(), 256 * 256 / 8);
        assert_eq!(sim.field_max(), 0.0); // no tick yet
    }

    #[test]
    fn agents_and_field_stay_in_bounds() {
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
        }
        for &v in sim.field() {
            assert!(v.is_finite() && v >= 0.0, "bad field value: {v}");
        }
        assert!(
            sim.field_max() > 0.0,
            "field should have signal after ticks"
        );
    }

    #[test]
    fn spawn_caps_at_capacity() {
        let mut sim = Sim::with_capacity(64, 64, 3, 1000);
        // Default population already filled it (64*64/8 = 512), 488 slots left.
        let before = sim.agent_count();
        let added = sim.spawn(32.0, 32.0, 100_000, SpawnPattern::UniformDisk);
        assert_eq!(sim.agent_count(), sim.capacity());
        assert_eq!(added, sim.capacity() - before);
        // A second spawn adds nothing — already full.
        assert_eq!(sim.spawn(0.0, 0.0, 10, SpawnPattern::Point), 0);
    }

    #[test]
    fn no_allocation_after_new() {
        // Capacity (and therefore the field/agent pointers) must not change
        // across ticks, spawns, or resets — this is the zero-copy invariant.
        let mut sim = Sim::new(96, 96, 99);
        let cap = sim.capacity();
        let field_ptr = sim.field().as_ptr();
        for _ in 0..30 {
            sim.tick();
        }
        sim.spawn(48.0, 48.0, 500, SpawnPattern::Ring);
        sim.reset(99);
        for _ in 0..30 {
            sim.tick();
        }
        assert_eq!(sim.capacity(), cap, "agent capacity must not change");
        assert_eq!(
            sim.field().as_ptr(),
            field_ptr,
            "field buffer must not reallocate"
        );
    }

    /// The determinism guard: a fixed seed must reproduce an identical field
    /// (checksum) after N ticks, both across two independent sims and against a
    /// pinned golden value. If the rule changes, update `GOLDEN` deliberately.
    #[test]
    fn tick_is_deterministic() {
        const GOLDEN_SEED: u64 = 0xABCD_1234;
        const TICKS: usize = 60;

        let run = || {
            let mut sim = Sim::new(128, 128, GOLDEN_SEED);
            for _ in 0..TICKS {
                sim.tick();
            }
            (checksum(sim.field()), sim.field_max())
        };

        let (sum_a, max_a) = run();
        let (sum_b, max_b) = run();
        assert_eq!(sum_a, sum_b, "two runs of the same seed must match");
        assert_eq!(max_a.to_bits(), max_b.to_bits());

        // Pinned golden checksum: this guards against accidental rule drift.
        // It is implementation-defined, not externally meaningful — if you change
        // the rule on purpose, run the test once and paste the new value here.
        const GOLDEN_CHECKSUM: u64 = 0xc3fd_bef3_77ef_f1cd;
        assert_eq!(sum_a, GOLDEN_CHECKSUM, "field checksum drifted from golden");
    }

    #[test]
    fn reset_restores_identical_state() {
        let mut sim = Sim::new(100, 100, 5);
        for _ in 0..20 {
            sim.tick();
        }
        let after_first = checksum(sim.field());

        sim.reset(5);
        for _ in 0..20 {
            sim.tick();
        }
        assert_eq!(
            checksum(sim.field()),
            after_first,
            "reset(seed) + same ticks must reproduce the field"
        );
    }
}
