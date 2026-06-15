# Cross-species sensing

In the [two-species](./two-species.md) model each agent senses and deposits only into its own
trail, and the species touch only through the shared food field. That single point of contact
gives competition, but the two networks never *see* each other. Cross-species sensing opens a
direct channel: each species can read the other's trail and steer by it — toward it or away from
it — and from that one signed weight come territories, predator and prey, and chasing fronts.

## The sensing matrix

Sensing is governed by a small matrix. For an agent of species `s`, the trail it perceives at a
sensor point is a weighted sum over every species' field:

```text
sensed = Σ_o  cross_sense[s][o] · field[o][point]
```

`cross_sense` defaults to the **identity** matrix — each species reads only its own trail, with
weight 1 and weight 0 for the other — which is exactly the original behaviour. The interesting
dynamics live in the **off-diagonal** weights, `cross_sense[s][o]` for `s ≠ o`:

- **Positive** → species `s` is *attracted* to species `o`'s trail and steers up its gradient.
- **Negative** → species `s` is *repelled* and steers away.

Because the default is the identity matrix, a run with no cross-sensing is bit-for-bit identical
to one from before the feature existed: the simulation only takes the weighted-sum path when a
weight is actually changed, so determinism and the golden-checksum guarantee are untouched.

## Territories and predator/prey

Two signs, two classic regimes:

- **Mutual avoidance** — both off-diagonals negative. Each species steers away from the other's
  trail, so the two networks refuse to overlap and settle into separate **territories** with a
  contested boundary between them. (The *Territories* preset.) The segregation is measurable:
  the spatial overlap of the two trail fields drops sharply compared with the identity matrix.
- **Asymmetric pursuit** — the predator is *attracted* to the prey's trail while the prey is
  *repelled* by the predator's. The predator's network chases the prey's, the prey's flees, and
  you get a moving front of **pursuit** that neither species produces alone. (The *Predator/prey*
  preset.)

These are a few lines of arithmetic in the sensing step and two new numbers on the control panel,
but they turn two indifferent species sharing a pantry into an ecology that interacts —
the doorway to richer behaviour the trail-only model can't reach.
