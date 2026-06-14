# Determinism

Petri Polis makes one strong promise: **the same seed grows the same world.** Run the simulation twice
from seed `S` on the same WASM binary and you get byte-identical fields at every tick. This is what
lets a run be captured as a seed-plus-parameters URL and replayed exactly on another machine — and it
is what makes the field-checksum tests meaningful.

Determinism is a property you have to *defend*; it is easy to lose by accident. Petri Polis defends it
with three rules.

## 1. All randomness comes from one seeded PRNG

The sim owns exactly one random generator, and every random decision in the whole simulation draws
from it. There is no thread-local RNG, no system entropy, no wall-clock anywhere in the sim logic.

The generator is **xoshiro256\*\*** (Blackman & Vigna, 2018) — a small, fast, high-quality PRNG —
hand-rolled rather than pulled from a crate, so its behaviour is stable across library versions and
platforms:

```rust
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
```

A single `u64` seed is expanded into the 256-bit state with **SplitMix64**, which mixes one word into
four well-distributed ones (and guards against the all-zero state xoshiro forbids):

```rust
pub fn seed(seed: u64) -> Self {
    let mut z = seed;
    let mut next = || -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = z;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    };
    let s = [next(), next(), next(), next()];
    let s = if s == [0, 0, 0, 0] { [1, 2, 3, 4] } else { s };
    Self { s }
}
```

Floats come from the top 24 bits of a `u64`, matching the `f32` mantissa width, giving a uniform value
in `[0, 1)`:

```rust
pub fn next_f32(&mut self) -> f32 {
    ((self.next_u64() >> 40) as f32) * (1.0 / (1u32 << 24) as f32)
}
```

## 2. The draw order is load-bearing

A PRNG is a *sequence*; reproducibility depends on consuming it in exactly the same order every run.
Petri Polis treats draw order as part of its contract and documents it where it matters:

- **Construction** draws food patches first (3 draws per patch: centre x, centre y, radius), then the
  initial population (per agent: x, y, heading — species is assigned round-robin and consumes **no**
  draw).
- **`reset`** mirrors that order exactly — patches, then population — so a reset reproduces a fresh
  `new` with the same seed.
- **In the tick**, the only draws are the steering coin-flip (only in the "forward worst" case) and
  the birth heading jitter, both in a fixed per-agent order.

Change the order — draw heading before position, say, or process agents differently — and the run
diverges even though nothing is "wrong." That is why the order is spelled out in the code comments and
exercised by the tests.

## 3. No hidden nondeterminism

There is no wall-clock in the sim. The renderer measures FPS, but the simulation is driven by tick
count, never elapsed time. There is no iteration over a `HashMap` or any other unordered collection;
everything is dense arrays walked by index.

## How the tests guard it

Two native tests pin determinism:

- **`tick_is_deterministic`** runs a fixed seed for 60 ticks, twice, asserts the two runs match, *and*
  asserts the combined field checksum equals a pinned **golden value**. The checksum is an FNV-1a hash
  folded over both species' fields:

  ```rust
  fn checksum(field: &[f32]) -> u64 {
      let mut h: u64 = 0xcbf2_9ce4_8422_2325;        // FNV-1a offset basis
      for &v in field {
          h ^= v.to_bits() as u64;
          h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
      }
      h
  }
  ```

  The golden value is implementation-defined, not externally meaningful. It is a **drift detector**:
  if you change the rule, the test fails, and you update the golden value *deliberately* after
  confirming the change was intended. An accidental change — a reordered draw, a tweaked constant —
  fails loudly instead of silently altering every shared URL.

- **`reset_restores_identical_state`** runs N ticks, resets with the same seed, runs N ticks again, and
  asserts the checksum matches — proving `reset` is a true re-seed, not a partial one.

Determinism is the quiet foundation under the shareable-URL feature and the whole test strategy. With
it, "the same seed" is a complete, portable description of a run.
