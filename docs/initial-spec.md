> ⚠️ **HISTORICAL — superseded by [`DESIGN.md`](./DESIGN.md).**
> This is the original architect-astronaut MVP spec, kept for reference. The project
> deliberately cut most of it (backend, intent/conflict system, event log, binary snapshots,
> 13-metric history, the 5-species/9-layer big-bang) and reframed around a Rust→WASM **Physarum**
> trail core with a bioluminescent WebGL renderer. See `DESIGN.md`'s decisions log for what
> changed and why. Do not implement from this file.

---

Petri Polis — MVP Specification

1. Project Summary

Petri Polis is a local-first artificial-life simulation where users spawn simple agents into a layered grid world. Agents act locally, consume resources, leave traces, reproduce, mutate, build structures, spread life, create decay, and die.

The goal of the MVP is not intelligence or storytelling. The goal is to produce visible emergent behavior from simple ecological and spatial feedback loops.

Core principle:

Users create local causes. The world creates global patterns.

2. MVP Goals

The MVP should demonstrate:

1. A deterministic grid simulation.
2. Several predefined agent species.
3. Visible emergence: trails, colonies, roads, decay, regrowth.
4. Interactive spawning of agents.
5. Pause/play/speed controls.
6. Basic world and agent inspection.
7. Save/load or deterministic replay support.

The MVP is single-player and local. No multiplayer. No LLM runtime agents. No RL.

3. Non-Goals

The MVP should not include:

* multiplayer
* server backend
* LLM-controlled agents
* online RL training
* complex narrative system
* realistic 3D rendering
* arbitrary user scripting
* large world streaming
* advanced procedural terrain

These can be considered later.

4. World Model

The world is a 2D grid.

Initial size:

256 × 256 cells

Recommended later sizes:

512 × 512
1024 × 1024 with chunks

Each cell contains multiple layers.

4.1 Cell Layers

Minimal cell state:

terrain:   water | soil | sand | rock
life:      none | grass | moss | fungus
built:     none | trail | road | hut | ruin
food:      0..255
material:  0..255
moisture:  0..255
decay:     0..255
signal:    0..255
owner:     none | species_id

Implementation note: use compact arrays, not per-cell objects.

Example:

terrain[width * height]
life[width * height]
built[width * height]
food[width * height]
material[width * height]
moisture[width * height]
decay[width * height]
signal[width * height]
owner[width * height]

Most fields should be u8.

5. Agent Model

Agents are simple entities with local perception and bounded actions.

5.1 Agent State

Each agent has:

id
species_id
x
y
energy
age
cargo_food
cargo_material
state
last_action
generation

Optional later:

parent_id
small memory array
mutation_seed

5.2 Agent Count

Initial target:

1,000 agents

MVP performance target:

3,000 agents at 30 ticks/sec on a normal laptop

Hard cap for first prototype:

5,000 agents

Recommended density:

1 agent per 50–100 cells

6. Species Model

A species defines behavior parameters shared by agents.

Each species has:

id
name
color
max_age
max_energy
metabolism
vision_radius
reproduction_threshold
reproduction_cost
child_energy
mutation_rate
behavior_weights

Behavior should be parameter-based, not hard-coded per individual agent.

7. Initial Species

Implement five predefined species.

7.1 Seeder

Purpose: spreads life.

Behavior:

* prefers moist soil
* plants grass or moss
* avoids high decay
* reproduces when energy is high

7.2 Forager

Purpose: creates trails through food-seeking movement.

Behavior:

* seeks food
* eats food
* leaves signal while moving
* prefers existing signal when no food is visible

7.3 Builder

Purpose: converts repeated movement into structures.

Behavior:

* follows strong signal
* collects material
* converts high-signal cells into trails or roads
* builds huts near roads if material is available

7.4 Cleaner

Purpose: stabilizes dense areas.

Behavior:

* seeks high decay
* reduces decay
* repairs ruins or damaged built cells later
* consumes energy while cleaning

7.5 Corruptor

Purpose: creates ecological pressure.

Behavior:

* seeks decay
* spreads fungus
* increases local decay
* thrives in abandoned or dense areas

8. Simulation Tick

The simulation advances in fixed ticks.

Recommended default:

30 ticks/sec

Supported speeds:

pause
1×
2×
5×
10×

8.1 Tick Order

Use this fixed order:

1. Environment update
2. Signal diffusion and evaporation
3. Agent perception
4. Agent intent generation
5. Conflict resolution
6. Action application
7. Birth/death handling
8. Metrics collection

Important: agents produce intents first. The world applies intents later. Do not mutate the world directly during individual agent decision-making.

9. Environment Rules

9.1 Food Production

Grass and moss produce food slowly.

Example rule:

if life is grass or moss:
    food += small amount

Food is capped at 255.

9.2 Moisture

Water increases moisture in nearby cells.

Simple MVP rule:

water cells have moisture = 255
neighboring soil slowly gains moisture
moisture slowly evaporates

9.3 Decay

Decay increases from:

dead agents
dense agent activity
corruptor actions
abandoned huts

Decay decreases naturally over time, unless reinforced.

High decay should:

reduce grass/moss growth
support fungus
damage built structures

9.4 Signals

Agents can leave signal.

Signal should:

diffuse slightly
evaporate every tick
accumulate on frequently used paths

Repeated signal can become trail. Trail can later become road.

10. Agent Mechanics

10.1 Energy

Energy is the main survival variable.

Each tick:

energy -= metabolism

Actions may add or remove energy:

move:       -1
eat food:   +20
plant:      -5
build:      -10
clean:      -8
reproduce:  -reproduction_cost

10.2 Death

An agent dies when:

energy <= 0
age > max_age

On death:

food += small amount
decay += medium amount
remove agent

Death should feed the world. This is required for ecological feedback.

10.3 Reproduction

An agent may reproduce if:

energy >= reproduction_threshold
empty neighboring cell exists

On reproduction:

create child nearby
parent.energy -= reproduction_cost
child.energy = child_energy
child.generation = parent.generation + 1
child species parameters may mutate

10.4 Mutation

Mutation should affect numeric species/agent parameters slightly.

Initial mutation candidates:

metabolism
vision_radius
reproduction_threshold
movement_bias
food_preference
signal_following_weight
decay_avoidance_weight
build_tendency

Do not mutate all parameters at once. Use small bounded mutations.

11. Agent Decision Model

Use weighted utility, not RL.

Each agent evaluates possible actions:

move north
move south
move east
move west
eat
plant
build
clean
reproduce
rest

Each action receives a score based on local features.

Example local features:

food_here
near_food
near_signal
near_water
cell_decay
cell_moisture
has_material
near_road
energy_low
empty_neighbor_available

Decision rule:

choose highest-scoring action with small randomness

Randomness is required to avoid frozen behavior.

All random choices must use deterministic seeded RNG.

12. Intents and Conflict Resolution

Agents produce intents.

Example intents:

Move(agent_id, target_x, target_y)
Eat(agent_id, x, y)
Plant(agent_id, x, y, life_type)
Build(agent_id, x, y, built_type)
Clean(agent_id, x, y)
EmitSignal(agent_id, x, y, amount)
Reproduce(agent_id, x, y)
Die(agent_id)
Rest(agent_id)

Resolve conflicts after all intents are collected.

Conflict examples:

multiple agents move to same cell
one agent plants while another builds
multiple agents consume same food
multiple agents clean/corrupt same cell

Initial resolution rules:

movement conflict: random winner
food conflict: split or random winner
built layer beats life layer
clean and corrupt subtract from each other
signal emissions accumulate

13. Emergent Feedback Loops

The MVP must support these loops.

13.1 Trail to Road Loop

food attracts foragers
foragers leave signal
signal accumulates into trails
builders follow trails
builders convert strong trails into roads
roads reduce movement cost
more agents use roads

13.2 Density Collapse Loop

many agents gather
food is depleted
decay increases
life dies
agents starve
settlement collapses
ruins remain

13.3 Recovery Loop

dead agents add food and decay
decay slowly fades
seeders spread grass
grass produces food
new colony appears

13.4 Corruption/Cleaning Loop

decay attracts corruptors
corruptors spread fungus and decay
cleaners reduce decay
stable regions survive
neglected regions collapse

14. State Tracking

Track three kinds of state.

14.1 Current State

The current state is the full simulation state:

tick
rng_state
world arrays
agents
species
metrics

14.2 Event Log

The event log stores user/system events:

tick: spawn species at x,y
tick: place beacon at x,y
tick: change speed
tick: save snapshot

The event log should be sufficient for deterministic replay from an initial seed and snapshot.

14.3 Metrics History

Collect low-frequency metrics:

population per species
births per species
deaths per species
total food
total decay
total life cells
total built cells
road length
hut count
ruin count
average energy
average age

Metrics are used for debugging and later UI plots.

15. Persistence

For MVP, persistence can be local.

Required:

save snapshot
load snapshot
reset world from seed

Snapshot must include:

version
tick
seed
rng_state
width
height
world arrays
agents
species
metrics summary

Preferred format:

binary for world arrays
JSON or MessagePack for metadata

JSON-only is acceptable for first prototype if simpler.

16. UI Requirements

The MVP UI should contain:

main world canvas
spawn tool panel
simulation controls
cell/agent inspector
basic metrics display
save/load/reset controls

16.1 World Canvas

Render:

terrain
life
built structures
decay overlay
signals/trails
agents
selected cell/agent highlight

Do not render cells as React components. Use canvas or WebGL.

16.2 Spawn Tools

User can spawn predefined species:

Seeder
Forager
Builder
Cleaner
Corruptor

Spawn interaction:

select species
click map
spawn N agents around click

Default spawn count:

20 agents

16.3 Inspector

Clicking a cell shows:

x, y
terrain
life
built
food
material
moisture
decay
signal
owner
agents on/near cell

Clicking an agent shows:

id
species
energy
age
generation
cargo
last action
current local features if available

16.4 Simulation Controls

Required controls:

pause/play
step one tick
speed selector
reset
save
load

17. Rendering Requirements

First renderer:

Canvas2D

Render world layers into an image buffer.

Recommended order:

terrain base color
life tint
built overlay
decay darkening/tint
signal glow or brightness
agents as small dots
selection overlay

Rendering target:

smooth enough at 256×256 and 512×512
visually readable before visually fancy

18. Determinism Requirements

Given:

same initial seed
same initial snapshot
same event log
same simulation version

The simulation should produce the same result.

Required:

seeded RNG
fixed tick order
no wall-clock-dependent simulation logic
stable iteration order
versioned snapshots

19. Suggested Project Structure

Implementation details are open, but this structure is recommended:

petri-polis/
  README.md
  SPEC.md
  crates/
    petri-core/
      src/
        world.rs
        cell.rs
        agent.rs
        species.rs
        behavior.rs
        intent.rs
        simulation.rs
        environment.rs
        metrics.rs
        snapshot.rs
        rng.rs
    petri-wasm/
      src/
        lib.rs
  apps/
    web/
      src/
        App.tsx
        renderer/
        controls/
        inspector/
        simulation/

Alternative: TypeScript-only prototype is acceptable if speed of iteration is preferred.

20. Acceptance Criteria

The MVP is acceptable when:

1. A user can open the app and see a generated 256×256 world.
2. A user can spawn each of five species by clicking.
3. Simulation can run, pause, step, and reset.
4. Agents consume energy, move, act, reproduce, mutate, and die.
5. Dead agents affect food/decay.
6. Trails emerge from repeated movement.
7. Builders can convert trails into roads or structures.
8. Decay can damage or suppress life.
9. Cleaners and corruptors visibly affect decay.
10. Inspector shows useful cell and agent state.
11. The same seed produces repeatable behavior.
12. Snapshot save/load works.
13. The simulation remains interactive with at least 1,000 agents.

21. First Implementation Milestones

Milestone 1: Simulation Core

Deliver:

grid world
agent list
species definitions
seeded RNG
tick loop
movement
energy
death
basic reproduction

Milestone 2: Renderer and Controls

Deliver:

canvas renderer
pause/play/step
speed control
spawn agents by click
reset world

Milestone 3: Environment Feedback

Deliver:

food growth
moisture
decay
signals
signal evaporation
death feeds world

Milestone 4: Species Behavior

Deliver:

Seeder
Forager
Builder
Cleaner
Corruptor

Milestone 5: Inspection and Persistence

Deliver:

cell inspector
agent inspector
metrics
save/load snapshot
deterministic seed replay

22. Critical Design Constraints

Keep the first version simple.

Do not add intelligence before ecology works.

The simulation should be built around:

local perception
bounded action
energy economy
reproduction
mutation
shared world fields
death
decay
regrowth

If these loops are interesting, the project has a foundation. If these loops are not interesting, adding LLMs, RL, multiplayer, or narrative will not fix it.
