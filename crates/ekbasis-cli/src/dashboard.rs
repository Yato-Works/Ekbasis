//! `aion dashboard` — the interactive alternative-timeline page (plan §13, §19).
//!
//! One self-contained HTML file: inline CSS, inline JS, no external assets, no server. It can be
//! opened from a checkout (`file://`), committed as an artifact, or attached to a pull request
//! comment — the browser is the only dependency.
//!
//! The page has two views of the same data as `aion timeline`:
//!
//! * the **futures graph** — base commit, branches, deltas, evaluation letters, trend sparkline,
//! * one **detail card per future** — Performance / Memory / Correctness / Stability, the metric
//!   table, the correctness checks, reliability notes and the delta history chart.

use ekbasis_benchmark::Evaluation;
use ekbasis_core::fmt;

use crate::futures::Future;

/// Repository facts shown in the page header.
pub struct DashboardMeta {
    pub repository: String,
    pub generated_at: String,
    pub aion_version: String,
}

/// Renders the whole dashboard page.
pub fn render(futures: &[Future], meta: &DashboardMeta) -> String {
    let mut out = String::with_capacity(32 * 1024);
    out.push_str(HEAD);
    out.push_str(&format!(
        "<header><h1>AION <span>· alternative timelines</span></h1>\
         <p class=\"meta\">{} · generated {} · aion {}</p></header>\n<main>\n",
        escape(&meta.repository),
        escape(&meta.generated_at),
        escape(&meta.aion_version),
    ));
    render_graph(&mut out, futures);
    for future in futures {
        render_future(&mut out, future);
    }
    if futures.is_empty() {
        out.push_str(
            "<section class=\"empty\"><h2>nothing explored yet</h2>\
             <p>Run <code>aion experiment create &lt;name&gt;</code> and \
             <code>aion experiment run &lt;name&gt;</code> to branch your first future.</p>\
             </section>\n",
        );
    }
    out.push_str("</main>\n");
    out.push_str(TAIL);
    out
}

/// The branching-futures graph: base commit on top, one clickable branch per future.
fn render_graph(out: &mut String, futures: &[Future]) {
    out.push_str("<section class=\"graph\"><h2>timeline</h2>\n");
    if futures.is_empty() {
        out.push_str("</section>\n");
        return;
    }
    // Group futures by the commit they branch from.
    let mut groups: Vec<(String, Vec<&Future>)> = Vec::new();
    for future in futures {
        let base = future
            .summary
            .experiment
            .base_ref
            .clone()
            .unwrap_or_else(|| "(current branch)".to_string());
        let sha = future
            .summary
            .experiment
            .base_commit
            .as_deref()
            .map(fmt::short_sha)
            .unwrap_or_else(|| "???".to_string());
        let key = format!("{base} {sha}");
        match groups.iter_mut().find(|(candidate, _)| *candidate == key) {
            Some((_, members)) => members.push(future),
            None => groups.push((key, vec![future])),
        }
    }
    for (base, members) in &groups {
        out.push_str(&format!(
            "<div class=\"base\"><span class=\"dot\"></span>{}</div>\n<div class=\"branches\">\n",
            escape(base)
        ));
        for future in members {
            let anchor = anchor_of(future.name());
            let delta = future
                .delta_percent()
                .map(fmt::delta_percent)
                .unwrap_or_else(|| "-".to_string());
            let delta_class = match future.delta_percent() {
                Some(value) if value < 0.0 => "delta down",
                Some(value) if value > 0.0 => "delta up",
                _ => "delta flat",
            };
            out.push_str(&format!(
                "<a class=\"branch\" href=\"#{anchor}\">\
                 <span class=\"branch-name\">{}</span>\
                 <span class=\"{delta_class}\">{delta}</span>\
                 <span class=\"letters\">{}</span>\
                 <span class=\"trend\">{}</span>\n\
                 <span class=\"branch-meta\">{}</span></a>\n",
                escape(future.name()),
                letters_html(future.evaluation.as_ref()),
                escape(&trend_text(future)),
                branch_meta(future),
            ));
        }
        out.push_str("</div>\n");
    }
    out.push_str("</section>\n");
}

/// `A B C -` as coloured chips.
fn letters_html(evaluation: Option<&Evaluation>) -> String {
    match evaluation {
        Some(evaluation) => evaluation
            .dimensions()
            .iter()
            .map(|dimension| {
                format!(
                    "<b class=\"{}\">{}</b>",
                    grade_class(dimension.grade),
                    dimension.grade
                )
            })
            .collect::<Vec<_>>()
            .join(""),
        None => "<b class=\"g-unknown\">-</b><b class=\"g-unknown\">-</b>\
                 <b class=\"g-unknown\">-</b><b class=\"g-unknown\">-</b>"
            .to_string(),
    }
}

/// One expandable future: header, four evaluation cards, metrics, history.
fn render_future(out: &mut String, future: &Future) {
    let anchor = anchor_of(future.name());
    out.push_str(&format!("<section class=\"future\" id=\"{anchor}\">\n"));
    out.push_str(&format!(
        "<h2>{} <small>{}</small></h2>\n",
        escape(future.name()),
        future
            .branch()
            .map(|branch| escape(branch))
            .unwrap_or_else(|| "CI candidate".to_string()),
    ));
    if let Some(description) = &future.summary.experiment.description {
        out.push_str(&format!("<p class=\"description\">{}</p>\n", escape(description)));
    }

    // -- four evaluation cards -------------------------------------------------
    out.push_str("<div class=\"dimensions\">\n");
    match &future.evaluation {
        Some(evaluation) => {
            for dimension in evaluation.dimensions() {
                out.push_str(&format!(
                    "<div class=\"dimension {cls}\">\
                     <h3>{title} <b class=\"{cls}\">{grade}</b></h3>\
                     <p class=\"summary\">{summary}</p>\
                     <p class=\"detail\">{detail}</p></div>\n",
                    cls = grade_class(dimension.grade),
                    title = escape(&dimension.title),
                    grade = dimension.grade,
                    summary = escape(&dimension.summary),
                    detail = escape(&dimension.detail),
                ));
            }
            let limiting = evaluation.limiting_dimensions();
            out.push_str(&format!(
                "<div class=\"dimension overall\">\
                 <h3>Overall <b class=\"{cls}\">{grade}</b></h3>\
                 <p class=\"summary\">{note}</p>\
                 <p class=\"detail\">weakest known dimension, A best … F worst</p></div>\n",
                cls = grade_class(evaluation.overall),
                grade = evaluation.overall,
                note = if limiting.is_empty() {
                    escape(evaluation.overall.as_str())
                } else {
                    escape(&format!(
                        "{} ({})",
                        evaluation.overall.as_str(),
                        limiting.join(", ").to_lowercase()
                    ))
                },
            ));
        }
        None => out.push_str(
            "<div class=\"dimension overall unknown\">\
             <h3>not evaluated</h3>\
             <p class=\"summary\">no comparison of baseline vs experiment is stored</p>\
             <p class=\"detail\">run <code>aion experiment run</code> to measure this future</p>\
             </div>\n",
        ),
    }
    out.push_str("</div>\n");

    render_metrics(out, future);
    render_history(out, future);
    out.push_str("</section>\n");
}


/// Metric table + correctness checks + reliability notes of the stored comparison.
fn render_metrics(out: &mut String, future: &Future) {
    let Some(comparison) = &future.comparison else {
        return;
    };
    out.push_str("<h3>measurements</h3>\n<table class=\"metrics\">\n");
    out.push_str(
        "<thead><tr><th>metric</th><th>baseline</th><th>experiment</th><th>difference</th>\
         <th>verdict</th></tr></thead>\n<tbody>\n",
    );
    for metric in &comparison.metrics {
        out.push_str(&format!(
            "<tr class=\"{cls}\"><td>{name}{context}</td><td>{baseline}</td><td>{candidate}</td>\
             <td>{delta}</td><td><span class=\"verdict v-{verdict}\">{verdict}</span></td></tr>\n",
            cls = if metric.secondary { "secondary" } else { "" },
            name = escape(&metric.name),
            context = if metric.secondary {
                " <span class=\"tag\">context</span>"
            } else {
                ""
            },
            baseline = escape(&metric.baseline_text()),
            candidate = escape(&metric.candidate_text()),
            delta = escape(&metric.delta_text()),
            verdict = metric.verdict.as_str(),
        ));
    }
    out.push_str("</tbody>\n</table>\n");

    if let Some(significance) = &comparison.significance {
        out.push_str(&format!(
            "<p class=\"significance\">{} — {}</p>\n",
            escape(&significance.test),
            escape(&significance.describe())
        ));
    }

    if !comparison.checks.is_empty() {
        out.push_str("<h3>correctness</h3>\n<ul class=\"checks\">\n");
        for check in &comparison.checks {
            out.push_str(&format!(
                "<li class=\"c-{state}\"><b>{state}</b> {name} — {detail}</li>\n",
                state = state_class(check.state),
                name = escape(&check.name),
                detail = escape(&check.detail),
            ));
        }
        out.push_str("</ul>\n");
    }

    if !comparison.warnings.is_empty() {
        out.push_str("<h3>reliability notes</h3>\n<ul class=\"warnings\">\n");
        for warning in &comparison.warnings {
            out.push_str(&format!("<li>{}</li>\n", escape(warning)));
        }
        out.push_str("</ul>\n");
    }
}

/// Delta history as an inline SVG bar chart plus the run table beneath it.
fn render_history(out: &mut String, future: &Future) {
    if future.history.is_empty() {
        return;
    }
    out.push_str("<h3>history</h3>\n");
    let deltas: Vec<(f64, &ekbasis_storage::HistoryPoint)> = future
        .history
        .iter()
        .filter_map(|point| point.delta_percent.map(|delta| (delta, point)))
        .collect();
    if deltas.len() >= 2 {
        render_history_chart(out, &deltas, future.summary.delta_percent);
    }

    out.push_str("<table class=\"history\">\n<thead><tr><th>run</th><th>started</th>\
                  <th>mean</th><th>delta</th><th>trend</th><th>state</th></tr></thead>\n<tbody>\n");
    for point in &future.history {
        let trend = point.trend(future.threshold());
        out.push_str(&format!(
            "<tr><td>#{}</td><td>{}</td><td>{}</td><td>{delta}</td>\
             <td>{trend}</td><td>{state}</td></tr>\n",
            point.run_id,
            escape(&point.started_at),
            point
                .mean_ms
                .map(fmt::duration_ms)
                .unwrap_or_else(|| "-".to_string()),
            delta = point
                .delta_percent
                .map(fmt::delta_percent)
                .unwrap_or_else(|| "-".to_string()),
            trend = trend.unwrap_or("-"),
            state = if point.success { "ok" } else { "failed" },
        ));
    }
    out.push_str("</tbody>\n</table>\n");
}


/// SVG bars for the delta history: zero line in the middle, green below (faster), red above.
fn render_history_chart(out: &mut String, deltas: &[(f64, &ekbasis_storage::HistoryPoint)], _current: Option<f64>) {
    const WIDTH: f64 = 640.0;
    const HEIGHT: f64 = 140.0;
    const PAD: f64 = 8.0;
    let max_abs = deltas
        .iter()
        .map(|(delta, _)| delta.abs())
        .fold(1.0f64, f64::max);
    let count = deltas.len() as f64;
    let slot = (WIDTH - 2.0 * PAD) / count;
    let bar_width = (slot * 0.6).max(2.0);
    let zero = HEIGHT / 2.0;
    let scale = (zero - PAD) / max_abs;

    out.push_str(&format!(
        "<svg class=\"chart\" viewBox=\"0 0 {WIDTH} {HEIGHT}\" role=\"img\" \
         aria-label=\"delta history\">\n<line x1=\"0\" y1=\"{zero}\" x2=\"{WIDTH}\" y2=\"{zero}\" \
         class=\"zero\"/>\n"
    ));
    for (index, (delta, point)) in deltas.iter().enumerate() {
        let x = PAD + index as f64 * slot + (slot - bar_width) / 2.0;
        let height = (delta.abs() * scale).max(1.0);
        let (y, class) = if *delta <= 0.0 {
            (zero, "bar better")
        } else {
            (zero - height, "bar worse")
        };
        let title = format!(
            "run #{} · {} · {}",
            point.run_id,
            point.started_at,
            fmt::delta_percent(*delta)
        );
        out.push_str(&format!(
            "<rect class=\"{class}\" x=\"{x:.1}\" y=\"{y:.1}\" width=\"{bar_width:.1}\" \
             height=\"{height:.1}\"><title>{}</title></rect>\n",
            escape(&title)
        ));
    }
    out.push_str("</svg>\n");
    out.push_str(&format!(
        "<p class=\"chart-note\">zero = baseline · below = faster · above = slower \
         (max |delta| {})</p>\n",
        fmt::delta_percent(max_abs)
    ));
}

/// URL-safe element id for a future (used as `href`/`id` pair).
fn anchor_of(name: &str) -> String {
    let mut out = String::from("future-");
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            out.push(character.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    out
}

/// CSS class for a grade chip.
fn grade_class(grade: ekbasis_benchmark::Grade) -> String {
    match grade {
        ekbasis_benchmark::Grade::A => "g-a",
        ekbasis_benchmark::Grade::B => "g-b",
        ekbasis_benchmark::Grade::C => "g-c",
        ekbasis_benchmark::Grade::D => "g-d",
        ekbasis_benchmark::Grade::F => "g-f",
        ekbasis_benchmark::Grade::Unknown => "g-unknown",
    }
    .to_string()
}

/// CSS class for a check state.
fn state_class(state: ekbasis_benchmark::CheckState) -> &'static str {
    match state {
        ekbasis_benchmark::CheckState::Pass => "pass",
        ekbasis_benchmark::CheckState::Fail => "fail",
        ekbasis_benchmark::CheckState::Warn => "warn",
        ekbasis_benchmark::CheckState::Info => "info",
    }
}

/// Short branch line under a future in the graph.
fn branch_meta(future: &Future) -> String {
    let mut parts = Vec::new();
    if let Some(run) = &future.summary.latest_experiment {
        parts.push(format!("run #{}", run.id));
        parts.push(fmt::short_sha(&run.commit_sha));
    } else {
        parts.push("never run".to_string());
    }
    if future.history.len() > 1 {
        parts.push(format!("{} runs", future.history.len()));
    }
    parts.join(" · ")
}

/// Trend sparkline text for the graph (empty when there is no history yet).
fn trend_text(future: &Future) -> String {
    let deltas = future.deltas();
    if deltas.len() < 2 {
        return String::new();
    }
    ekbasis_benchmark::statistics::sparkline(&deltas)
}

/// Escapes text for HTML text and attribute contexts.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}


/// Page head: document skeleton plus the whole stylesheet. Everything is inline — the file must
/// work from `file://` with no network at all.
const HEAD: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>AION · alternative timelines</title>
<style>
:root {
  --bg: #0e1116; --panel: #161b22; --border: #262d38; --text: #d7dde6;
  --muted: #8b94a3; --accent: #58a6ff;
  --a: #3fb950; --b: #56d364; --c: #d29922; --d: #f0883e; --f: #f85149; --unknown: #6e7681;
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--text);
  font: 15px/1.5 ui-sans-serif, system-ui, "Segoe UI", sans-serif; }
header { padding: 28px 32px 18px; border-bottom: 1px solid var(--border); }
h1 { margin: 0; font-size: 22px; letter-spacing: .04em; }
h1 span { color: var(--muted); font-weight: 400; }
.meta { margin: 6px 0 0; color: var(--muted); font-size: 13px; }
main { padding: 24px 32px 64px; max-width: 1080px; margin: 0 auto; }
h2 { font-size: 15px; text-transform: uppercase; letter-spacing: .12em;
  color: var(--muted); margin: 32px 0 14px; }
h3 { font-size: 14px; margin: 0 0 6px; }
code { background: var(--panel); border: 1px solid var(--border); border-radius: 4px;
  padding: 1px 5px; font-size: 13px; }
a { color: var(--accent); text-decoration: none; }

/* futures graph */
.base { display: inline-flex; align-items: center; gap: 8px; background: var(--panel);
  border: 1px solid var(--border); border-radius: 999px; padding: 5px 16px;
  font-family: ui-monospace, Consolas, monospace; font-size: 13px; }
.base .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--accent); }
.branches { border-left: 2px solid var(--border); margin: 10px 0 0 24px;
  padding: 4px 0 4px 22px; display: flex; flex-direction: column; gap: 10px; position: relative; }
.branch { display: grid; grid-template-columns: minmax(180px, 1fr) 90px auto minmax(60px, auto);
  gap: 12px; align-items: center; background: var(--panel); border: 1px solid var(--border);
  border-radius: 10px; padding: 10px 14px; color: var(--text); position: relative; }
.branch::before { content: ""; position: absolute; left: -23px; top: 50%; width: 21px;
  height: 2px; background: var(--border); }
.branch:hover { border-color: var(--accent); }
.branch-name { font-weight: 600; overflow: hidden; text-overflow: ellipsis;
  white-space: nowrap; }
.delta { font-family: ui-monospace, Consolas, monospace; text-align: right; font-size: 14px; }
.delta.down { color: var(--a); } .delta.up { color: var(--f); } .delta.flat { color: var(--muted); }
.letters { display: inline-flex; gap: 4px; }
.letters b { width: 22px; height: 22px; border-radius: 5px; display: inline-flex;
  align-items: center; justify-content: center; font-size: 12px; color: #0e1116; }
.trend { font-family: ui-monospace, Consolas, monospace; color: var(--muted);
  justify-self: end; letter-spacing: -1px; }
.branch-meta { grid-column: 1 / -1; color: var(--muted); font-size: 12px; }
.g-a, .g-b { background: var(--b); } .g-c { background: var(--c); }
.g-d { background: var(--d); } .g-f { background: var(--f); }
.g-unknown { background: var(--unknown); color: var(--text) !important; }

/* future detail cards */
.future { margin-top: 36px; border-top: 1px solid var(--border); padding-top: 20px; }
.future h2 { color: var(--text); text-transform: none; letter-spacing: 0; font-size: 19px;
  margin: 0 0 4px; }
.future h2 small { color: var(--muted); font-weight: 400; font-size: 13px;
  font-family: ui-monospace, Consolas, monospace; margin-left: 8px; }
.description { color: var(--muted); margin: 0 0 16px; }
.dimensions { display: grid; grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
  gap: 12px; }
.dimension { background: var(--panel); border: 1px solid var(--border); border-radius: 10px;
  padding: 12px 14px; border-left: 4px solid var(--unknown); }
.dimension.g-a, .dimension.g-b { border-left-color: var(--b); }
.dimension.g-c { border-left-color: var(--c); }
.dimension.g-d { border-left-color: var(--d); }
.dimension.g-f { border-left-color: var(--f); }
.dimension.overall { background: #12171e; }
.dimension h3 { display: flex; justify-content: space-between; align-items: center; }
.dimension h3 b { width: 26px; height: 26px; border-radius: 6px; display: inline-flex;
  align-items: center; justify-content: center; color: #0e1116; font-size: 13px; }
.summary { margin: 0 0 4px; font-size: 13px; }
.detail { margin: 0; color: var(--muted); font-size: 12px; }

/* tables, checks, notes */
h3 { color: var(--muted); text-transform: uppercase; letter-spacing: .1em;
  font-size: 12px; margin-top: 26px; }
table { width: 100%; border-collapse: collapse; font-size: 13px; margin-top: 8px; }
th, td { text-align: right; padding: 6px 10px; border-bottom: 1px solid var(--border); }
th:first-child, td:first-child { text-align: left; }
th { color: var(--muted); font-weight: 500; }
tr.secondary td { color: var(--muted); }
.tag { background: var(--border); border-radius: 4px; padding: 0 5px; font-size: 10px;
  color: var(--muted); }
.verdict { font-family: ui-monospace, Consolas, monospace; font-size: 12px; }
.v-improved { color: var(--a); } .v-regressed { color: var(--f); }
.v-neutral { color: var(--c); } .v-unavailable { color: var(--unknown); }
.significance { color: var(--muted); font-size: 13px; }
ul.checks, ul.warnings { list-style: none; padding: 0; margin: 8px 0 0; font-size: 13px; }
ul.checks li, ul.warnings li { padding: 5px 10px; border: 1px solid var(--border);
  border-radius: 6px; margin-bottom: 6px; background: var(--panel); }
ul.checks li b { font-family: ui-monospace, Consolas, monospace; font-size: 11px;
  margin-right: 6px; }
.c-pass b { color: var(--a); } .c-fail b { color: var(--f); }
.c-warn b { color: var(--c); } .c-info b { color: var(--muted); }
ul.warnings li { border-left: 3px solid var(--c); }

/* charts */
svg.chart { width: 100%; height: 140px; background: var(--panel); border: 1px solid var(--border);
  border-radius: 8px; margin-top: 8px; }
svg.chart .zero { stroke: var(--muted); stroke-dasharray: 4 4; stroke-width: 1; }
svg.chart .bar.better { fill: var(--a); } svg.chart .bar.worse { fill: var(--f); }
.chart-note { color: var(--muted); font-size: 12px; margin: 4px 0 0; }
.empty { text-align: center; color: var(--muted); margin-top: 60px; }
footer { color: var(--unknown); font-size: 12px; text-align: center; padding: 24px; }
@media (max-width: 720px) { main { padding: 16px; } .branch { grid-template-columns: 1fr auto; }
  .trend, .branch-meta { display: none; } }
</style>
</head>
<body>
"#;


/// Page tail: a small script for smooth scrolling and the footer. No external references at all.
const TAIL: &str = r#"<footer>
Git tells you how your software changed. AION lets you test what it could become.
</footer>
<script>
document.addEventListener("click", function (event) {
  var branch = event.target.closest("a.branch");
  if (!branch) return;
  var target = document.querySelector(branch.getAttribute("href"));
  if (!target) return;
  event.preventDefault();
  target.scrollIntoView({ behavior: "smooth", block: "start" });
  history.replaceState(null, "", branch.getAttribute("href"));
});
</script>
</body>
</html>
"#;


#[cfg(test)]
mod tests {
    use super::*;
    use ekbasis_storage::{ExperimentRow, ExperimentSummary, RunRow};
    use ekbasis_core::model::RunKind;

    fn run_row(kind: RunKind, id: i64) -> RunRow {
        RunRow {
            id,
            experiment_id: 1,
            experiment_name: "demo".to_string(),
            kind,
            label: format!("demo:{kind}"),
            branch: Some("aion/demo".to_string()),
            commit_sha: "abcdef1234567890".to_string(),
            base_commit: "1234567890abcdef".to_string(),
            started_at: "2026-10-01T00:00:00Z".to_string(),
            finished_at: "2026-10-01T00:01:00Z".to_string(),
            success: true,
            iterations: 3,
            warmup: 1,
            build_ms: Some(900.0),
            setup_ms: None,
            test_success: Some(true),
            mean_ms: Some(120.0),
            median_ms: Some(119.0),
            min_ms: Some(110.0),
            max_ms: Some(130.0),
            stddev_ms: Some(5.0),
            cv_percent: Some(4.1),
            peak_memory_bytes: Some(1024 * 1024),
            avg_cpu_percent: Some(42.0),
            report_path: Some(".aion/results/demo.experiment.json".to_string()),
            machine: Some("TestOS".to_string()),
        }
    }

    fn future_named(name: &str) -> Future {
        let experiment = ExperimentRow {
            id: 1,
            name: name.to_string(),
            spec_path: format!(".aion/experiments/{name}.yaml"),
            base_ref: Some("main".to_string()),
            base_commit: Some("1234567890abcdef".to_string()),
            description: Some("a <b>future</b> & \"more\"".to_string()),
            created_at: "2026-10-01T00:00:00Z".to_string(),
        };
        let experiment_run = run_row(RunKind::Experiment, 2);
        let baseline = run_row(RunKind::Baseline, 1);
        let summary = ExperimentSummary {
            experiment,
            runs: vec![experiment_run.clone(), baseline.clone()],
            latest_baseline: Some(baseline),
            latest_experiment: Some(experiment_run),
            delta_percent: Some(-12.5),
        };
        Future {
            summary,
            comparison: None,
            evaluation: None,
            history: Vec::new(),
            spec_threshold: Some(3.0),
        }
    }

    fn meta() -> DashboardMeta {
        DashboardMeta {
            repository: "C:/repo".to_string(),
            generated_at: "2026-10-01T00:00:00Z".to_string(),
            aion_version: "1.0.0".to_string(),
        }
    }

    #[test]
    fn empty_repository_offers_the_next_command() {
        let html = render(&[], &meta());
        assert!(html.contains("nothing explored yet"), "{html}");
        assert!(html.contains("aion experiment create"));
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn futures_appear_in_the_graph_with_escaped_text() {
        let future = future_named("speed-up <test>");
        let html = render(std::slice::from_ref(&future), &meta());

        // The graph links to the detail section and shows the measured delta.
        assert!(html.contains("class=\"branch\""), "{html}");
        assert!(html.contains("-12.5%"));
        assert!(html.contains("id=\"future-speed-up--test-\""));
        // User supplied text must be escaped, not executed.
        assert!(html.contains("a &lt;b&gt;future&lt;/b&gt; &amp; &quot;more&quot;"));
        assert!(!html.contains("<b>future</b>"), "raw HTML leaked into the page");
        // No evaluation means the placeholder letters, never a fabricated grade.
        assert!(html.contains("<b class=\"g-unknown\">-</b>"));
        assert!(html.contains("not evaluated"));
    }

    #[test]
    fn html_special_characters_are_escaped_everywhere() {
        assert_eq!(escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
        assert_eq!(anchor_of("Speed.Up: 2!"), "future-speed-up--2-");
        assert_eq!(anchor_of("ok_name-1"), "future-ok_name-1");
    }

    #[test]
    fn page_has_no_external_dependencies() {
        let future = future_named("demo");
        let html = render(std::slice::from_ref(&future), &meta());
        assert!(!html.contains("http://"), "external URL in {html}");
        assert!(!html.contains("https://"), "external URL in {html}");
        assert!(!html.contains("<link "), "external stylesheet in {html}");
        assert!(!html.contains("<script src="), "external script in {html}");
    }

    #[test]
    fn history_table_renders_deltas_and_trends() {
        let mut future = future_named("demo");
        future.history = vec![
            ekbasis_storage::HistoryPoint {
                run_id: 2,
                started_at: "2026-10-01".to_string(),
                mean_ms: Some(120.0),
                delta_percent: Some(-12.5),
                success: true,
                cv_percent: Some(4.1),
            },
            ekbasis_storage::HistoryPoint {
                run_id: 4,
                started_at: "2026-10-02".to_string(),
                mean_ms: Some(140.0),
                delta_percent: Some(16.6),
                success: true,
                cv_percent: Some(3.0),
            },
        ];
        let html = render(std::slice::from_ref(&future), &meta());
        assert!(html.contains("<h3>history</h3>"), "{html}");
        assert!(html.contains("-12.5%"));
        assert!(html.contains("improved"), "the trend must classify the delta");
        assert!(html.contains("regressed"));
        assert!(html.contains("<svg class=\"chart\""), "the history needs its chart");
    }
}

