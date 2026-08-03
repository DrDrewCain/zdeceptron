//! What analysing a file costs, and how much of that is code generation.
//!
//! `zdc check` and this crate now run the emitter and drop its output, so
//! the editor sees the diagnostics only codegen can give. That is a real
//! cost paid on every keystroke, and the honest thing is to measure it
//! rather than to argue about it.
//!
//! Run it as
//!
//! ```text
//! cargo test --release -p zdc-lsp --test latency -- --nocapture
//! ```
//!
//! and it prints one row per program size: the whole analysis, the analysis
//! without the emitter, and the difference. A debug build is one to two
//! orders of magnitude slower here — 4.5 seconds for the largest program
//! against 95 milliseconds — so under `cargo test` it prints the small
//! sizes and asserts nothing, and the release invocation above is the one
//! whose numbers are worth reading.
//!
//! **What it found.** Emission is not a fraction of the front end; it is
//! most of the cost, and it grows faster than linearly in the size of the
//! view. On this machine, in release:
//!
//! ```text
//!   rows    bytes          full     front end       codegen     share
//!      1      128       0.041ms       0.013ms       0.028ms     68.8%
//!     10     1136       0.188ms       0.080ms       0.108ms     57.3%
//!     50     5776       1.040ms       0.352ms       0.688ms     66.1%
//!    200    23576      11.859ms       1.338ms      10.522ms     88.7%
//!    500    59576     114.729ms       2.973ms     111.755ms     97.4%
//! ```
//!
//! The front end is linear; the emitter is close to quadratic. That cost is
//! not new — `zdc build` has always paid it — but the language server now
//! pays it on every keystroke, so it is written down here rather than
//! discovered later. At the size of file this language is for it does not
//! matter: the largest checked-in example is under three kilobytes, and a
//! six-kilobyte file is one millisecond. A sixty-kilobyte one is a tenth of
//! a second, which is the point at which typing would feel it, and the fix
//! then is to make the emitter linear rather than to hide it from the
//! editor.
//!
//! The assertion below is therefore an editor-responsiveness budget at a
//! realistic size, not a ratio. A ratio would fail on a loaded machine and
//! teach everyone to ignore it.

use std::time::{Duration, Instant};

/// A program with `rows` signals and a view that reads every one of them.
///
/// Rows are what scale in a real file — a view is mostly elements — and
/// each one costs the emitter a template hole, a binding and a signal
/// declaration, so this grows the part of the work codegen owns.
fn program(rows: usize) -> String {
    let mut out = String::new();
    for row in 0..rows {
        out.push_str(&format!(
            "state field{row} is client Text starting \"value {row}\"\n"
        ));
    }
    out.push_str("view\n    Column\n");
    for row in 0..rows {
        out.push_str(&format!(
            "        Row\n            Text \"field {row}: \"\n            Text field{row}\n"
        ));
    }
    out
}

/// The passes before codegen, written out rather than called.
fn front_end_only(source: &str) -> bool {
    let Ok(program) = zdc_parser::parse(source) else {
        return false;
    };
    let Ok(hir) = zdc_resolve::Resolver::new(&program).resolve() else {
        return false;
    };
    let split = zdc_graph::split(&hir);
    if split.has_errors() {
        return false;
    }
    zdc_graph::ifc(&hir, &split);
    zdc_types::check(&hir, &split).is_ok()
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn time(runs: usize, mut body: impl FnMut()) -> Duration {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        body();
        samples.push(started.elapsed());
    }
    median(samples)
}

#[test]
fn analysing_a_realistic_file_stays_inside_an_editors_budget() {
    println!(
        "{:>6}  {:>7}  {:>12}  {:>12}  {:>12}  {:>8}",
        "rows", "bytes", "full", "front end", "codegen", "share"
    );

    // The two largest sizes take about fifty seconds between them without
    // optimisation, which is not something to put in every `cargo test`.
    let release = !cfg!(debug_assertions);
    let sizes: &[usize] = if release {
        &[1, 10, 50, 200, 500]
    } else {
        &[1, 10, 50]
    };

    let mut budget = Duration::ZERO;
    for &rows in sizes {
        let source = program(rows);
        // One warm-up of each, so the first measurement is not paying for
        // whatever the allocator does the first time it sees this shape.
        let _ = zdc_lsp::Analysis::of(&source);
        let _ = front_end_only(&source);

        let full = time(11, || {
            let analysis = zdc_lsp::Analysis::of(&source);
            assert!(
                analysis.diagnostics().is_empty(),
                "the generated program must be one the compiler accepts"
            );
        });
        let front = time(11, || assert!(front_end_only(&source)));
        let codegen = full.saturating_sub(front);
        let share = codegen.as_secs_f64() / full.as_secs_f64();
        if rows == 50 {
            budget = full;
        }

        println!(
            "{rows:>6}  {:>7}  {:>10.3}ms  {:>10.3}ms  {:>10.3}ms  {:>7.1}%",
            source.len(),
            full.as_secs_f64() * 1_000.0,
            front.as_secs_f64() * 1_000.0,
            codegen.as_secs_f64() * 1_000.0,
            share * 100.0
        );
    }

    // A debug build measures the compiler's own missing optimisations, not
    // what an editor would feel, so there is nothing here to hold it to.
    if !release {
        println!("debug build: timings above are not the ones to read; see the module comment");
        return;
    }

    // A file twice the size of the largest checked-in example, analysed
    // end to end, is what an editor's per-keystroke budget has to cover.
    // The number to defend is that one, not the share codegen takes of it.
    assert!(
        budget < Duration::from_millis(10),
        "analysing a six-kilobyte file takes {:.3}ms, which an editor would show as lag",
        budget.as_secs_f64() * 1_000.0
    );
}
