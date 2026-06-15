# The rendering pipeline

The simulation produces scalar fields; the renderer turns them into the glowing picture. The guiding
idea (see decision D3 in
[`docs/DESIGN.md`](https://github.com/VitalyVorobyev/petri-polis/blob/main/docs/DESIGN.md)) is that
**the renderer is the product** — the trail field is colour-mapped and bloomed entirely on the GPU.
This chapter is an overview of how a field becomes light. The renderer lives in `app/src/main.ts` and
uses WebGL2.

## Fields to textures

Each trail field is a single-channel `f32` buffer in WASM memory. The renderer reads it as a
`Float32Array` view (see [WASM and the zero-copy field](./wasm.md)) and uploads it to the GPU as an
**`R32F` texture** — one each for species 0, species 1, and the shared food field. Because the views
alias WASM memory directly, this upload is the only copy that happens, and it is a GPU texture upload,
not a serialization.

## The colour-map pass

Each species' field is normalized by a per-species **auto-exposure** gain (roughly `1 / field_max`,
smoothed across frames so brightness doesn't flicker as the network grows). The normalized intensity is
run through a **bioluminescent palette**:

- **Species 0** maps through a **cyan** LUT (near-black → teal → cyan → white).
- **Species 1** maps through a **magenta** LUT.
- The two are **added**, so where the networks overlap the colour blends toward white.
- The **food field** renders underneath as a dim, desaturated substrate — depletion darkens it and
  regrowth re-greens it, so the boom/bust cycle is legible — kept below the bloom threshold so only the
  trails glow.

## Geography: walls and endpoints

When the world has geography (see [Sources, sinks & mazes](./geography.md)), two more things reach
the picture. The **obstacle mask** uploads as an `R8` texture; wall cells render as a dim slate
underlay, kept below the bloom threshold so they read as solid barriers rather than glowing. Each
**endpoint** is drawn as a soft additive amber ring at its position, so the sources and sinks the
colony is meant to connect are visible at a glance. Both live in the same pipeline — the walls
under the trails, the rings over the bloom — so the structure stays legible without a separate mode.

## Bloom

To make the bright trails read as light rather than flat colour, a **bloom** pass runs:

1. **Bright-pass** — threshold the composited image so only the brightest parts (the trails) pass.
2. **Blur** — a separable Gaussian, ping-ponged between half-resolution framebuffers a few times, to
   spread the bright parts into a soft halo.
3. **Composite** — add the blurred halo back over the base image.

The result is presented to a full-bleed canvas over a near-black background. The "beauty bar" is smooth
gradients and soft additive glow, with no hard pixel edges at the target zoom.

## Why this split

The renderer deliberately knows nothing about the rules — it only consumes fields and metrics. That
keeps the simulation testable in plain Rust (no GPU needed) and keeps the render passes small and
readable. The same fields also drive the inspector and metrics: point queries (`trail_at`, `food_at`,
`nearest_agent`) feed the hover readout, and the folded-in metrics (see
[Ecology and population dynamics](./ecology.md)) feed the sparklines and the CSV/JSON export, all keyed
to `tick_count` so a measured run is reproducible.

For the design rationale behind WebGL2-with-bloom over a Canvas2D dots renderer, and Tweakpane over
React for the controls, see the decisions log (D3, D8) in
[`docs/DESIGN.md`](https://github.com/VitalyVorobyev/petri-polis/blob/main/docs/DESIGN.md).
