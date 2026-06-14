// Hover inspector: on pointermove over the WebGL canvas, map cursor → sim
// coordinates and read trail/food/nearest-agent values from the Rust sim.
// The readout is a fixed-position DOM overlay (pointer-events: none).
// Call attachInspector() once after the canvas and sim are ready.

import type { Sim } from "./wasm/petri_wasm.js";

export function attachInspector(
  canvas: HTMLCanvasElement,
  readout: HTMLDivElement,
  getSimRef: () => Sim,
): void {
  canvas.addEventListener("pointermove", (e: PointerEvent) => {
    const sim = getSimRef();
    const rect = canvas.getBoundingClientRect();
    const u = (e.clientX - rect.left) / rect.width;
    const v = (e.clientY - rect.top) / rect.height;
    // Match the same Y-flip used by the click-to-spawn handler.
    const sx = u * sim.width();
    const sy = (1 - v) * sim.height();

    const t0 = sim.trail_at(0, sx, sy);
    const t1 = sim.trail_at(1, sx, sy);
    const food = sim.food_at(sx, sy);

    // Read nearest agent and its properties in one synchronous block
    // (indices are invalidated by the next tick/spawn/reset).
    const idx = sim.nearest_agent(sx, sy);
    let agentLine = "no agent";
    if (idx >= 0) {
      const asp = sim.agent_species(idx);
      const aen = sim.agent_energy(idx);
      const ax = sim.agent_x(idx);
      const ay = sim.agent_y(idx);
      const dx = toroidalDist1D(sx, ax, sim.width());
      const dy = toroidalDist1D(sy, ay, sim.height());
      const dist = Math.sqrt(dx * dx + dy * dy).toFixed(1);
      const spName = asp === 0 ? "cyan" : "magenta";
      agentLine = `${spName} agent  energy ${aen.toFixed(2)}  dist ${dist}`;
    }

    const cx = Math.floor(sx).toString().padStart(3);
    const cy = Math.floor(sy).toString().padStart(3);

    readout.textContent =
      `cell (${cx},${cy})\n` +
      `cyan trail  ${t0.toFixed(3)}\n` +
      `mag trail   ${t1.toFixed(3)}\n` +
      `food        ${food.toFixed(3)}\n` +
      agentLine;
    readout.style.display = "block";
  });

  canvas.addEventListener("pointerleave", () => {
    readout.style.display = "none";
  });
}

function toroidalDist1D(a: number, b: number, size: number): number {
  const d = Math.abs(a - b);
  return Math.min(d, size - d);
}
