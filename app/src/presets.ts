// Preset scenarios — the lab bench.
//
// Each preset is a named, fully-specified scenario: per-species Physarum +
// ecology + chemotaxis params, a reachability threshold, optional built-in
// geometry (the maze) or explicit endpoints (Tokyo rail), and an optional
// pinned seed. Applying a preset fully resets the world to its canonical state
// (see `applyScenario` in main.ts), so each entry reproduces a visibly distinct
// classical complex-systems structure in one click.
//
// Presets are in-code TS data (no storage dependency) and reuse the same
// SharedState shape the share-link codec round-trips, so every preset is
// shareable as a URL.

import type { CrossSense, Endpoint, GeometryTag, SpeciesState } from "./urlstate";

// A preset's scenario: everything `applyScenario` needs. `seed` is optional —
// when omitted, applying the preset keeps the current seed (only param/geometry
// presets care to stay reproducible without pinning a seed). `crossSense` is
// optional too — omitted means identity coupling (off-diagonals 0), so a preset
// without it resets any cross-species coupling left by the previous scenario.
export interface Scenario {
  seed?: number;
  species: SpeciesState[];
  network_threshold: number;
  geometry: GeometryTag;
  endpoints: Endpoint[];
  crossSense?: CrossSense;
  // Optional per-species evolution of the heritable sensor_distance trait. When
  // omitted, `applyScenario` still sets the full state to disabled / strength 0,
  // so switching away from an evolving preset turns evolution off cleanly.
  evolution?: { enabled: [boolean, boolean]; mutation: [number, number] };
}

export interface Preset {
  id: string;
  name: string;
  caption: string;
  scenario: Scenario;
}

// ---------------------------------------------------------------------------
// Shared building blocks
// ---------------------------------------------------------------------------

// The shipped two-species defaults (mirrors crates/petri-core Params/Ecology
// `default_for`). Species 0 = fine cyan mesh, species 1 = coarse magenta veins.
const SPECIES0_DEFAULT: SpeciesState = {
  sensor_angle: 0.46,
  sensor_distance: 7.0,
  rotation_angle: 0.46,
  step_size: 1.0,
  deposit: 4.0,
  decay: 0.88,
  diffuse_weight: 0.5,
  metabolism: 0.0058,
  eat_rate: 0.1,
  repro_threshold: 1.12,
  food_regrow: 0.0042,
  death_return: 0.3,
  food_attraction: 0.0,
};

const SPECIES1_DEFAULT: SpeciesState = {
  sensor_angle: 0.34,
  sensor_distance: 16.0,
  rotation_angle: 0.28,
  step_size: 1.3,
  deposit: 7.0,
  decay: 0.92,
  diffuse_weight: 0.55,
  metabolism: 0.0058,
  eat_rate: 0.1,
  repro_threshold: 1.28,
  food_regrow: 0.0042,
  death_return: 0.3,
  food_attraction: 0.0,
};

const DEFAULT_THRESHOLD = 0.05;

// Clone helpers so each preset gets independent objects (no aliasing).
function s0(over: Partial<SpeciesState> = {}): SpeciesState {
  return { ...SPECIES0_DEFAULT, ...over };
}
function s1(over: Partial<SpeciesState> = {}): SpeciesState {
  return { ...SPECIES1_DEFAULT, ...over };
}

// A ring of "city" endpoints for the Tokyo-rail demo, scattered across the
// 256×256 field (sim cell coords). Mixed radii read as differently-sized cities.
function tokyoEndpoints(): Endpoint[] {
  // 9 cities: a loose central cluster plus a wide outer scatter, so the network
  // has to bridge real distances — the classic Physarum transport-graph setup.
  return [
    { x: 48, y: 60, radius: 7 },
    { x: 128, y: 40, radius: 8 },
    { x: 210, y: 56, radius: 7 },
    { x: 36, y: 138, radius: 6 },
    { x: 120, y: 128, radius: 9 },
    { x: 222, y: 150, radius: 7 },
    { x: 64, y: 214, radius: 7 },
    { x: 150, y: 206, radius: 8 },
    { x: 220, y: 220, radius: 6 },
  ];
}

// ---------------------------------------------------------------------------
// The gallery
// ---------------------------------------------------------------------------

export const PRESETS: Preset[] = [
  {
    id: "coexistence",
    name: "Coexistence",
    caption: "Two niches: a cyan fine mesh interwoven with magenta veins.",
    scenario: {
      species: [s0(), s1()],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: [],
    },
  },
  {
    id: "competitive-exclusion",
    name: "Competitive exclusion",
    caption: "Identical niches — one species overruns the other.",
    scenario: {
      // Species 1 made identical to species 0 (params AND ecology) so the niche
      // separation collapses and the two compete for one strategy.
      species: [s0(), s0()],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: [],
    },
  },
  {
    id: "capillary-mesh",
    name: "Capillary mesh",
    caption: "Fine bushy branching — short sensors, sharp turns, light deposit.",
    scenario: {
      species: [
        s0({ sensor_distance: 5.0, sensor_angle: 0.5, rotation_angle: 0.5, deposit: 2.0 }),
        s1({ sensor_distance: 5.0, sensor_angle: 0.5, rotation_angle: 0.5, deposit: 2.0 }),
      ],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: [],
    },
  },
  {
    id: "trunk-roads",
    name: "Trunk roads",
    caption: "A few thick veins — long sensors, gentle turns, heavy persistent trails.",
    scenario: {
      species: [
        s0({
          sensor_distance: 20.0,
          sensor_angle: 0.2,
          rotation_angle: 0.15,
          deposit: 10.0,
          decay: 0.96,
        }),
        s1({
          sensor_distance: 20.0,
          sensor_angle: 0.2,
          rotation_angle: 0.15,
          deposit: 10.0,
          decay: 0.96,
        }),
      ],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: [],
    },
  },
  {
    id: "spirals",
    name: "Spirals",
    caption: "Over-steering can't lock straight — trails curl into spirals.",
    scenario: {
      species: [
        s0({ rotation_angle: 0.9, sensor_distance: 10.0 }),
        s1({ rotation_angle: 0.9, sensor_distance: 10.0 }),
      ],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: [],
    },
  },
  {
    id: "boom-bust",
    name: "Boom/bust oscillator",
    caption: "Strong population cycles — cheap reproduction, slow regrow, hungry metabolism.",
    scenario: {
      // Lower repro_threshold (split sooner → faster boom), lower food_regrow
      // (slow recovery → deep bust), higher metabolism (steeper crash).
      species: [
        s0({ repro_threshold: 0.8, food_regrow: 0.0022, metabolism: 0.012 }),
        s1({ repro_threshold: 0.85, food_regrow: 0.0022, metabolism: 0.012 }),
      ],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: [],
    },
  },
  {
    id: "maze",
    name: "Maze",
    caption: "Physarum solves the maze — chemotaxis pulls a trail between two food wells.",
    scenario: {
      // The wall maze, its two endpoints, food-attraction, and the seeded
      // population are all baked into `load_maze_demo` (a reset-class call).
      // We still record canonical params so the panel reads sensibly; the maze
      // call then overrides geometry/endpoints/chemotaxis to its built-in state.
      species: [s0({ food_attraction: 6.0 }), s1({ food_attraction: 6.0 })],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "maze",
      endpoints: [],
    },
  },
  {
    id: "tokyo-rail",
    name: "Tokyo rail",
    caption: "A transport graph self-organizes among scattered cities.",
    scenario: {
      // No walls; food-attraction on both species pulls trails between the
      // city food wells, forming the classic Physarum transport network.
      species: [s0({ food_attraction: 5.0 }), s1({ food_attraction: 5.0 })],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: tokyoEndpoints(),
    },
  },
  {
    id: "territories",
    name: "Territories",
    caption: "Mutual avoidance — each species shuns the other's trail, carving separate domains.",
    scenario: {
      // Symmetric repulsion: each species is pushed away from the other's
      // trail, so they segregate into separate regions with a contested seam.
      species: [s0(), s1()],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: [],
      crossSense: { s01: -0.6, s10: -0.6 },
    },
  },
  {
    id: "predator-prey",
    name: "Predator/prey",
    caption: "Magenta hunts cyan's trail while cyan flees — a chase front sweeps the field.",
    scenario: {
      // Asymmetric coupling: predator magenta (s10 > 0) is drawn up prey cyan's
      // trail; prey cyan (s01 < 0) is repelled by the predator's trail and runs.
      // Slightly faster, slower-fading trails make the chase read more clearly.
      species: [s0({ step_size: 1.2, decay: 0.9 }), s1({ step_size: 1.4, decay: 0.92 })],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: [],
      crossSense: { s01: -0.8, s10: 0.8 },
    },
  },
  {
    id: "evolution",
    name: "Evolution",
    caption: "Cyan's sensor reach is heritable and mutates at birth — watch the trait drift.",
    scenario: {
      // The shared-food world, but cyan's sensor_distance is heritable: births
      // mutate it (strength ~1 cell) and selection lets the trait distribution
      // drift away from the default 7 cells. Open the trait sparkline / Trait-map
      // render mode to watch it. Magenta is left non-evolving as a control.
      species: [s0(), s1()],
      network_threshold: DEFAULT_THRESHOLD,
      geometry: "none",
      endpoints: [],
      evolution: { enabled: [true, false], mutation: [1.0, 0.0] },
    },
  },
];

export const DEFAULT_PRESET_ID = "coexistence";

export function presetById(id: string): Preset | undefined {
  return PRESETS.find((p) => p.id === id);
}
