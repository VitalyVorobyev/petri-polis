# Sources, sinks & mazes

Until now the world had no geography. The field wraps toroidally with no edges and no walls, and
food sits in soft patches scattered by the seed. Agents follow their own trail and eat whatever
food they happen to cross, but nothing in the world is *placed* — there is no "here" to connect to
"there." Geography adds three things that turn the open arena into a problem the colony can solve:
**obstacles** it must route around, **endpoints** it is pulled toward, and a **chemotaxis** drive
that makes that pull real. With them, Petri Polis reproduces the classic Physarum result — a slime
mold finding the shortest path through a maze — and a **reachability metric** reads out whether it
has.

## Obstacles: the wall mask

A second per-cell map rides alongside the trail and food fields:

```rust
obstacles: Vec<u8>,   // one byte per cell: 0 = open, 1 = wall
```

It is allocated once at `Sim::new` (never grown — the zero-copy contract holds) and starts
all-zero: an open world. Three rules make a wall impassable to the trail network:

1. **Sensing reads walls as empty.** A sensor sample that lands on a wall cell returns `0`, so no
   agent is ever drawn into one.
2. **Movement is blocked.** If an agent's next step would land in a wall, it stays in its current
   cell and turns by a single random step instead. (That random turn is the *only* new draw from
   the simulation's PRNG, and it happens *only* when an agent is actually blocked — which is why a
   world with no walls behaves exactly as it did before; see *Determinism preserved* below.)
3. **Trails can't bleed through.** After the diffuse-and-decay pass, every wall cell's trail value
   is forced back to `0`, so the glow never seeps across a barrier.

You paint walls into the mask live with `paint_obstacle(x, y, radius, on)` and clear them with
`clear_obstacles()`. Painting writes in place, so the field views stay valid — no re-fetch needed.

## Endpoints: sources and sinks

An endpoint is a **persistent food well** — a place the colony wants to reach because the food
there never runs out. `add_endpoint(x, y, radius)` bakes a high, flat-topped food source into the
food *cap* (a solid core out to the radius with a soft Gaussian skirt, combined with the random
patches by taking the maximum). Because the food field continually regrows toward its cap, an
endpoint stays full no matter how hard the colony grazes it — exactly the role the oat flakes play
in the laboratory maze experiment. The renderer marks each endpoint with a glowing ring so the
"connect these points" task is legible at a glance. `clear_endpoints()` removes them.

## Chemotaxis: why agents move toward food

Endpoints are only attractive if agents can *sense* them, and by default a Physarum agent senses
only its own trail — not food. The bridge is one new parameter:

```rust
food_attraction: f32,   // Params, default 0.0
```

When it is non-zero, each of the three trail samples is augmented with the food under that sensor:

```text
sensed = trail_sample + food_attraction · food_sample
```

Now the same steer-toward-the-strongest-sensor rule that builds trail networks also climbs the
food gradient: agents wander up toward the nearest well, lay trail as they go, and that trail
recruits more agents along the same route. Reinforcement does the rest — the heavily-travelled
paths between wells brighten and the dead ends fade. Turn `food_attraction` up and the colony
becomes purposeful; leave it at its `0.0` default and the agents behave exactly as the trail-only
chapters describe.

## The reachability metric

To ask *has the colony connected the endpoints, and how expensively?*, Petri Polis runs a
read-only graph search over the trail field on demand (never inside `tick`, so it cannot perturb
the simulation):

1. **Threshold.** Combine both species' trail fields and keep only cells above
   `network_threshold` (a fraction of the current peak, default `0.05`). That binary mask is "where
   the network is."
2. **Flood-fill.** Starting from the cells under endpoint 0, breadth-first search across
   above-threshold, non-wall, 4-connected neighbours (wrapping toroidally, like the sim). It uses
   pre-allocated scratch buffers, so it allocates nothing.
3. **Read out.**
   - `endpoints_connected()` — how many endpoints (including the start) the flood reached. When it
     equals `endpoint_count()`, the network spans them all.
   - `network_cost()` — how many cells the flood visited: the size of the connecting structure, a
     proxy for total path length. A leaner network solving the same task scores lower.

Watch `endpoints_connected` flip from `1` to `2` as the trail first bridges two wells, then watch
`network_cost` fall as the colony prunes its early exploratory mesh down toward the shortest route —
the quantitative signature of the maze being solved.

## The maze demo

`load_maze_demo()` assembles all of the above into one reproducible scenario: it clears the random
food patches, lays a set of offset-gap walls into the mask to make a serpentine corridor, places a
food well at each end, sets a positive `food_attraction` so the colony is drawn between them, and
respawns the agents in the open cells of the entry corridor. Because it rebuilds the world it is a
reset-class call — re-fetch the field, food, and obstacle views afterwards, just as you would after
`reset`. From the same seed it runs identically every time, which is what makes "watch it solve the
maze" a repeatable demonstration rather than a lucky frame.

## Determinism preserved

Geography is additive: a world with no walls, no endpoints, and `food_attraction = 0` produces
**bit-for-bit** the same run as before any of this existed. The wall-collision turn is the only new
PRNG draw, and it fires only when an agent is genuinely blocked; the food term is added only when
`food_attraction` is non-zero. With the defaults, neither path is ever taken, so the original
golden-checksum test still passes unchanged — and the maze demo has its own pinned checksum, so it
too is guarded against accidental drift.
