# Ecology and population dynamics

The Physarum rule on its own gives a beautiful but static-population network. Petri Polis layers a
light **ecology** on top so the world also lives and dies: agents spend energy, eat, reproduce, and
starve, and the population rises and crashes in **boom/bust** cycles. The whole ecology is five
tunable numbers per species plus one shared food field.

## The food field

A second scalar field, `food`, rides alongside the trail fields. Its ceiling is `food_cap`, a
**static** map of soft Gaussian patches generated once at construction:

```rust
const FOOD_PATCH_COUNT: usize = 16;
```

Sixteen patches are placed at PRNG-chosen centres and radii and combined by **`max`**, so the field
peaks near 1.0 at patch centres and falls toward 0 between them. The result is a **patchy** landscape —
rich pockets separated by near-empty space. Patchiness is not cosmetic; it is the engine of recovery
(below), which is why it is a fixed structural choice rather than a live knob. The food field starts
full (`food = food_cap`) so the first boom has fuel.

## The per-agent loop: metabolize, eat, reproduce, die

Each tick, woven into the agent pass, every agent runs its ecology.

**Metabolism and eating** happen right after the agent moves:

```rust
let eaten = self.food[idx].min(e.eat_rate);
self.food[idx] -= eaten;
self.energy[i] = self.energy[i] - e.metabolism + eaten;
```

The agent pays `metabolism` (the cost of living) every tick, and eats up to `eat_rate` from the cell it
stands on, converted 1:1 to energy. An agent in a rich patch gains net energy; one in the void slowly
starves.

**Reproduction** is a split: a well-fed agent halves its energy and donates half to a newborn.

```rust
if self.energy[i] >= self.ecology[s].repro_threshold && self.x.len() < cap {
    let child_energy = self.energy[i] * 0.5;
    self.energy[i] = child_energy;
    // child inherits position and species, with a jittered heading:
    self.heading.push(self.heading[i] + jitter);
    // …push x, y, energy, species…
}
```

The child appears at the parent's position, with the parent's species and a slightly jittered heading,
at an index past the snapshot boundary so it waits until next tick to act. The capacity check
(`< cap`) means reproduction can never grow memory.

**Death feeds the world.** When an agent's energy reaches zero, it is removed — and it returns
`death_return` nutrient to the food cell where it died:

```rust
if self.energy[i] <= 0.0 {
    let idx = self.idx(self.x[i], self.y[i]);
    self.food[idx] += self.ecology[s].death_return;
    // swap_remove from all five agent arrays
}
```

This is the coupling that closes the loop: a die-off enriches the ground where it happened, so the next
generation can grow back there.

## Food regrowth

After the agent pass, `food_pass` relaxes every cell toward its cap by a small fraction:

```rust
let v = self.food[k] + regrow * (self.food_cap[k] - self.food[k]);
```

This is exponential relaxation: depleted cells refill quickly at first, then slowly as they approach
the cap. The shared food field is regrown at the **mean** of the two species' `food_regrow` values, so
either species' knob still influences the shared cycle (explained in [Two species](./two-species.md)).

## Why boom and bust

The cycle is an emergent consequence of the rules, not a scripted animation:

1. **Boom.** Food starts full, so agents eat freely, reproduce, and the population climbs — networks
   grow into the rich patches.
2. **Depletion.** A dense population eats faster than the food regrows; patches drain.
3. **Bust.** Energy income collapses, metabolism keeps draining, and a wave of starvation crashes the
   population. The die-off returns nutrient to the ground.
4. **Recovery.** Survivors sheltering in still-rich pockets — and the nutrient freed by the dead — let
   the population rebound, and the cycle repeats.

**Patchiness is what makes this recover instead of going extinct.** With uniform food, a global crash
would have no refuge; with patches, some pocket is always still rich enough to seed the next boom. A
native test, `boom_bust_cycle_recovers`, asserts exactly this shape: the population rises above its
start, later crashes to well under 60% of its peak, and never reaches zero.

## The metrics

The food pass folds in three world metrics with no extra scan — `food_total` (the `f64` sum of the
field), `food_coverage` (the fraction of cells above a small threshold), and `food_max` — joining the
per-species `trail_mass` from the field pass. These feed the live sparklines and the CSV/JSON export,
all keyed to `tick_count` so a measured run is reproducible (see [Determinism](./determinism.md)).

## The five ecology knobs

`metabolism`, `eat_rate`, `repro_threshold`, `food_regrow`, and `death_return` are all live —
changeable mid-run via `set_ecology`, no rebuild — and are catalogued in the
[Parameter reference](./parameters.md). Together they set the period and amplitude of the cycle:
cheaper living and faster regrowth give tighter, gentler oscillations; expensive living and slow
regrowth give longer, more violent ones.
