# The Physarum rule

The core of Petri Polis is the **Physarum** trail rule (Jeff Jones, 2010), an agent-based model of how
the slime mold *Physarum polycephalum* grows efficient transport networks. Each agent is a point with
a position and a heading; the world is a single scalar **trail field** the agents both write to and
read from.

Every tick, each agent runs four steps: **sense → steer → move → deposit.**

## 1. Sense

The agent samples the trail field at **three points** a fixed distance ahead of it: straight ahead
(along its heading), and to the left and right, offset by the sensor angle.

```rust
let f = self.sense(s, cx, cy, heading,                 p.sensor_distance);
let l = self.sense(s, cx, cy, heading - p.sensor_angle, p.sensor_distance);
let r = self.sense(s, cx, cy, heading + p.sensor_angle, p.sensor_distance);
```

`sense` projects a point `sensor_distance` cells ahead along an angle and reads the field cell there:

```rust
fn sense(&self, s: usize, x: f32, y: f32, angle: f32, distance: f32) -> f32 {
    let sx = x + angle.cos() * distance;
    let sy = y + angle.sin() * distance;
    self.field[s][self.idx(sx, sy)]
}
```

Sampling is **nearest-cell**, not bilinear — it is cheaper, and the rule does not need sub-cell
precision. Two knobs live here:

- **`sensor_distance`** — how far ahead the agent looks. Short distances produce a fine,
  closely-spaced mesh; long distances produce coarse, far-apart veins.
- **`sensor_angle`** — how wide the side sensors splay. Wide angles make bushy, branching networks;
  narrow angles make straighter trunks.

## 2. Steer

The agent compares the three readings (`F`, `L`, `R`) and turns by a fixed `rotation_angle` according
to the **Jones steering rule**:

```rust
let new_heading = if f >= l && f >= r {
    heading                          // forward is best → keep going straight
} else if f < l && f < r {
    // forward is worst → turn away, picking a side at random
    if self.rng.next_f32() < 0.5 { heading - p.rotation_angle }
    else                         { heading + p.rotation_angle }
} else if l > r {
    heading - p.rotation_angle       // left is stronger → turn left
} else {
    heading + p.rotation_angle       // right is stronger → turn right
};
```

Read it as three cases:

- **Forward is strongest** → keep the current heading (commit to the trail you're on).
- **Forward is weakest** of the three → you're heading into a void; turn away by `rotation_angle`,
  choosing left or right with a coin flip.
- **Otherwise** → one side beats the other; turn toward the stronger side.

The single random draw — the coin flip in the "forward worst" case — is the only stochastic part of
the step, and it comes from the sim's seeded PRNG (see [Determinism](./determinism.md)). It lets the
network break symmetry and explore while staying perfectly reproducible.

## 3. Move

The agent advances one `step_size` along its new heading and **wraps toroidally** — the world has no
edges; it is a torus.

```rust
let mut nx = cx + new_heading.cos() * p.step_size;
let mut ny = cy + new_heading.sin() * p.step_size;
nx = nx.rem_euclid(w);   // wrap into [0, width)
ny = ny.rem_euclid(h);   // wrap into [0, height)
```

`rem_euclid` returns a non-negative remainder, so an agent leaving the right edge reappears on the
left, and the top connects to the bottom. Every field sample and deposit uses the same wrapping (the
`idx` helper), so the simulation is seamless across edges.

## 4. Deposit

After moving, the agent adds `deposit_amount` to the trail field at its new cell:

```rust
self.field[s][idx] += p.deposit_amount;
```

Deposits **accumulate additively** — many agents landing on one cell simply add up. There is no
conflict-resolution phase; contention is rare and handled by addition. This deposit is what the *next*
agents will sense, closing the stigmergic loop.

## Why this makes networks

Put the four steps together and the reinforcement dynamic appears:

1. An agent that happens to be on a faint trail senses it ahead and steers to stay on it.
2. Staying on it deposits more trail, making it stronger.
3. A stronger trail attracts more agents, which deposit more still.

Busy routes brighten and pull in traffic; unused routes are not reinforced and fade (see
[Diffusion and decay](./diffusion-decay.md)). The balance between **reinforcement** (deposit) and
**forgetting** (decay), shaped by the sensor geometry, tunes the result between a dense capillary mesh
and a few thick trunk roads. The agents never represent the network; it is an emergent equilibrium of
millions of local sense/steer/move/deposit decisions.

The next chapter covers the other half of the loop: how the field diffuses and decays between ticks.
