//! `sweep` — headless parameter sweep → phase diagram for the Petri Polis sim.
//!
//! A native, browser-free batch runner: it varies one Physarum knob across `N`
//! values, runs each value under `M` seeds for `T` ticks on a small grid, reads an
//! **order parameter** off the settled field — by default
//! [`Sim::component_count`], which is high (fragmented speckle) when the network is
//! shattered and low (a few large blobs) when it consolidates — and emits:
//!
//! - a **CSV** with one row per `(knob value, seed)` run plus an aggregated block
//!   (mean / std / min / max of the order parameter over the seeds), and
//! - a dependency-free **SVG** line plot of the mean order parameter vs the knob,
//!   with ±std error bars, so a regime boundary (a bifurcation / collapse) is
//!   visible at a glance.
//!
//! The whole thing is `std`-only — no plotting crate, no rayon. Parallelism across
//! the independent runs uses [`std::thread`]. The sim is deterministic, and this
//! tool adds no wall-clock and no RNG of its own, so **the same config produces a
//! byte-identical CSV** (and SVG) every time.
//!
//! # Usage
//!
//! ```text
//! cargo run --release --bin sweep -- [options]
//!
//!   --knob <name>        knob to vary: decay | deposit | sensor-angle |
//!                        sensor-distance | step | diffuse      [default: decay]
//!   --start <f32>        first knob value                      [default: knob-specific]
//!   --end <f32>          last  knob value                      [default: knob-specific]
//!   --steps <N>          number of knob values, inclusive      [default: 20]
//!   --seeds <M>          number of seeds per knob value         [default: 6]
//!   --ticks <T>          ticks settled before measuring         [default: 3000]
//!   --grid <G>           sim is G×G cells                        [default: 256]
//!   --metric <name>      order parameter: components | loops |
//!                        fractal | trail-mass                   [default: components]
//!   --threads <K>        worker threads (0 = auto)               [default: 0]
//!   --csv <path>         CSV output path                         [default: sweep.csv]
//!   --svg <path>         SVG output path                         [default: sweep.svg]
//!
//!   # 2-D sweep (bonus): adds a second knob → heatmap SVG.
//!   --knob2 <name>       second knob to vary (enables 2-D mode)
//!   --start2/--end2/--steps2   range + count for the second knob
//! ```
//!
//! The default sweep (`decay` from 0.80 to 0.98, 20 values × 6 seeds × 3000 ticks
//! at 256×256) shows `component_count` **collapsing** as `decay` rises — the live
//! phase transition from a fragmented mesh (~14 components at fast fade) to a
//! consolidated network (~4 at slow fade). The single decay value is applied to
//! both species, so the whole world shifts along the same axis.
//!
//! ## Grid-size choice
//! The default is the renderer's `256×256`, which reproduces the clean, well-spaced
//! ~14→~4 collapse. A sweep is many runs, but they are independent, so the runner
//! parallelizes across them with [`std::thread`] and the ~120-run default still
//! finishes in well under a minute on a multi-core machine. Pass `--grid 128` for a
//! roughly 4× faster sweep: the collapse stays visible (fewer absolute components,
//! ~6→~1) but the small field compresses the dynamic range, so `256` is the default
//! for a legible phase diagram.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use petri_core::{Params, Sim, SPECIES};

/// Which Physarum knob a sweep varies. Each variant maps a scalar value onto a
/// field of [`Params`], applied to **every** species so the whole world shifts
/// together (the order parameter reads the combined field).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Knob {
    Decay,
    Deposit,
    SensorAngle,
    SensorDistance,
    Step,
    Diffuse,
}

impl Knob {
    /// Parse a CLI knob name. Returns `None` for an unknown name.
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "decay" => Knob::Decay,
            "deposit" => Knob::Deposit,
            "sensor-angle" => Knob::SensorAngle,
            "sensor-distance" => Knob::SensorDistance,
            "step" => Knob::Step,
            "diffuse" => Knob::Diffuse,
            _ => return None,
        })
    }

    /// Human-readable axis label for the SVG / CSV header.
    fn label(self) -> &'static str {
        match self {
            Knob::Decay => "decay",
            Knob::Deposit => "deposit_amount",
            Knob::SensorAngle => "sensor_angle",
            Knob::SensorDistance => "sensor_distance",
            Knob::Step => "step_size",
            Knob::Diffuse => "diffuse_weight",
        }
    }

    /// A sensible default `[start, end]` range for this knob — the window where the
    /// interesting behavior lives — used when the CLI omits `--start` / `--end`.
    fn default_range(self) -> (f32, f32) {
        match self {
            Knob::Decay => (0.80, 0.98),
            Knob::Deposit => (1.0, 12.0),
            Knob::SensorAngle => (0.10, 1.20),
            Knob::SensorDistance => (3.0, 24.0),
            Knob::Step => (0.5, 2.5),
            Knob::Diffuse => (0.20, 1.00),
        }
    }

    /// Write this knob's value into `p`.
    fn apply(self, p: &mut Params, value: f32) {
        match self {
            Knob::Decay => p.decay = value,
            Knob::Deposit => p.deposit_amount = value,
            Knob::SensorAngle => p.sensor_angle = value,
            Knob::SensorDistance => p.sensor_distance = value,
            Knob::Step => p.step_size = value,
            Knob::Diffuse => p.diffuse_weight = value,
        }
    }
}

/// Which scalar the sweep records off each settled run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Metric {
    Components,
    Loops,
    Fractal,
    TrailMass,
}

impl Metric {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "components" => Metric::Components,
            "loops" => Metric::Loops,
            "fractal" => Metric::Fractal,
            "trail-mass" => Metric::TrailMass,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Metric::Components => "component_count",
            Metric::Loops => "loop_count",
            Metric::Fractal => "fractal_dimension",
            Metric::TrailMass => "trail_mass",
        }
    }

    /// Read the order parameter off a settled sim.
    fn read(self, sim: &mut Sim) -> f64 {
        match self {
            Metric::Components => sim.component_count() as f64,
            Metric::Loops => sim.loop_count() as f64,
            Metric::Fractal => sim.fractal_dimension() as f64,
            Metric::TrailMass => (0..SPECIES).map(|s| sim.trail_mass(s)).sum(),
        }
    }
}

/// A fully-resolved sweep configuration (defaults applied, CLI parsed).
#[derive(Clone, Debug)]
struct Config {
    knob: Knob,
    start: f32,
    end: f32,
    steps: usize,
    seeds: usize,
    ticks: usize,
    grid: usize,
    metric: Metric,
    threads: usize,
    csv_path: String,
    svg_path: String,
    // 2-D (bonus): when `knob2` is `Some`, the sweep becomes a grid → heatmap SVG.
    knob2: Option<Knob>,
    start2: f32,
    end2: f32,
    steps2: usize,
}

impl Config {
    /// The default 1-D `decay` sweep.
    fn default_for(knob: Knob) -> Self {
        let (start, end) = knob.default_range();
        Self {
            knob,
            start,
            end,
            steps: 20,
            seeds: 6,
            ticks: 3000,
            grid: 256,
            metric: Metric::Components,
            threads: 0,
            csv_path: "sweep.csv".to_string(),
            svg_path: "sweep.svg".to_string(),
            knob2: None,
            start2: 0.0,
            end2: 0.0,
            steps2: 0,
        }
    }
}

/// The result of one `(knob value, seed)` run.
#[derive(Clone, Copy)]
struct RunResult {
    /// Index into the (row-major) job grid; used to scatter results back in order.
    job: usize,
    value: f32,
    value2: f32,
    seed: u64,
    metric: f64,
    trail_mass: f64,
    coexistence: bool,
}

/// One settled run: build a `grid×grid` sim seeded with `seed`, set the knob(s) on
/// every species, tick `ticks` times, then read the order parameter and companions.
/// Deterministic and self-contained — no shared state, no RNG of its own.
fn run_one(cfg: &Config, value: f32, value2: f32, seed: u64) -> (f64, f64, bool) {
    let mut sim = Sim::new(cfg.grid, cfg.grid, seed);
    for s in 0..SPECIES {
        let mut p = sim.params(s);
        cfg.knob.apply(&mut p, value);
        if let Some(k2) = cfg.knob2 {
            k2.apply(&mut p, value2);
        }
        sim.set_params(s, p);
    }
    for _ in 0..cfg.ticks {
        sim.tick();
    }
    let metric = cfg.metric.read(&mut sim);
    let trail_mass: f64 = (0..SPECIES).map(|s| sim.trail_mass(s)).sum();
    // Coexistence: both species still have a living population.
    let coexistence = (0..SPECIES).all(|s| sim.species_population(s) > 0);
    (metric, trail_mass, coexistence)
}

/// Linear ramp of `n` inclusive values from `start` to `end` (single point for
/// `n == 1`). Computed in `f64` then narrowed so the endpoints are exact.
fn linspace(start: f32, end: f32, n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![start];
    }
    let (s, e) = (start as f64, end as f64);
    (0..n)
        .map(|i| {
            let t = i as f64 / (n - 1) as f64;
            (s + (e - s) * t) as f32
        })
        .collect()
}

/// Aggregate statistics of the order parameter over the seeds at one knob value.
#[derive(Clone, Copy)]
struct Agg {
    value: f32,
    value2: f32,
    mean: f64,
    std: f64,
    min: f64,
    max: f64,
    coexist_frac: f64,
}

/// Population-std aggregation of a slice of per-seed metric values.
fn aggregate(value: f32, value2: f32, metrics: &[f64], coexist: &[bool]) -> Agg {
    let n = metrics.len().max(1) as f64;
    let mean = metrics.iter().sum::<f64>() / n;
    let var = metrics.iter().map(|m| (m - mean) * (m - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    let min = metrics.iter().copied().fold(f64::INFINITY, f64::min);
    let max = metrics.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let coexist_frac = coexist.iter().filter(|&&c| c).count() as f64 / n;
    Agg {
        value,
        value2,
        mean,
        std,
        min: if min.is_finite() { min } else { 0.0 },
        max: if max.is_finite() { max } else { 0.0 },
        coexist_frac,
    }
}

fn main() {
    let cfg = match parse_args() {
        Ok(cfg) => cfg,
        Err(msg) => {
            eprintln!("sweep: {msg}\n");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };

    let values = linspace(cfg.start, cfg.end, cfg.steps);
    let values2 = if cfg.knob2.is_some() {
        linspace(cfg.start2, cfg.end2, cfg.steps2)
    } else {
        vec![0.0]
    };
    let cells = values.len() * values2.len();
    let total_runs = cells * cfg.seeds;

    eprintln!(
        "sweep: knob={} [{:.4}..{:.4}] x{}{} | seeds={} ticks={} grid={}x{} metric={}",
        cfg.knob.label(),
        cfg.start,
        cfg.end,
        cfg.steps,
        match cfg.knob2 {
            Some(k2) => format!(
                " | knob2={} [{:.4}..{:.4}] x{}",
                k2.label(),
                cfg.start2,
                cfg.end2,
                cfg.steps2
            ),
            None => String::new(),
        },
        cfg.seeds,
        cfg.ticks,
        cfg.grid,
        cfg.grid,
        cfg.metric.label(),
    );
    eprintln!(
        "sweep: {total_runs} runs ({cells} cells x {} seeds)",
        cfg.seeds
    );

    // --- Build the flat job list. Determinism: jobs are laid out in a fixed order
    // (value2-major, then value, then seed), each carries its own seed, and results
    // scatter back by `job` index — so the worker-thread count never changes the
    // output. Seeds are `0..M` per cell. ---
    let mut jobs = Vec::with_capacity(total_runs);
    for &v2 in &values2 {
        for &v in &values {
            for seed in 0..cfg.seeds as u64 {
                jobs.push(Job {
                    value: v,
                    value2: v2,
                    seed,
                });
            }
        }
    }

    let started = Instant::now();
    let results = run_jobs(&cfg, &jobs);
    let elapsed = started.elapsed();

    eprintln!(
        "sweep: {total_runs} runs in {:.2}s ({:.1} ms/run)",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / total_runs as f64,
    );

    // --- Aggregate across seeds per cell (value2-major, value-minor). ---
    let mut aggs = Vec::with_capacity(cells);
    let mut metric_buf: Vec<f64> = Vec::with_capacity(cfg.seeds);
    let mut coexist_buf: Vec<bool> = Vec::with_capacity(cfg.seeds);
    for c in 0..cells {
        metric_buf.clear();
        coexist_buf.clear();
        let base = c * cfg.seeds;
        for r in &results[base..base + cfg.seeds] {
            metric_buf.push(r.metric);
            coexist_buf.push(r.coexistence);
        }
        let v = results[base].value;
        let v2 = results[base].value2;
        aggs.push(aggregate(v, v2, &metric_buf, &coexist_buf));
    }

    // --- Emit. ---
    let csv = render_csv(&cfg, &results, &aggs);
    if let Err(e) = std::fs::write(&cfg.csv_path, csv) {
        eprintln!("sweep: failed to write CSV to {}: {e}", cfg.csv_path);
        std::process::exit(1);
    }
    let svg = if cfg.knob2.is_some() {
        render_heatmap_svg(&cfg, &aggs, &values, &values2)
    } else {
        render_line_svg(&cfg, &aggs)
    };
    if let Err(e) = std::fs::write(&cfg.svg_path, svg) {
        eprintln!("sweep: failed to write SVG to {}: {e}", cfg.svg_path);
        std::process::exit(1);
    }

    eprintln!("sweep: wrote {} and {}", cfg.csv_path, cfg.svg_path);

    // --- A compact stdout summary so the collapse is visible without opening files. ---
    println!(
        "# {} vs {} (mean +/- std over {} seeds)",
        cfg.metric.label(),
        cfg.knob.label(),
        cfg.seeds
    );
    if cfg.knob2.is_none() {
        println!(
            "{:>14}  {:>10}  {:>9}  {:>6}  {:>6}  {:>9}",
            cfg.knob.label(),
            "mean",
            "std",
            "min",
            "max",
            "coexist"
        );
        for a in &aggs {
            println!(
                "{:>14.4}  {:>10.3}  {:>9.3}  {:>6.0}  {:>6.0}  {:>8.0}%",
                a.value,
                a.mean,
                a.std,
                a.min,
                a.max,
                a.coexist_frac * 100.0,
            );
        }
    }
}

/// One unit of work: a single `(knob value, [knob2 value], seed)` run.
#[derive(Clone, Copy)]
struct Job {
    value: f32,
    value2: f32,
    seed: u64,
}

/// Run every job, returning a `Vec<RunResult>` whose `i`-th entry is the result of
/// `jobs[i]` (scattered back by job index, so order is independent of threading).
fn run_jobs(cfg: &Config, jobs: &[Job]) -> Vec<RunResult> {
    let total = jobs.len();
    // Auto thread count: cap at the job count and at the available parallelism.
    let want = if cfg.threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    } else {
        cfg.threads
    };
    let nthreads = want.clamp(1, total.max(1));

    // Pre-sized results, one slot per job; results scatter back by job index so the
    // output order is independent of how the work was split across threads.
    let placeholder = RunResult {
        job: usize::MAX,
        value: 0.0,
        value2: 0.0,
        seed: 0,
        metric: 0.0,
        trail_mass: 0.0,
        coexistence: false,
    };
    let mut results = vec![placeholder; total];

    if nthreads <= 1 {
        for (i, j) in jobs.iter().enumerate() {
            results[i] = exec(cfg, j, i);
        }
        return results;
    }

    // Shared, lock-free work queue: an atomic cursor each worker fetch-and-adds to
    // claim the next job index. Scoped threads borrow `cfg`/`jobs` directly (no
    // 'static bound, no clone). Each worker collects into a thread-local Vec; the
    // results are scattered back into their job slots after the join.
    let cursor = AtomicUsize::new(0);
    let cfg_ref = &cfg;
    let jobs_ref = jobs;
    let per_thread: Vec<Vec<RunResult>> = std::thread::scope(|scope| {
        let cursor = &cursor;
        let mut handles = Vec::with_capacity(nthreads);
        for _ in 0..nthreads {
            handles.push(scope.spawn(move || {
                let mut local = Vec::new();
                loop {
                    let i = cursor.fetch_add(1, Ordering::Relaxed);
                    if i >= total {
                        break;
                    }
                    local.push(exec(cfg_ref, &jobs_ref[i], i));
                }
                local
            }));
        }
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    for chunk in per_thread {
        for r in chunk {
            results[r.job] = r;
        }
    }
    results
}

/// Execute one job into a [`RunResult`] tagged with its slot index `i`.
fn exec(cfg: &Config, j: &Job, i: usize) -> RunResult {
    let (metric, trail_mass, coexistence) = run_one(cfg, j.value, j.value2, j.seed);
    RunResult {
        job: i,
        value: j.value,
        value2: j.value2,
        seed: j.seed,
        metric,
        trail_mass,
        coexistence,
    }
}

/// Render the CSV: a per-run block (one row per knob value × seed) followed by an
/// aggregated block (mean / std / min / max / coexistence-fraction per cell).
fn render_csv(cfg: &Config, results: &[RunResult], aggs: &[Agg]) -> String {
    let mut s = String::new();
    let knob = cfg.knob.label();
    let metric = cfg.metric.label();

    // Header comment block — self-documenting config so a CSV is reproducible.
    s.push_str("# petri-polis parameter sweep\n");
    s.push_str(&format!(
        "# knob={knob} metric={metric} seeds={} ticks={} grid={}x{}\n",
        cfg.seeds, cfg.ticks, cfg.grid, cfg.grid
    ));
    if let Some(k2) = cfg.knob2 {
        s.push_str(&format!("# knob2={}\n", k2.label()));
    }

    // Per-run rows.
    if let Some(k2) = cfg.knob2 {
        s.push_str(&format!(
            "section,{knob},{},seed,{metric},trail_mass,coexistence\n",
            k2.label()
        ));
        for r in results {
            s.push_str(&format!(
                "run,{:.6},{:.6},{},{},{:.6},{}\n",
                r.value, r.value2, r.seed, r.metric, r.trail_mass, r.coexistence as u8
            ));
        }
    } else {
        s.push_str(&format!(
            "section,{knob},seed,{metric},trail_mass,coexistence\n"
        ));
        for r in results {
            s.push_str(&format!(
                "run,{:.6},{},{},{:.6},{}\n",
                r.value, r.seed, r.metric, r.trail_mass, r.coexistence as u8
            ));
        }
    }

    // Aggregated rows.
    if let Some(k2) = cfg.knob2 {
        s.push_str(&format!(
            "section,{knob},{},{metric}_mean,{metric}_std,{metric}_min,{metric}_max,coexist_frac\n",
            k2.label()
        ));
        for a in aggs {
            s.push_str(&format!(
                "agg,{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}\n",
                a.value, a.value2, a.mean, a.std, a.min, a.max, a.coexist_frac
            ));
        }
    } else {
        s.push_str(&format!(
            "section,{knob},{metric}_mean,{metric}_std,{metric}_min,{metric}_max,coexist_frac\n"
        ));
        for a in aggs {
            s.push_str(&format!(
                "agg,{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}\n",
                a.value, a.mean, a.std, a.min, a.max, a.coexist_frac
            ));
        }
    }

    s
}

// ----------------------------------------------------------------------------
// SVG rendering. Plain-text, dependency-free. A simple Cartesian plot with a
// margined plot box, labeled/ticked axes, a mean polyline, ±std error bars, and a
// title. Numbers are formatted to a few decimals; coordinates are f32 → string.
// ----------------------------------------------------------------------------

/// Overall SVG canvas size and plot-box margins (px).
const SVG_W: f64 = 800.0;
const SVG_H: f64 = 520.0;
const MARGIN_L: f64 = 86.0;
const MARGIN_R: f64 = 28.0;
const MARGIN_T: f64 = 56.0;
const MARGIN_B: f64 = 70.0;

/// Format an f64 with a fixed number of decimals, trimming for tick labels.
fn fmt(v: f64, dp: usize) -> String {
    format!("{v:.*}", dp)
}

/// XML-escape the handful of characters that matter in SVG text/attribute content.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// "Nice" tick step for a `[lo, hi]` range targeting roughly `target` ticks —
/// rounds the raw step up to the nearest 1 / 2 / 5 × 10^k.
fn nice_step(range: f64, target: usize) -> f64 {
    if range <= 0.0 {
        return 1.0;
    }
    let raw = range / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let nice = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

/// Render the 1-D line plot: mean order parameter vs the knob, with ±std bars.
fn render_line_svg(cfg: &Config, aggs: &[Agg]) -> String {
    let plot_w = SVG_W - MARGIN_L - MARGIN_R;
    let plot_h = SVG_H - MARGIN_T - MARGIN_B;

    // Data domain. X is the knob range; Y spans mean±std with a little padding.
    let x_lo = cfg.start as f64;
    let x_hi = cfg.end as f64;
    let (x_lo, x_hi) = if (x_hi - x_lo).abs() < 1e-9 {
        (x_lo - 0.5, x_hi + 0.5)
    } else {
        (x_lo, x_hi)
    };

    let mut y_lo = f64::INFINITY;
    let mut y_hi = f64::NEG_INFINITY;
    for a in aggs {
        y_lo = y_lo.min(a.mean - a.std).min(a.min);
        y_hi = y_hi.max(a.mean + a.std).max(a.max);
    }
    if !y_lo.is_finite() || !y_hi.is_finite() {
        y_lo = 0.0;
        y_hi = 1.0;
    }
    // Pad Y by ~8% and always include 0 so the collapse is read against a baseline.
    let span = (y_hi - y_lo).max(1e-6);
    y_hi += span * 0.08;
    y_lo = (y_lo - span * 0.08).min(0.0);
    if (y_hi - y_lo).abs() < 1e-9 {
        y_hi = y_lo + 1.0;
    }

    // Coordinate transforms (data → px). Y is flipped (SVG origin top-left).
    let sx = |x: f64| MARGIN_L + (x - x_lo) / (x_hi - x_lo) * plot_w;
    let sy = |y: f64| MARGIN_T + plot_h - (y - y_lo) / (y_hi - y_lo) * plot_h;

    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{SVG_W}\" height=\"{SVG_H}\" viewBox=\"0 0 {SVG_W} {SVG_H}\" font-family=\"sans-serif\">\n"
    ));
    svg.push_str(&format!(
        "<rect width=\"{SVG_W}\" height=\"{SVG_H}\" fill=\"#0e1116\"/>\n"
    ));

    // Title + subtitle.
    let title = format!("{} vs {}", pretty_metric(cfg.metric), cfg.knob.label());
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"30\" fill=\"#e6edf3\" font-size=\"20\" font-weight=\"bold\">{}</text>\n",
        MARGIN_L,
        esc(&title)
    ));
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"48\" fill=\"#9aa7b3\" font-size=\"12\">mean +/- std over {} seeds, {} ticks, {}x{} grid</text>\n",
        MARGIN_L, cfg.seeds, cfg.ticks, cfg.grid, cfg.grid
    ));

    // Plot frame.
    svg.push_str(&format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#161b22\" stroke=\"#30363d\"/>\n",
        MARGIN_L, MARGIN_T, plot_w, plot_h
    ));

    // --- Gridlines + ticks. ---
    let x_step = nice_step(x_hi - x_lo, 8);
    let mut gx = (x_lo / x_step).ceil() * x_step;
    while gx <= x_hi + 1e-9 {
        let px = sx(gx);
        svg.push_str(&format!(
            "<line x1=\"{px:.2}\" y1=\"{}\" x2=\"{px:.2}\" y2=\"{}\" stroke=\"#21262d\"/>\n",
            MARGIN_T,
            MARGIN_T + plot_h
        ));
        svg.push_str(&format!(
            "<text x=\"{px:.2}\" y=\"{:.2}\" fill=\"#9aa7b3\" font-size=\"11\" text-anchor=\"middle\">{}</text>\n",
            MARGIN_T + plot_h + 18.0,
            fmt(gx, decimals_for(x_step))
        ));
        gx += x_step;
    }
    let y_step = nice_step(y_hi - y_lo, 6);
    let mut gy = (y_lo / y_step).ceil() * y_step;
    while gy <= y_hi + 1e-9 {
        let py = sy(gy);
        svg.push_str(&format!(
            "<line x1=\"{}\" y1=\"{py:.2}\" x2=\"{}\" y2=\"{py:.2}\" stroke=\"#21262d\"/>\n",
            MARGIN_L,
            MARGIN_L + plot_w
        ));
        svg.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{py:.2}\" fill=\"#9aa7b3\" font-size=\"11\" text-anchor=\"end\" dominant-baseline=\"middle\">{}</text>\n",
            MARGIN_L - 8.0,
            fmt(gy, decimals_for(y_step))
        ));
        gy += y_step;
    }

    // --- Axis labels. ---
    svg.push_str(&format!(
        "<text x=\"{:.2}\" y=\"{:.2}\" fill=\"#e6edf3\" font-size=\"13\" text-anchor=\"middle\">{}</text>\n",
        MARGIN_L + plot_w / 2.0,
        SVG_H - 22.0,
        esc(cfg.knob.label())
    ));
    let y_label_x = 22.0;
    let y_label_y = MARGIN_T + plot_h / 2.0;
    svg.push_str(&format!(
        "<text x=\"{y_label_x:.2}\" y=\"{y_label_y:.2}\" fill=\"#e6edf3\" font-size=\"13\" text-anchor=\"middle\" transform=\"rotate(-90 {y_label_x:.2} {y_label_y:.2})\">{}</text>\n",
        esc(pretty_metric(cfg.metric))
    ));

    // --- Error bars (±std), drawn under the mean line. ---
    for a in aggs {
        let px = sx(a.value as f64);
        let top = sy(a.mean + a.std);
        let bot = sy(a.mean - a.std);
        svg.push_str(&format!(
            "<line x1=\"{px:.2}\" y1=\"{top:.2}\" x2=\"{px:.2}\" y2=\"{bot:.2}\" stroke=\"#3fb4c9\" stroke-width=\"1.4\"/>\n"
        ));
        // Caps.
        svg.push_str(&format!(
            "<line x1=\"{:.2}\" y1=\"{top:.2}\" x2=\"{:.2}\" y2=\"{top:.2}\" stroke=\"#3fb4c9\" stroke-width=\"1.4\"/>\n",
            px - 3.5,
            px + 3.5
        ));
        svg.push_str(&format!(
            "<line x1=\"{:.2}\" y1=\"{bot:.2}\" x2=\"{:.2}\" y2=\"{bot:.2}\" stroke=\"#3fb4c9\" stroke-width=\"1.4\"/>\n",
            px - 3.5,
            px + 3.5
        ));
    }

    // --- Mean polyline. ---
    let mut pts = String::new();
    for a in aggs {
        pts.push_str(&format!("{:.2},{:.2} ", sx(a.value as f64), sy(a.mean)));
    }
    svg.push_str(&format!(
        "<polyline points=\"{}\" fill=\"none\" stroke=\"#e85d9c\" stroke-width=\"2.2\"/>\n",
        pts.trim()
    ));
    // Mean markers.
    for a in aggs {
        svg.push_str(&format!(
            "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"3.2\" fill=\"#e85d9c\"/>\n",
            sx(a.value as f64),
            sy(a.mean)
        ));
    }

    svg.push_str("</svg>\n");
    svg
}

/// Render the 2-D heatmap (bonus): mean order parameter over a `knob × knob2` grid,
/// colored low→high, with axis ticks and a small color legend.
fn render_heatmap_svg(cfg: &Config, aggs: &[Agg], values: &[f32], values2: &[f32]) -> String {
    let plot_w = SVG_W - MARGIN_L - MARGIN_R - 70.0; // leave room for the legend
    let plot_h = SVG_H - MARGIN_T - MARGIN_B;
    let nx = values.len().max(1);
    let ny = values2.len().max(1);
    let cw = plot_w / nx as f64;
    let ch = plot_h / ny as f64;

    let mut v_lo = f64::INFINITY;
    let mut v_hi = f64::NEG_INFINITY;
    for a in aggs {
        v_lo = v_lo.min(a.mean);
        v_hi = v_hi.max(a.mean);
    }
    if !v_lo.is_finite() {
        v_lo = 0.0;
        v_hi = 1.0;
    }
    let vspan = (v_hi - v_lo).max(1e-9);

    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{SVG_W}\" height=\"{SVG_H}\" viewBox=\"0 0 {SVG_W} {SVG_H}\" font-family=\"sans-serif\">\n"
    ));
    svg.push_str(&format!(
        "<rect width=\"{SVG_W}\" height=\"{SVG_H}\" fill=\"#0e1116\"/>\n"
    ));
    let title = format!(
        "{} over {} x {}",
        pretty_metric(cfg.metric),
        cfg.knob.label(),
        cfg.knob2.map(|k| k.label()).unwrap_or("")
    );
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"30\" fill=\"#e6edf3\" font-size=\"20\" font-weight=\"bold\">{}</text>\n",
        MARGIN_L,
        esc(&title)
    ));
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"48\" fill=\"#9aa7b3\" font-size=\"12\">mean over {} seeds, {} ticks, {}x{} grid</text>\n",
        MARGIN_L, cfg.seeds, cfg.ticks, cfg.grid, cfg.grid
    ));

    // Cells. aggs are laid out value2-major (row), value-minor (col).
    for (row, _) in values2.iter().enumerate() {
        for (col, _) in values.iter().enumerate() {
            let a = &aggs[row * nx + col];
            let t = (a.mean - v_lo) / vspan;
            let (r, g, b) = viridis(t);
            // Row 0 is the bottom of the plot (small knob2 at bottom).
            let px = MARGIN_L + col as f64 * cw;
            let py = MARGIN_T + (ny - 1 - row) as f64 * ch;
            svg.push_str(&format!(
                "<rect x=\"{px:.2}\" y=\"{py:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"rgb({r},{g},{b})\"/>\n",
                cw + 0.5,
                ch + 0.5
            ));
        }
    }

    // Frame.
    svg.push_str(&format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"none\" stroke=\"#30363d\"/>\n",
        MARGIN_L, MARGIN_T, plot_w, plot_h
    ));

    // X ticks (knob): label a handful.
    let xticks = tick_indices(nx);
    for &i in &xticks {
        let px = MARGIN_L + (i as f64 + 0.5) * cw;
        svg.push_str(&format!(
            "<text x=\"{px:.2}\" y=\"{:.2}\" fill=\"#9aa7b3\" font-size=\"11\" text-anchor=\"middle\">{}</text>\n",
            MARGIN_T + plot_h + 16.0,
            fmt(values[i] as f64, 3)
        ));
    }
    // Y ticks (knob2).
    let yticks = tick_indices(ny);
    for &i in &yticks {
        let py = MARGIN_T + (ny - 1 - i) as f64 * ch + ch / 2.0;
        svg.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{py:.2}\" fill=\"#9aa7b3\" font-size=\"11\" text-anchor=\"end\" dominant-baseline=\"middle\">{}</text>\n",
            MARGIN_L - 8.0,
            fmt(values2[i] as f64, 3)
        ));
    }

    // Axis labels.
    svg.push_str(&format!(
        "<text x=\"{:.2}\" y=\"{:.2}\" fill=\"#e6edf3\" font-size=\"13\" text-anchor=\"middle\">{}</text>\n",
        MARGIN_L + plot_w / 2.0,
        SVG_H - 22.0,
        esc(cfg.knob.label())
    ));
    let yl_x = 22.0;
    let yl_y = MARGIN_T + plot_h / 2.0;
    svg.push_str(&format!(
        "<text x=\"{yl_x:.2}\" y=\"{yl_y:.2}\" fill=\"#e6edf3\" font-size=\"13\" text-anchor=\"middle\" transform=\"rotate(-90 {yl_x:.2} {yl_y:.2})\">{}</text>\n",
        esc(cfg.knob2.map(|k| k.label()).unwrap_or(""))
    ));

    // Color legend (a vertical strip on the right).
    let lx = MARGIN_L + plot_w + 24.0;
    let ly = MARGIN_T;
    let lw = 16.0;
    let lh = plot_h;
    let segs = 64;
    for k in 0..segs {
        let t = k as f64 / (segs - 1) as f64;
        let (r, g, b) = viridis(t);
        let y = ly + (1.0 - t) * lh - lh / segs as f64;
        svg.push_str(&format!(
            "<rect x=\"{lx:.2}\" y=\"{y:.2}\" width=\"{lw:.2}\" height=\"{:.2}\" fill=\"rgb({r},{g},{b})\"/>\n",
            lh / segs as f64 + 1.0
        ));
    }
    svg.push_str(&format!(
        "<text x=\"{:.2}\" y=\"{:.2}\" fill=\"#9aa7b3\" font-size=\"11\">{}</text>\n",
        lx,
        ly - 6.0,
        fmt(v_hi, 1)
    ));
    svg.push_str(&format!(
        "<text x=\"{:.2}\" y=\"{:.2}\" fill=\"#9aa7b3\" font-size=\"11\">{}</text>\n",
        lx,
        ly + lh + 14.0,
        fmt(v_lo, 1)
    ));

    svg.push_str("</svg>\n");
    svg
}

/// A friendly long-form name for the order parameter, used in titles/axis labels.
fn pretty_metric(m: Metric) -> &'static str {
    match m {
        Metric::Components => "component_count (order parameter)",
        Metric::Loops => "loop_count",
        Metric::Fractal => "fractal_dimension",
        Metric::TrailMass => "trail_mass",
    }
}

/// Choose a sensible number of decimals for a tick step.
fn decimals_for(step: f64) -> usize {
    if step >= 1.0 {
        0
    } else if step >= 0.1 {
        2
    } else {
        3
    }
}

/// Pick up to ~6 evenly-spaced indices in `0..n` for axis ticks.
fn tick_indices(n: usize) -> Vec<usize> {
    if n == 0 {
        return vec![];
    }
    let want = 6.min(n);
    if want <= 1 {
        return vec![0];
    }
    (0..want)
        .map(|i| (i * (n - 1)) / (want - 1))
        .collect::<Vec<_>>()
}

/// A compact viridis-like colormap (perceptually-ordered low→high), returning an
/// 8-bit RGB triple for `t` in `[0, 1]`. Hand-rolled, no dependency: a handful of
/// anchor colors with linear interpolation between them.
fn viridis(t: f64) -> (u8, u8, u8) {
    const ANCHORS: [(f64, f64, f64); 6] = [
        (0.267, 0.005, 0.329), // deep purple
        (0.282, 0.141, 0.458),
        (0.254, 0.265, 0.530),
        (0.207, 0.372, 0.553),
        (0.164, 0.471, 0.558),
        (0.993, 0.906, 0.144), // yellow
    ];
    let t = t.clamp(0.0, 1.0);
    let seg = t * (ANCHORS.len() - 1) as f64;
    let i = (seg.floor() as usize).min(ANCHORS.len() - 2);
    let f = seg - i as f64;
    let (r0, g0, b0) = ANCHORS[i];
    let (r1, g1, b1) = ANCHORS[i + 1];
    let lerp = |a: f64, b: f64| ((a + (b - a) * f) * 255.0).round().clamp(0.0, 255.0) as u8;
    (lerp(r0, r1), lerp(g0, g1), lerp(b0, b1))
}

// ----------------------------------------------------------------------------
// CLI parsing. Hand-rolled `--flag value` parser (no clap). Unknown flags error.
// ----------------------------------------------------------------------------

const USAGE: &str = "usage: sweep [--knob NAME] [--start F] [--end F] [--steps N] \
[--seeds M] [--ticks T] [--grid G] [--metric NAME] [--threads K] \
[--csv PATH] [--svg PATH] [--knob2 NAME --start2 F --end2 F --steps2 N]\n\
  knobs:   decay deposit sensor-angle sensor-distance step diffuse\n\
  metrics: components loops fractal trail-mass";

/// Parse `std::env::args` into a [`Config`], applying knob-specific defaults.
fn parse_args() -> Result<Config, String> {
    let mut args = std::env::args().skip(1).peekable();

    // First resolve the knob (so its default range applies before other flags).
    let mut raw: Vec<(String, String)> = Vec::new();
    let mut knob = Knob::Decay;
    while let Some(flag) = args.next() {
        if flag == "-h" || flag == "--help" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        let val = args
            .next()
            .ok_or_else(|| format!("flag `{flag}` needs a value"))?;
        if flag == "--knob" {
            knob = Knob::parse(&val).ok_or_else(|| format!("unknown knob `{val}`"))?;
        }
        raw.push((flag, val));
    }

    let mut cfg = Config::default_for(knob);

    // Track whether start/end were explicitly given for knob2 so we can default
    // them from the knob's own range when omitted.
    let mut start2_seen = false;
    let mut end2_seen = false;

    for (flag, val) in raw {
        match flag.as_str() {
            "--knob" => { /* already applied */ }
            "--start" => cfg.start = parse_f32(&flag, &val)?,
            "--end" => cfg.end = parse_f32(&flag, &val)?,
            "--steps" => cfg.steps = parse_usize(&flag, &val)?.max(1),
            "--seeds" => cfg.seeds = parse_usize(&flag, &val)?.max(1),
            "--ticks" => cfg.ticks = parse_usize(&flag, &val)?,
            "--grid" => cfg.grid = parse_usize(&flag, &val)?.max(8),
            "--threads" => cfg.threads = parse_usize(&flag, &val)?,
            "--csv" => cfg.csv_path = val,
            "--svg" => cfg.svg_path = val,
            "--metric" => {
                cfg.metric = Metric::parse(&val).ok_or_else(|| format!("unknown metric `{val}`"))?
            }
            "--knob2" => {
                let k2 = Knob::parse(&val).ok_or_else(|| format!("unknown knob `{val}`"))?;
                let (s, e) = k2.default_range();
                cfg.knob2 = Some(k2);
                if !start2_seen {
                    cfg.start2 = s;
                }
                if !end2_seen {
                    cfg.end2 = e;
                }
                if cfg.steps2 == 0 {
                    cfg.steps2 = 12;
                }
            }
            "--start2" => {
                cfg.start2 = parse_f32(&flag, &val)?;
                start2_seen = true;
            }
            "--end2" => {
                cfg.end2 = parse_f32(&flag, &val)?;
                end2_seen = true;
            }
            "--steps2" => cfg.steps2 = parse_usize(&flag, &val)?.max(1),
            other => return Err(format!("unknown flag `{other}`")),
        }
    }

    if cfg.knob2.is_some() && cfg.steps2 == 0 {
        cfg.steps2 = 12;
    }

    Ok(cfg)
}

fn parse_f32(flag: &str, val: &str) -> Result<f32, String> {
    val.parse::<f32>()
        .map_err(|_| format!("flag `{flag}` expects a number, got `{val}`"))
}

fn parse_usize(flag: &str, val: &str) -> Result<usize, String> {
    val.parse::<usize>()
        .map_err(|_| format!("flag `{flag}` expects a non-negative integer, got `{val}`"))
}
