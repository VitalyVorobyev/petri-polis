# The preset gallery

Every structure in this guide is reachable by hand from the control panel — but finding the
right corner of the parameter space takes patience. **Presets** are the lab bench: named
scenarios that load a known structure in one click, so the toy doubles as a gallery of classical
complex-systems demos you can step through and compare.

## What a preset is

A preset is a complete **scenario**: a seed, both species' Physarum parameters and ecology, the
chemotaxis strength, the reachability threshold, any geometry (walls and endpoint food wells),
and how the colony is spawned. Picking one from the **Presets** menu applies all of it at once —
it resets the world, pushes every parameter through the live setters, rebuilds the geometry
(clearing any previous walls and endpoints first), and refreshes the panel to show the loaded
values. The canonical structure forms within seconds.

## The starter gallery

- **Coexistence** — the default two-species tuning: a fine cyan mesh interwoven with coarse
  magenta veins, sharing one food field without either excluding the other.
- **Competitive exclusion** — the two species made parameter-for-parameter identical, so the
  niche separation that sustains coexistence is gone and one overruns the other.
- **Capillary mesh** — short sensors, wide sensing, sharp turns: dense, bushy, fine branching.
- **Trunk roads** — long sensors, narrow sensing, gentle turns, heavy long-lived trail: a few
  thick veins instead of a mesh.
- **Spirals** — over-cranked rotation the agents can't steer straight out of, so the network
  curls into rotating cells.
- **Boom/bust oscillator** — an ecology tuned for strong population cycles: eat out, crash,
  recover, repeat.
- **Maze** — the serpentine-wall scenario with two food wells and chemotaxis (see
  [Sources, sinks & mazes](./geography.md)).
- **Tokyo rail** — no walls, a scatter of food wells standing in for cities; the colony weaves a
  transport network among them, the classic Physarum result that inspired studies of the Tokyo
  rail map.

## Sharing a scenario

The same codec that powers the **Copy link** button now carries a whole scenario in the URL
hash: the seed, both species' parameters and ecology, the chemotaxis strength and reachability
threshold, the list of endpoint wells, and a compact **tag** for any built-in procedural
geometry (the maze's walls are regenerated from the tag rather than shipping the whole mask).
Opening the link replays the scenario exactly, so a structure you found — preset or hand-tuned —
travels as a single link. (Walls you paint by hand are not packed into the link; only the
gallery's built-in geometry and the endpoint wells are, which keeps the URL short while letting
every gallery preset round-trip.)
