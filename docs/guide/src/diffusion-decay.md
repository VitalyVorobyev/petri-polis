# Diffusion and decay

The agents write to the trail field; between ticks, the field itself is updated in one pass that does
two things: it **diffuses** (blurs) the field so trails spread into a soft glow, and it **decays**
(evaporates) the field so unused trails fade. This is the "forgetting" half of the reinforcement loop,
and it is what makes the trails look like light rather than scattered dots.

## Diffusion: a separable 3-tap blur

Diffusion is a small Gaussian-like blur that mixes each cell with its neighbours. Doing a full 2D blur
directly would be expensive, so it is done **separably**: a horizontal pass followed by a vertical
pass. A separable blur gives the same result as the 2D kernel at a fraction of the cost (`2·k` samples
per cell instead of `k²`).

Each pass is a 3-tap kernel `[side, center, side]`, where the centre weight is the `diffuse_weight`
parameter and the two neighbours split the remainder:

```rust
let side = (1.0 - center) * 0.5;
```

So `diffuse_weight = 1.0` means "all centre, no neighbours" — no blur at all — and lower values spread
the field more. The weights always sum to 1, so a flat field stays flat: the blur conserves total mass
on its own, and decay is what removes mass.

The horizontal pass reads the field and writes a scratch buffer; the vertical pass reads the scratch
buffer and writes back into the field. Both wrap at the edges, matching the toroidal world:

```rust
// Horizontal: field -> scratch (wrap at column edges)
let left  = if col == 0     { w - 1 } else { col - 1 };
let right = if col == w - 1 { 0 }     else { col + 1 };
scratch[base + col] =
    field[base + left] * side + field[base + col] * center + field[base + right] * side;
```

## Decay: multiplicative evaporation

After the blur, every cell is multiplied by `decay`, a number just below 1:

```rust
let v = (/* blurred value */) * decay;
field[base + col] = v;
```

Decay is applied **in the same vertical pass** as the blur — no extra sweep over the field. A cell
that is no longer being reinforced loses a fixed fraction of its value each tick, so an abandoned trail
decays geometrically toward zero. `decay = 0.9` keeps a trail visible for tens of ticks; `decay = 0.99`
makes long-lived, slowly-fading structure; small values give a fast, twitchy, quick-turnover field.

The tension between **deposit** (see [The Physarum rule](./physarum-rule.md)) and **decay** sets the
equilibrium brightness of a trail: a route survives only if agents reinforce it at least as fast as it
evaporates. That is exactly why only *used* routes persist — and why the network continuously reshapes
itself as traffic shifts.

## Why blur and decay together

- **Blur** turns a deposit (a hard spike on one cell) into a smooth gradient the sensors can detect
  from a distance. Without it, an agent would have to land exactly on a previous deposit to feel it;
  with it, trails have a "scent" that reaches the sensor points. It is also what gives the render its
  soft, glowing filaments instead of speckle.
- **Decay** bounds the field (deposits would otherwise grow without limit) and provides the forgetting
  that lets the network adapt — old structure must be continually re-earned or it disappears.

## Folding in the metrics for free

The same vertical pass that applies decay also computes, with no extra scan, the two values the rest of
the system needs from the field:

- **`field_max`** — the largest cell value, used by the renderer for auto-exposure (it normalizes
  brightness by `1 / field_max`).
- **`trail_mass`** — the `f64` sum of every cell, exposed as a metric (see
  [Ecology and population dynamics](./ecology.md) and the inspector).

```rust
if v > max { max = v; }
total += v as f64;   // f64 keeps precision when summing a large, high-deposit field
```

Computing them here keeps the tick to a single field sweep and means the metrics are always exact for
the post-tick field, never a stale extra pass.

The function that does all of this — blur, decay, max, and mass — is `blur_field_decay` in
`crates/petri-core/src/lib.rs`. It is pure and allocation-free: it takes the field and a scratch buffer
and writes the result back in place. The next chapter covers how that scratch buffer is reused and how
the whole tick avoids allocation.
