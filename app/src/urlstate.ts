// Shareable scenario URL codec.
// Encoding: JSON → btoa → location.hash "#s=<encoded>"
// A shared link reproduces the same run on load because the decoded seed,
// params, ecology, chemotaxis, reachability threshold, endpoints, and built-in
// geometry tag are applied before the first tick.
//
// Scope line: arbitrary hand-painted wall masks are NOT serialized (too big for
// a URL). Only built-in procedural geometry is captured — by a compact tag
// (e.g. "maze") that is regenerated with `load_maze_demo` on load — plus the
// explicit endpoint list. Every gallery preset round-trips.

import type { Sim } from "./wasm/petri_wasm.js";

// ---------------------------------------------------------------------------
// Shape of a full scenario — the unit a preset describes and a link captures.
// ---------------------------------------------------------------------------

export interface SpeciesState {
  sensor_angle: number;
  sensor_distance: number;
  rotation_angle: number;
  step_size: number;
  deposit: number;
  decay: number;
  diffuse_weight: number;
  metabolism: number;
  eat_rate: number;
  repro_threshold: number;
  food_regrow: number;
  death_return: number;
  food_attraction: number;
}

export interface Endpoint {
  x: number;
  y: number;
  radius: number;
}

// A built-in procedural geometry tag. "none" = open world (or only endpoints);
// "maze" = regenerate the wall maze with `load_maze_demo` on load.
export type GeometryTag = "none" | "maze";

export interface SharedState {
  seed: number;
  species: SpeciesState[];
  network_threshold: number;
  geometry: GeometryTag;
  endpoints: Endpoint[];
}

// ---------------------------------------------------------------------------
// Read the live sim into a SharedState.
// ---------------------------------------------------------------------------

export function readSharedState(sim: Sim, seed: number, geometry: GeometryTag): SharedState {
  const species: SpeciesState[] = [];
  const n = sim.species_count();
  for (let s = 0; s < n; s++) {
    species.push({
      sensor_angle: sim.sensor_angle(s),
      sensor_distance: sim.sensor_distance(s),
      rotation_angle: sim.rotation_angle(s),
      step_size: sim.step_size(s),
      deposit: sim.deposit_amount(s),
      decay: sim.decay(s),
      diffuse_weight: sim.diffuse_weight(s),
      metabolism: sim.metabolism(s),
      eat_rate: sim.eat_rate(s),
      repro_threshold: sim.repro_threshold(s),
      food_regrow: sim.food_regrow(s),
      death_return: sim.death_return(s),
      food_attraction: sim.food_attraction(s),
    });
  }

  const endpoints: Endpoint[] = [];
  const epCount = sim.endpoint_count();
  for (let i = 0; i < epCount; i++) {
    endpoints.push({
      x: sim.endpoint_x(i),
      y: sim.endpoint_y(i),
      radius: sim.endpoint_radius(i),
    });
  }

  return {
    seed,
    species,
    network_threshold: sim.network_threshold(),
    geometry,
    endpoints,
  };
}

// ---------------------------------------------------------------------------
// Encode current sim state into a location.hash payload.
// `geometry` is the built-in geometry tag for the active scenario (the maze's
// walls can't be URL-serialized in full; we regenerate them from the tag).
// ---------------------------------------------------------------------------

export function encodeState(sim: Sim, seed: number, geometry: GeometryTag): string {
  return `s=${btoa(JSON.stringify(readSharedState(sim, seed, geometry)))}`;
}

// ---------------------------------------------------------------------------
// Attempt to decode location.hash; returns null on malformed input.
// ---------------------------------------------------------------------------

export function decodeHash(): SharedState | null {
  try {
    const h = location.hash.slice(1); // drop leading '#'
    if (!h.startsWith("s=")) return null;
    const json = atob(h.slice(2));
    const parsed = JSON.parse(json) as unknown;
    return validateState(parsed);
  } catch {
    return null;
  }
}

function validateState(raw: unknown): SharedState | null {
  if (typeof raw !== "object" || raw === null) return null;
  const obj = raw as Record<string, unknown>;
  if (typeof obj.seed !== "number") return null;
  if (!Array.isArray(obj.species)) return null;
  for (const sp of obj.species as unknown[]) {
    if (!isSpeciesState(sp)) return null;
  }

  // network_threshold, geometry, and endpoints are tolerant: older links that
  // predate them decode with sensible fallbacks.
  if (obj.network_threshold !== undefined && typeof obj.network_threshold !== "number") {
    return null;
  }
  if (obj.geometry !== undefined && obj.geometry !== "none" && obj.geometry !== "maze") {
    return null;
  }
  if (obj.endpoints !== undefined) {
    if (!Array.isArray(obj.endpoints)) return null;
    for (const ep of obj.endpoints as unknown[]) {
      if (!isEndpoint(ep)) return null;
    }
  }

  // Normalize so callers can rely on the optional fields being present.
  // food_attraction defaults to 0 on any species that predates it.
  const species = (obj.species as SpeciesState[]).map((sp) => ({
    ...sp,
    food_attraction: typeof sp.food_attraction === "number" ? sp.food_attraction : 0,
  }));
  const normalized: SharedState = {
    seed: obj.seed as number,
    species,
    network_threshold:
      typeof obj.network_threshold === "number" ? (obj.network_threshold as number) : 0.05,
    geometry: obj.geometry === "maze" ? "maze" : "none",
    endpoints: Array.isArray(obj.endpoints) ? (obj.endpoints as Endpoint[]) : [],
  };
  return normalized;
}

function isSpeciesState(v: unknown): v is SpeciesState {
  if (typeof v !== "object" || v === null) return false;
  const required: (keyof SpeciesState)[] = [
    "sensor_angle",
    "sensor_distance",
    "rotation_angle",
    "step_size",
    "deposit",
    "decay",
    "diffuse_weight",
    "metabolism",
    "eat_rate",
    "repro_threshold",
    "food_regrow",
    "death_return",
  ];
  const obj = v as Record<string, unknown>;
  if (!required.every((k) => typeof obj[k] === "number")) return false;
  // food_attraction is tolerant for backward compatibility (older links omit it).
  if (obj.food_attraction !== undefined && typeof obj.food_attraction !== "number") return false;
  return true;
}

function isEndpoint(v: unknown): v is Endpoint {
  if (typeof v !== "object" || v === null) return false;
  const obj = v as Record<string, unknown>;
  return typeof obj.x === "number" && typeof obj.y === "number" && typeof obj.radius === "number";
}
