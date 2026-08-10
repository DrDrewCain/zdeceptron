//! The committed results table.
//!
//! Generated from the measurements rather than typed, and compared against
//! `BENCHMARKS.md` by a test, so a number in the repository that disagrees
//! with the code is a build failure (§14A.4). Regenerate with
//! `ZDC_BLESS=1 cargo test -p zdc-bench`.

use crate::sizes::{bundle_sizes, runtime_sizes};
use crate::Report;

pub const START_MARKER: &str = "<!-- generated: benchmark results -->";
pub const END_MARKER: &str = "<!-- end generated -->";

const ARM_LABELS: &[(&str, &str)] = &[
    ("zd-positional", "ZDeceptron (positional keys, today)"),
    ("zd-identity", "ZDeceptron (identity keys, with `unique`)"),
    ("direct", "Direct emission (rejected design)"),
    ("vanilla", "Vanilla JS (node by node)"),
    ("vanilla-tuned", "Vanilla JS (hand-tuned)"),
];

fn label(arm: &str) -> &str {
    ARM_LABELS
        .iter()
        .find(|(name, _)| *name == arm)
        .map(|(_, label)| *label)
        .unwrap_or(arm)
}

fn row(cells: &[String]) -> String {
    format!("| {} |\n", cells.join(" | "))
}

fn divider(columns: usize) -> String {
    format!("|{}\n", "---|".repeat(columns))
}

/// A table of one counter across every arm and every step.
fn matrix(report: &Report, title: &str, note: &str, key: &str) -> String {
    let arms = report.arms();
    let mut out = format!("### {title}\n\n{note}\n\n");
    let mut header = vec!["Operation".to_string()];
    header.extend(arms.iter().map(|arm| label(arm).to_string()));
    out.push_str(&row(&header));
    out.push_str(&divider(header.len()));
    for step in report.steps() {
        let mut cells = vec![step.to_string()];
        for arm in &arms {
            cells.push(report.find(arm, step).get(key).to_string());
        }
        out.push_str(&row(&cells));
    }
    out.push('\n');
    out
}

/// The counter-by-counter breakdown of one step.
fn breakdown(report: &Report, step: &str) -> String {
    let arms = report.arms();
    let counters = [
        ("cloneNode", "cross.cloneNode"),
        ("createElement", "cross.createElement"),
        ("createTextNode", "cross.createTextNode"),
        ("createComment", "cross.createComment"),
        ("insertBefore", "cross.insertBefore"),
        ("removeChild", "cross.removeChild"),
        ("replaceChildren", "cross.replaceChildren"),
        ("setAttribute", "cross.setAttribute"),
        ("addEventListener", "cross.addEventListener"),
        ("text writes", "cross.textWrite"),
        ("**crossings, total**", "crossings"),
        ("nodes allocated", "work.createElement"),
        ("effects created", "reactive.effect"),
        ("effect runs", "reactive.effectRun"),
        ("signals created", "reactive.signal"),
    ];

    let mut out = format!("### `{step}` — every counter\n\n");
    let mut header = vec!["Counter".to_string()];
    header.extend(arms.iter().map(|arm| label(arm).to_string()));
    out.push_str(&row(&header));
    out.push_str(&divider(header.len()));
    for (title, key) in counters {
        let mut cells = vec![title.to_string()];
        for arm in &arms {
            let measurement = report.find(arm, step);
            let value = if key == "work.createElement" {
                measurement.get("work.createElement")
                    + measurement.get("work.createTextNode")
                    + measurement.get("work.createComment")
            } else {
                measurement.get(key)
            };
            cells.push(value.to_string());
        }
        out.push_str(&row(&cells));
    }
    out.push('\n');
    out
}

/// What one row costs, which is the number that scales.
fn per_row(report: &Report, step: &str) -> String {
    let arms = report.arms();
    let mut out = format!("### What one row costs, at `{step}`\n\n");
    let mut header = vec!["Per row".to_string()];
    header.extend(arms.iter().map(|arm| label(arm).to_string()));
    out.push_str(&row(&header));
    out.push_str(&divider(header.len()));

    let rows = [
        ("DOM crossings", "crossings"),
        ("nodes allocated", "nodes"),
        ("effects created", "reactive.effect"),
        ("event listeners", "cross.addEventListener"),
        ("attribute writes", "cross.setAttribute"),
        ("text writes", "cross.textWrite"),
    ];
    for (title, key) in rows {
        let mut cells = vec![title.to_string()];
        for arm in &arms {
            let measurement = report.find(arm, step);
            let count = measurement.get("rows").max(1);
            let value = if key == "nodes" {
                measurement.get("work.createElement")
                    + measurement.get("work.createTextNode")
                    + measurement.get("work.createComment")
            } else {
                measurement.get(key)
            };
            // One decimal place, formatted by hand so the table stays
            // integer-exact where the number is an integer.
            let tenths = (value * 10 + count / 2) / count;
            cells.push(if tenths % 10 == 0 {
                (tenths / 10).to_string()
            } else {
                format!("{}.{}", tenths / 10, tenths % 10)
            });
        }
        out.push_str(&row(&cells));
    }
    out.push('\n');
    out
}

fn sizes() -> String {
    let mut out = String::from("### Bundle size, in bytes\n\n");
    out.push_str(&row(&[
        "Program".to_string(),
        "client.js".to_string(),
        "boot.js".to_string(),
        "styles.css".to_string(),
        "index.html".to_string(),
        "manifest.json".to_string(),
        "total".to_string(),
    ]));
    out.push_str(&divider(7));
    for size in bundle_sizes() {
        out.push_str(&row(&[
            format!("`{}`", size.name),
            size.client_js.to_string(),
            size.boot_js.to_string(),
            size.styles_css.to_string(),
            size.index_html.to_string(),
            size.manifest_json.to_string(),
            size.total().to_string(),
        ]));
    }
    out.push('\n');
    out.push_str(&row(&["Runtime file".to_string(), "bytes".to_string()]));
    out.push_str(&divider(2));
    for (name, bytes) in runtime_sizes() {
        out.push_str(&row(&[format!("`{name}`"), bytes.to_string()]));
    }
    out.push('\n');
    out
}

/// Moves per reorder, both reconcilers, at every shape and size.
///
/// One counter and two arms, laid out the other way round from the tables
/// above: the row names the shape and the size, because what is in
/// question here is how the count grows with the list rather than how two
/// emissions compare on one list.
fn reorders(reorder: &Report) -> String {
    let mut out = String::from("### Moves per reorder\n\n");
    out.push_str(
        "`insertBefore` calls one reorder makes. Every row in this measurement has exactly one \
         root, so a move is one call and the count is the size of the move set rather than a \
         proxy for it. **`cursor walk`** is the placement pass `eachInto` used before the \
         longest-increasing-subsequence reconciler landed; it is kept as an arm so that the \
         change is measured rather than remembered, and the two arms are checked for having \
         produced the same order.\n\n",
    );
    out.push_str(&row(&[
        "Reorder".to_string(),
        "moves, LIS reconciler".to_string(),
        "moves, cursor walk (before)".to_string(),
        "rows retired".to_string(),
    ]));
    out.push_str(&divider(4));
    for step in reorder.steps() {
        out.push_str(&row(&[
            step.to_string(),
            reorder.find("lis", step).get("moves").to_string(),
            reorder.find("cursor", step).get("moves").to_string(),
            reorder.find("lis", step).get("removals").to_string(),
        ]));
    }
    out.push('\n');
    out
}

/// The whole generated region of `BENCHMARKS.md`.
pub fn generated_section(report: &Report, reorder: &Report) -> String {
    let mut out = String::new();
    out.push_str(&matrix(
        report,
        "DOM crossings per operation",
        "Calls from JavaScript into the DOM. Work performed *inside* one call \
         — the subtree `cloneNode(true)` allocates, the children inserting a \
         fragment links, the removals `replaceChildren()` performs — is not a \
         further crossing; it is counted as work below.",
        "crossings",
    ));
    out.push_str(&matrix(
        report,
        "Effect runs per operation",
        "A binding re-running. Zero for the vanilla arms, which have no \
         bindings. This is the number that says whether a list operation \
         touched only what changed.",
        "reactive.effectRun",
    ));
    out.push_str(&matrix(
        report,
        "Text-node writes per operation",
        "`nodeValue` writes that actually reached a text node. `bindText` \
         compares before writing (§16.2 R7), so a re-run that computes the \
         same string costs an effect run and no write.",
        "cross.textWrite",
    ));
    out.push_str(&reorders(reorder));
    out.push_str(&per_row(report, "create 10,000 rows"));
    out.push_str(&breakdown(report, "create 10,000 rows"));
    out.push_str(&breakdown(report, "update every 10th row"));
    out.push_str(&sizes());
    out
}
