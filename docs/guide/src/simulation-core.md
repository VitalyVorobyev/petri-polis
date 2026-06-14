# The simulation core

This chapter is about *how* the rules from the previous chapters are implemented so they run fast and
predictably. Three ideas carry the design: **struct-of-arrays** agents, a **double-buffered** field
blur, and a **no-allocation** tick over pre-reserved capacity.

## Struct-of-arrays agents

An agent has a position, a heading, an energy, and a species tag. The obvious layout — a `Vec<Agent>`
of structs — interleaves those fields in memory. Petri Polis instead uses **struct-of-arrays (SoA)**:
one `Vec` per attribute, all kept the same length and in lockstep.

```rust
x: Vec<f32>,
y: Vec<f32>,
heading: Vec<f32>,
energy: Vec<f32>,
species: Vec<u8>,
```

The agent with index `i` is `(x[i], y[i], heading[i], energy[i], species[i])`. SoA matters because the
hot loop streams one attribute at a time over the whole population; keeping each attribute contiguous
is cache-friendly and vectorizes well. Births and deaths keep every array in step: a birth `push`es to
all five, a death `swap_remove`s from all five at the same index.

## The fields

Alongside the agents, the sim owns the fields the agents and renderer use:

- **`field: [Vec<f32>; SPECIES]`** — one trail field per species, each row-major `width × height`. A
  species senses and deposits only into its own.
- **`field_b: Vec<f32>`** — a single shared scratch buffer for the blur, the size of one field, reused
  for each species in turn.
- **`food: Vec<f32>`** and **`food_cap: Vec<f32>`** — the shared nutrient field and its static per-cell
  ceiling (see [Ecology and population dynamics](./ecology.md)).

A continuous `(x, y)` maps to a field index through `idx`, the one place the row-major, wrap-around
indexing lives:

```rust
fn idx(&self, x: f32, y: f32) -> usize {
    let mut ix = (x.floor() as isize).rem_euclid(w as isize) as usize;
    let mut iy = (y.floor() as isize).rem_euclid(h as isize) as usize;
    // guard the f32→isize edge where x == w exactly after wrap
    if ix >= w { ix = w - 1; }
    if iy >= h { iy = h - 1; }
    iy * w + ix
}
```

## The tick pipeline

`tick()` advances the world by one step in three passes:

```rust
pub fn tick(&mut self) {
    self.tick_count = self.tick_count.wrapping_add(1);
    self.agent_pass();   // sense/steer/move/eat/deposit, then reproduce, then die
    self.field_pass();   // per species: blur + decay (+ field_max, trail_mass)
    self.food_pass();    // regrow food toward caps (+ food metrics)
}
```

### `agent_pass` — three phases over a snapshot

The agent pass iterates a **snapshot** of the population count `n` taken at the start of the tick, in
three phases:

1. **Move and metabolize.** For each of the first `n` agents: sense its own trail, steer, move
   (wrapping), eat from the shared food cell, pay its metabolic cost, and deposit trail. Agents read
   the current field and accumulate deposits into it as they go — there is no buffer swap between
   sensing and depositing within a tick.
2. **Reproduce.** Well-fed agents split (covered in
   [Ecology and population dynamics](./ecology.md)). Children are `push`ed at indices `≥ n`, so they
   are **not processed again this tick**. That snapshot boundary stops a newborn from acting on the
   tick it is born, which would create feedback loops.
3. **Die.** Starved agents are removed with `swap_remove`. Because `swap_remove` moves the last agent
   into the freed slot, the sweep does **not** advance its index after a removal — the swapped-in
   agent still needs checking:

```rust
let mut i = 0;
while i < self.x.len() {
    if self.energy[i] <= 0.0 {
        // … return food to the cell, then swap_remove from all five arrays …
    } else {
        i += 1;
    }
}
```

### `field_pass` — double-buffered blur, one scratch buffer

For each species, `field_pass` runs `blur_field_decay` on that species' field through the **shared**
scratch buffer:

```rust
for s in 0..SPECIES {
    let field = &mut self.field[s];
    let scratch = &mut self.field_b;
    let (max, mass) = blur_field_decay(field, scratch, w, h, center, decay);
    self.field_max[s] = max;
    self.trail_mass[s] = mass;
}
```

Each species is fully blurred before the next, so a single scratch buffer serves all of them — no
per-species allocation. The blur is "double-buffered" in the sense that it reads the field and writes
the scratch (horizontal), then reads the scratch and writes the field (vertical); the field ends
updated in place.

### `food_pass` — relax toward the caps

The food pass relaxes every cell toward its static cap and folds in the food metrics in the same loop
(see [Ecology and population dynamics](./ecology.md)).

## No allocation after construction

The load-bearing performance invariant: **`tick()` never allocates.** All capacity is reserved once at
construction. The agent `Vec`s are created with `Vec::with_capacity(capacity)` and never grow past it
— `spawn` is capped so the total can't exceed capacity, births `push` into the reserved space, deaths
`swap_remove`. The fields are allocated once and never resized; `reset` clears them in place rather
than reallocating.

This is not only a speed concern. It is also what keeps the **zero-copy** boundary valid: if any `Vec`
grew, WASM linear memory could move, and the JavaScript views aliasing it would detach (see
[WASM and the zero-copy field](./wasm.md)). A native test, `no_allocation_after_new`, pins the field,
food, and agent buffer pointers and asserts they are unchanged after ticks, spawns, and a reset.

The next chapter covers the other invariant the tests guard: determinism.
