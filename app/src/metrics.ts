// Metrics ring buffer and sparkline renderer.
// One sample is pushed per frame (keyed to tick_count so the x-axis is
// reproducible regardless of frame rate). The sparkline canvas is a small
// overlay drawn each frame — it never touches WebGL state.

export interface MetricSample {
  tick: number;
  pop: [number, number]; // species 0, species 1 live agent counts
  mass: [number, number]; // trail mass (unbounded f64)
  foodTotal: number; // total food remaining (f64, bounded by ceiling)
  foodCoverage: number; // fraction of cells with food, [0, 1]
  connected: number; // endpoints reachable from endpoint 0 (incl. itself)
  endpointCount: number; // total endpoints
  networkCost: number; // cells in the reachable network (unbounded)
  // Structure metrics — heavier O(N) reductions, sampled on a throttled cadence
  // (the last value is reused between samples, so successive samples may repeat).
  componentCount: number; // connected components in the thresholded foreground (gate)
  loopCount: number; // independent loops (first Betti number b1) of the foreground
  fractalDimension: number; // box-counting dimension of the foreground, ~1..2
  autocorrLength: number; // trail-field autocorrelation (grain) length, in cells
}

const RING_CAP = 600;

export class MetricsBuffer {
  private buf: MetricSample[] = [];
  private head = 0; // index of the oldest sample (write position)
  private count = 0;

  // Running maxima for unbounded series (trail mass).
  maxMass: [number, number] = [1, 1];

  // Running maximum for the unbounded network-cost series.
  maxNetworkCost = 1;

  // Running maximum for the component-count series (unbounded above; collapses
  // toward 1 at the decay phase transition — that collapse is the gate signal).
  maxComponentCount = 1;

  // Food ceiling: total food right after new/reset (food starts full).
  foodCeiling = 1;

  push(s: MetricSample): void {
    if (this.count < RING_CAP) {
      this.buf.push(s);
      this.count++;
    } else {
      this.buf[this.head] = s;
      this.head = (this.head + 1) % RING_CAP;
    }
    if (s.mass[0] > this.maxMass[0]) this.maxMass[0] = s.mass[0];
    if (s.mass[1] > this.maxMass[1]) this.maxMass[1] = s.mass[1];
    if (s.networkCost > this.maxNetworkCost) this.maxNetworkCost = s.networkCost;
    if (s.componentCount > this.maxComponentCount) this.maxComponentCount = s.componentCount;
  }

  // Chronological ordered snapshot (oldest → newest).
  ordered(): MetricSample[] {
    if (this.count < RING_CAP) return this.buf.slice();
    return [...this.buf.slice(this.head), ...this.buf.slice(0, this.head)];
  }

  clear(): void {
    this.buf = [];
    this.head = 0;
    this.count = 0;
    this.maxMass = [1, 1];
    this.maxNetworkCost = 1;
    this.maxComponentCount = 1;
    // foodCeiling is reset by the caller after querying the new value.
  }

  get length(): number {
    return this.count;
  }
}

// ---------------------------------------------------------------------------
// Sparkline canvas overlay
// ---------------------------------------------------------------------------

// Draw all sparklines onto the overlay canvas. Called each frame.
export function drawSparklines(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  buf: MetricsBuffer,
  latest: MetricSample | null,
): void {
  ctx.clearRect(0, 0, w, h);
  if (!latest || buf.length === 0) return;

  const samples = buf.ordered();

  // Layout: 6 sparkline rows (each CELL_H px tall with a GAP between) followed
  // by a compact two-line structure-metrics readout.
  const PAD_L = 6;
  const PAD_R = 6;
  const PAD_T = 6;
  const CELL_H = 34;
  const GAP = 4;
  const LABEL_H = 12;
  const plotW = w - PAD_L - PAD_R;

  // Background panel.
  ctx.fillStyle = "rgba(4,8,16,0.72)";
  roundRect(ctx, 0, 0, w, h, 6);
  ctx.fill();

  const rowY = (row: number) => PAD_T + row * (CELL_H + GAP);

  // --- Row 0: Population ---
  {
    const top = rowY(0);
    const maxPop = Math.max(1, ...samples.map((s) => s.pop[0]), ...samples.map((s) => s.pop[1]));
    drawLabel(ctx, PAD_L, top, `pop  cyan ${latest.pop[0]}  mag ${latest.pop[1]}`);
    drawLine(
      ctx,
      samples,
      PAD_L,
      top + LABEL_H,
      plotW,
      CELL_H - LABEL_H,
      (s) => s.pop[0] / maxPop,
      "#22d3c8",
      1.2,
    );
    drawLine(
      ctx,
      samples,
      PAD_L,
      top + LABEL_H,
      plotW,
      CELL_H - LABEL_H,
      (s) => s.pop[1] / maxPop,
      "#d943a8",
      1.2,
    );
  }

  // --- Row 1: Trail mass ---
  {
    const top = rowY(1);
    const maxM0 = Math.max(1, buf.maxMass[0]);
    const maxM1 = Math.max(1, buf.maxMass[1]);
    const maxM = Math.max(maxM0, maxM1);
    drawLabel(ctx, PAD_L, top, `mass  cyan ${fmtSI(latest.mass[0])}  mag ${fmtSI(latest.mass[1])}`);
    drawLine(
      ctx,
      samples,
      PAD_L,
      top + LABEL_H,
      plotW,
      CELL_H - LABEL_H,
      (s) => s.mass[0] / maxM,
      "#22d3c8",
      1.2,
    );
    drawLine(
      ctx,
      samples,
      PAD_L,
      top + LABEL_H,
      plotW,
      CELL_H - LABEL_H,
      (s) => s.mass[1] / maxM,
      "#d943a8",
      1.2,
    );
  }

  // --- Row 2: Food total ---
  {
    const top = rowY(2);
    const ceil = Math.max(1, buf.foodCeiling);
    const pct = Math.round((latest.foodTotal / ceil) * 100);
    drawLabel(ctx, PAD_L, top, `food total  ${pct}%`);
    drawLine(
      ctx,
      samples,
      PAD_L,
      top + LABEL_H,
      plotW,
      CELL_H - LABEL_H,
      (s) => s.foodTotal / ceil,
      "#5ca832",
      1.2,
    );
  }

  // --- Row 3: Food coverage ---
  {
    const top = rowY(3);
    const pct = Math.round(latest.foodCoverage * 100);
    drawLabel(ctx, PAD_L, top, `food coverage  ${pct}%`);
    drawLine(
      ctx,
      samples,
      PAD_L,
      top + LABEL_H,
      plotW,
      CELL_H - LABEL_H,
      (s) => s.foodCoverage,
      "#5ca832",
      1.2,
    );
  }

  // --- Row 4: Reachability (connected endpoints + network cost) ---
  {
    const top = rowY(4);
    const label =
      latest.endpointCount > 0
        ? `connected ${latest.connected}/${latest.endpointCount}  cost ${fmtSI(latest.networkCost)}`
        : "connected —  (no endpoints)";
    drawLabel(ctx, PAD_L, top, label);
    const maxCost = Math.max(1, buf.maxNetworkCost);
    drawLine(
      ctx,
      samples,
      PAD_L,
      top + LABEL_H,
      plotW,
      CELL_H - LABEL_H,
      (s) => s.networkCost / maxCost,
      "#f0b429",
      1.2,
    );
  }

  // --- Row 5: Component count (the gate metric) ---
  // High for a scattered speckle of blobs, collapsing toward 1 as the network
  // links up — the collapse at the decay phase transition is the gate signal.
  {
    const top = rowY(5);
    const maxC = Math.max(1, buf.maxComponentCount);
    drawLabel(ctx, PAD_L, top, `components  ${latest.componentCount}`);
    drawLine(
      ctx,
      samples,
      PAD_L,
      top + LABEL_H,
      plotW,
      CELL_H - LABEL_H,
      (s) => s.componentCount / maxC,
      "#9f7aea",
      1.2,
    );
  }

  // --- Structure readout: loops, fractal dimension, autocorrelation length ---
  // The remaining structure metrics as a compact two-line text readout below
  // the last sparkline (no extra rows — these read as single numbers).
  {
    const top = rowY(6);
    drawLabel(
      ctx,
      PAD_L,
      top,
      `loops ${latest.loopCount}   D ${latest.fractalDimension.toFixed(2)}`,
    );
    drawLabel(ctx, PAD_L, top + LABEL_H, `autocorr ${latest.autocorrLength.toFixed(1)} cells`);
  }
}

// Draw a thin sparkline within a (x, y, w, h) box.
function drawLine(
  ctx: CanvasRenderingContext2D,
  samples: MetricSample[],
  x: number,
  y: number,
  w: number,
  h: number,
  value: (s: MetricSample) => number,
  color: string,
  lineWidth: number,
): void {
  if (samples.length < 2) return;
  ctx.beginPath();
  ctx.strokeStyle = color;
  ctx.lineWidth = lineWidth;
  ctx.globalAlpha = 0.8;
  for (let i = 0; i < samples.length; i++) {
    const px = x + (i / (samples.length - 1)) * w;
    const py = y + h - value(samples[i]) * h;
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  }
  ctx.stroke();
  ctx.globalAlpha = 1.0;
}

function drawLabel(ctx: CanvasRenderingContext2D, x: number, y: number, text: string): void {
  ctx.font = "9px ui-monospace, SFMono-Regular, Menlo, monospace";
  ctx.fillStyle = "rgba(180,220,220,0.65)";
  ctx.fillText(text, x, y + 9);
}

function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
): void {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.lineTo(x + w - r, y);
  ctx.arcTo(x + w, y, x + w, y + r, r);
  ctx.lineTo(x + w, y + h - r);
  ctx.arcTo(x + w, y + h, x + w - r, y + h, r);
  ctx.lineTo(x + r, y + h);
  ctx.arcTo(x, y + h, x, y + h - r, r);
  ctx.lineTo(x, y + r);
  ctx.arcTo(x, y, x + r, y, r);
  ctx.closePath();
}

// SI suffix for large numbers (trail mass can be in the millions).
function fmtSI(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}G`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
  return n.toFixed(0);
}

// ---------------------------------------------------------------------------
// Export helpers
// ---------------------------------------------------------------------------

const CSV_HEADER =
  "tick,pop0,pop1,mass0,mass1,food_total,food_coverage," +
  "component_count,loop_count,fractal_dimension,autocorrelation_length";

function sampleToCSV(s: MetricSample): string {
  return [
    s.tick,
    s.pop[0],
    s.pop[1],
    s.mass[0].toFixed(2),
    s.mass[1].toFixed(2),
    s.foodTotal.toFixed(2),
    s.foodCoverage.toFixed(4),
    s.componentCount,
    s.loopCount,
    s.fractalDimension.toFixed(4),
    s.autocorrLength.toFixed(4),
  ].join(",");
}

export function exportCSV(buf: MetricsBuffer, seed: number): void {
  const rows = [CSV_HEADER, ...buf.ordered().map(sampleToCSV)].join("\n");
  triggerDownload(`petri-metrics-seed${seed}.csv`, "text/csv", rows);
}

export function exportJSON(buf: MetricsBuffer, seed: number): void {
  const data = JSON.stringify(buf.ordered(), null, 2);
  triggerDownload(`petri-metrics-seed${seed}.json`, "application/json", data);
}

function triggerDownload(filename: string, mime: string, content: string): void {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}
