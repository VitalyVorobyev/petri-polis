// Shareable seed + params URL codec.
// Encoding: JSON → btoa → location.hash "#s=<encoded>"
// A shared link reproduces the same run on load because the decoded seed
// and params are applied before the first tick.

import type { Sim } from "./wasm/petri_wasm.js";

// ---------------------------------------------------------------------------
// Shape of the encoded state
// ---------------------------------------------------------------------------

interface SpeciesState {
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
}

export interface SharedState {
  seed: number;
  species: SpeciesState[];
}

// ---------------------------------------------------------------------------
// Encode current sim state into location.hash
// ---------------------------------------------------------------------------

export function encodeState(sim: Sim, seed: number): string {
  const state: SharedState = {
    seed,
    species: [],
  };
  const n = sim.species_count();
  for (let s = 0; s < n; s++) {
    state.species.push({
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
    });
  }
  return `s=${btoa(JSON.stringify(state))}`;
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
  return obj as unknown as SharedState;
}

function isSpeciesState(v: unknown): v is SpeciesState {
  if (typeof v !== "object" || v === null) return false;
  const keys: (keyof SpeciesState)[] = [
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
  return keys.every((k) => typeof obj[k] === "number");
}

// ---------------------------------------------------------------------------
// Apply a decoded state to the sim (and sync the mutable param objects so
// Tweakpane shows the loaded values).
// ---------------------------------------------------------------------------

export interface SimParamObjects {
  allParams: Array<{
    sensor_angle: number;
    sensor_distance: number;
    rotation_angle: number;
    step_size: number;
    deposit: number;
    decay: number;
    diffuse_weight: number;
  }>;
  allEcology: Array<{
    metabolism: number;
    eat_rate: number;
    repro_threshold: number;
    food_regrow: number;
    death_return: number;
  }>;
}

export function applySharedState(
  sim: Sim,
  state: SharedState,
  paramObjects: SimParamObjects,
): void {
  const n = Math.min(state.species.length, sim.species_count());
  for (let s = 0; s < n; s++) {
    const sp = state.species[s];

    // Push into the mutable objects that Tweakpane is bound to.
    Object.assign(paramObjects.allParams[s], {
      sensor_angle: sp.sensor_angle,
      sensor_distance: sp.sensor_distance,
      rotation_angle: sp.rotation_angle,
      step_size: sp.step_size,
      deposit: sp.deposit,
      decay: sp.decay,
      diffuse_weight: sp.diffuse_weight,
    });
    Object.assign(paramObjects.allEcology[s], {
      metabolism: sp.metabolism,
      eat_rate: sp.eat_rate,
      repro_threshold: sp.repro_threshold,
      food_regrow: sp.food_regrow,
      death_return: sp.death_return,
    });

    // Push into the sim.
    sim.set_params(
      s,
      sp.sensor_angle,
      sp.sensor_distance,
      sp.rotation_angle,
      sp.step_size,
      sp.deposit,
      sp.decay,
    );
    sim.set_diffuse_weight(s, sp.diffuse_weight);
    sim.set_ecology(
      s,
      sp.metabolism,
      sp.eat_rate,
      sp.repro_threshold,
      sp.food_regrow,
      sp.death_return,
    );
  }
}
