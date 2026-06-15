# Evolution: heritable traits

Every agent of a species has so far shared one fixed strategy — the species' `Params`. Evolution
loosens that: it makes a trait **heritable**, carried by each individual and passed to its
offspring with a small change. Once a trait varies and is inherited, selection does the rest — the
agents that happen to forage better in this food landscape leave more offspring, and the trait
distribution drifts toward what the landscape rewards. No reinforcement learning, no gradients,
no learned policy: just heredity, mutation, and differential survival. Darwin, not backprop.

## The mechanism

The evolvable trait is **`sensor_distance`** — how far ahead an agent looks. With evolution
enabled for a species, each agent carries its own value instead of the species default, and senses
with it. When an agent reproduces, the child inherits the parent's `sensor_distance` plus a small
random mutation (its size set by a `mutation_strength` knob, drawn from the simulation's seeded
RNG and clamped to a sane range). Agents that sense at a distance the food landscape favours reach
food, survive, and reproduce more, so their value spreads; less-fit values fade. Over a run the
population's mean trait shifts and its spread narrows or widens — a distribution you can watch and
plot.

Keeping it to a single trait is deliberate: one heritable number gives a legible
one-dimensional distribution to watch drift, where evolving a dozen at once would just be noise.

## Reproducible evolution

Because the mutation is drawn from the same seeded PRNG as everything else, evolution is
**deterministic**: the same seed and the same world replay the *entire* evolutionary trajectory,
mutation for mutation. You can watch a lineage drift, reset, and watch the identical drift again —
which means an evolutionary run is a reproducible experiment, not a one-off. (And like every other
addition, evolution is off by default and byte-identical when off: enabling it per species is what
turns on the per-individual trait and the mutation draw.)

## Seeing it

Two views make the drift legible: a **trait-distribution readout** (the population's mean and
spread over time, plotted in the sparklines) shows *that* it drifts; and a **trait map** render
mode colors the network by the local evolved trait, showing *where* each strategy lives — short-
and long-sighted lineages settling into different parts of the food landscape.
