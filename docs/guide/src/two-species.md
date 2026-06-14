# Two species

Petri Polis runs **two species at once** (`SPECIES = 2`). Each is a full Physarum system in its own
right — its own trail field, its own parameter set, its own ecology timing, its own colour (cyan for
species 0, magenta for species 1). They share exactly one thing: the food field. From that single point
of contact come competition, niche partitioning, and a combined two-colour picture neither species
makes alone.

## What is per-species and what is shared

```rust
field: [Vec<f32>; SPECIES],   // one trail field each — NOT shared
params: [Params; SPECIES],    // one Physarum parameter set each
ecology: [Ecology; SPECIES],  // one ecology parameter set each
food: Vec<f32>,               // ONE food field — shared
```

Every agent carries a `species: u8` tag that selects which `params`, `ecology`, and `field` it uses.
The critical rule: **an agent senses and deposits only into its own species' trail.** Species 0 is
blind to species 1's trail and vice versa, so each self-organizes its own independent network. They
never read each other's trails; the only way one species affects the other is by **eating the shared
food first.**

## Competition through the shared food

Because the food field is shared, the two species compete for it spatially: wherever their networks
overlap a rich patch, whichever species' agents arrive first deplete it, and the other goes hungry
there. The two default parameter sets are deliberately tuned to **different niches** so this
competition settles into coexistence rather than one species erasing the other:

- **Species 0** — a fine, fast mesh: short sensors, sharp turns, a light deposit, a quick-fade trail,
  and a lean/fast ecology (cheap metabolism, reproduces sooner, faster food cycle).
- **Species 1** — coarse, thick veins: long sensors, gentle turns, a heavy deposit, a slow-fade trail,
  and a slower/hungrier ecology (reproduces later, rides a slower food cycle).

Different sensor scales make the two networks occupy different spatial *grain*, and different ecology
timing makes their boom/bust cycles **desynchronize**: when one species is busting, the other is often
booming on the food the first one stopped eating. That staggering is what keeps both alive.

## The shared regrowth rate

Sharing a single food field between two species that each have their own `food_regrow` knob raises a
question: the field can only regrow at one rate. Petri Polis uses the **mean** of the two:

```rust
let regrow = sum / SPECIES as f32;   // mean of the species' food_regrow
```

So each species still has a handle on the shared cycle's speed — tuning either `food_regrow` shifts the
common period — without the field needing a per-species regrowth it cannot have.

## Coexistence as an emergent, tested property

Coexistence is not guaranteed by fiat; it is an outcome of the tuning, and it is tested.
`two_species_coexist` runs 3000 ticks and asserts that **neither** species ever drops below a
network-sized floor and that both end at a real population — i.e. neither is competitively excluded and
each sustains a visible network. If a parameter change weakened the niche separation enough for one
species to dominate, that test would fail.

## Generalizing past two

The structure is written around the `SPECIES` constant, not hard-coded to two. Adding a third species
is, mechanically, bumping the constant and providing a third default `Params`/`Ecology` set (and a
third colour in the renderer). The reason it stays at two is restraint, not limitation: two species
already produce a legible, interwoven picture, and each additional species multiplies the knobs and
muddies the emergence. A third earns its place when there is a clear reason for it.
