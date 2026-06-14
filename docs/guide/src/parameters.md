# Parameter reference

Every knob in Petri Polis is **live** — changeable mid-run through `set_params` (Physarum) or
`set_ecology` (ecology), per species, with no rebuild and no reallocation. The renderer binds them to
Tweakpane sliders; the simulation simply reads the current value each tick. This chapter catalogues
them, with the default for each species and what each does to the emergent picture.

## Physarum parameters (`Params`)

Set together via `set_params(species, sensor_angle, sensor_distance, rotation_angle, step_size,
deposit, decay)`, plus `set_diffuse_weight(species, weight)`.

| Parameter | Units | Species 0 | Species 1 | Effect |
|---|---|---|---|---|
| `sensor_angle` | radians | 0.46 (~26°) | 0.34 (~19°) | Splay of the side sensors. Wider → bushy, branching networks; narrower → straighter trunks. |
| `sensor_distance` | cells | 7.0 | 16.0 | How far ahead the agent looks. Short → fine, closely-spaced mesh; long → coarse, far-apart veins. |
| `rotation_angle` | radians | 0.46 | 0.28 | How sharply the agent turns when steering. Larger → tight, twisty capillaries; smaller → smooth, sweeping curves. |
| `step_size` | cells/tick | 1.0 | 1.3 | How far the agent moves per tick. Larger → faster exploration, looser trails. |
| `deposit_amount` | field units | 4.0 | 7.0 | Trail laid per tick. Higher → brighter, more persistent, thicker trails that recruit more agents. |
| `decay` | multiplier (0,1) | 0.88 | 0.92 | Per-tick evaporation (`field *= decay`). Closer to 1 → long-lived, slowly-fading structure; smaller → fast, twitchy turnover. |
| `diffuse_weight` | weight [0,1] | 0.5 | 0.55 | Centre-tap weight of the separable blur. 1.0 → no diffusion (speckle); lower → softer, more spread glow. |

The species-0 set weaves a dense capillary mesh; the species-1 set lays a few broad trunk routes. The
contrast between them is the whole point — two visually distinct systems sharing a frame (see
[Two species](./two-species.md)).

### How they interact

These knobs are not independent. The look of a network is mostly set by two balances:

- **Deposit vs decay** sets how bright and persistent trails are. Raise deposit or push decay toward 1
  and trails thicken and linger; the network gets denser and slower to change. Lower them and it gets
  faint and quick to reshape.
- **Sensor distance vs sensor/rotation angle** sets the *grain*. Long sensors with gentle turns find
  and commit to distant trails → coarse trunks. Short sensors with sharp turns react to nearby trail →
  fine mesh.

`step_size` and `diffuse_weight` are the finishing touches: step trades exploration speed for trail
continuity, and diffuse weight trades crisp filaments for soft glow.

## Ecology parameters (`Ecology`)

Set together via `set_ecology(species, metabolism, eat_rate, repro_threshold, food_regrow,
death_return)`.

| Parameter | Units | Species 0 | Species 1 | Effect |
|---|---|---|---|---|
| `metabolism` | energy/tick | 0.0058 | 0.0058 | Cost of living, paid every tick. Higher → smaller sustainable population, faster busts. |
| `eat_rate` | food/tick | 0.10 | 0.10 | Max food eaten from the current cell per tick (1:1 to energy). Higher → faster patch depletion. |
| `repro_threshold` | energy | 1.12 | 1.28 | Energy needed to split. Lower → reproduces sooner, faster booms; higher → longer build-up. |
| `food_regrow` | fraction/tick (0,1) | 0.0042 | 0.0042 | Per-tick pull of the food field toward its cap. The shared field uses the **mean** of both species' values. Faster → quicker recovery, tighter cycles. |
| `death_return` | food units | 0.30 | 0.30 | Nutrient returned to the cell where an agent dies. Higher → faster local recovery after a die-off. |

By default the two species share `metabolism`, `eat_rate`, `food_regrow`, and `death_return`, and
differ mainly in `repro_threshold`, which staggers their cycles. Together these set the **period and
amplitude** of the boom/bust oscillation (see [Ecology and population dynamics](./ecology.md)).

## Structural constants (rebuild required)

A few values are fixed at compile time because they define the world's structure rather than tuning it:

- **`SPECIES = 2`** — number of species.
- **`DEFAULT_AGENT_CAPACITY = 200_000`** — reserved agent slots and the spawn cap; bounds the
  population and guarantees the zero-copy invariant.
- **Food-patch constants** — count (16), peak (1.0), and radius range — defining the patchy landscape.

Changing these means editing `crates/petri-core/src/lib.rs` and rebuilding the WASM; everything in the
tables above is reachable live from the running toy.
