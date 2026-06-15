# Structure metrics

The [metrics](./ecology.md) shown so far — trail mass, population, food — measure *how much* there
is. They can't tell a connected network from a field of disconnected speckle, because both can
carry the same total mass. **Structure metrics** measure *form* instead: they read the shape of
the trail field and turn "is it a network yet?" into numbers you can plot and sweep. Like the
[reachability](./geography.md) readout, they are computed on demand over the thresholded trail
field — read-only, never inside the tick — so they cost nothing until you ask and never perturb
the simulation.

All of them work on the **foreground**: the cells where the combined trail rises above a fraction
of its peak (the same `network_threshold` knob the maze uses), with walls excluded and the world
treated as a torus.

## The numbers

- **Connected components** — how many separate pieces the foreground breaks into (a union-find
  over neighbouring foreground cells). Scattered speckle has hundreds; a single consolidated
  network has one. This is the hard "is it a network yet?" number.
- **Loops** — the count of independent cycles in the network, from the grid graph's first Betti
  number `b₁ = E − V + C` (edges minus vertices plus components). A tree has none; a mesh with
  redundant routes has many. Loops rise as the network goes from branching filaments to a
  vascular web.
- **Fractal dimension** — how space-filling the structure is, by box-counting: cover the
  foreground with boxes at several sizes and take the slope of `log(count)` against `log(1/size)`.
  A thin curve sits near 1, a plane-filling mat near 2; the value between them is a single clean
  scalar for "how dense."
- **Autocorrelation length** — the grain size: the distance over which the field stays correlated
  with itself, one number that's small for a fine capillary mesh and large for coarse trunks.

## Watching a phase transition

The point of measuring form is that form changes *suddenly*. Drag `decay` slowly from 0.7 toward
0.99 and there is a threshold where the field stops being speckle and snaps into a connected
network — the component count falls off a cliff and the loop count climbs. That collapse, plotted
live in the sparklines and exported in the CSV, is a **phase transition** you can point at: the
same kind of order parameter crossing a critical value that a physicist would measure. It is the
clearest evidence that the simple local rules are producing genuine collective structure, and the
quantity a headless parameter sweep turns into a phase diagram.
