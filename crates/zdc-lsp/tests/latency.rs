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
//! orders of magnitude slower here — seconds where release is milliseconds
//! — so under `cargo test` it prints the small sizes and asserts nothing,
//! and the release invocation above is the one whose numbers are worth
//! reading.
//!
//! **The `codegen` column is a subtraction, and it used to subtract the
//! wrong thing.** `front_end_only` compiled the file on its own;
//! `Analysis::of` compiles it against the prelude. So the whole cost of
//! analysing §17.4.1's library — 150 definitions and 1,200 expressions
//! that are there whatever the file says — fell out of the subtraction and
//! landed in the emitter's column, which read 98% at six kilobytes and was
//! believed. Both sides now compile the same program. Nothing about the
//! `full` column changed; it was never wrong, and the budget below is
//! asserted against it and only against it.
//!
//! **What the corrected instrument found**, on this machine, in release,
//! before either of issue #8's fixes:
//!
//! ```text
//!   rows    bytes          full     front end       codegen     share
//!      1      128      17.663ms      17.227ms       0.437ms      2.5%
//!     10     1136      17.473ms      17.057ms       0.416ms      2.4%
//!     50     5776      18.616ms      17.474ms       1.141ms      6.1%
//!    200    23576      26.091ms      19.144ms       6.947ms     26.6%
//!    500    59576      94.958ms      21.857ms      73.100ms     77.0%
//! ```
//!
//! Two costs, not one, and they are different in kind. The emitter really
//! was superlinear — 1.1ms to 73ms over a tenfold view — and that is issue
//! #8's quadratic. But at the size a person edits it was not what a
//! keystroke was spent on: 17ms of it was **flat**, paid on a 128-byte
//! file as fully as on a 60-kilobyte one, because it is the prelude being
//! re-analysed from scratch every time. Optimising the emitter alone would
//! have moved the six-kilobyte row by one millisecond in twenty.
//!
//! After both — the emitter's walk scheduling routed rather than searched,
//! and the flow pass's witness reconstruction run only where a leak is
//! possible:
//!
//! ```text
//!   rows    bytes          full     front end       codegen     share
//!      1      128       6.744ms       6.081ms       0.663ms      9.8%
//!     10     1136       6.586ms       6.061ms       0.525ms      8.0%
//!     50     5776       7.196ms       6.398ms       0.798ms     11.1%
//!    200    23576       9.355ms       7.796ms       1.559ms     16.7%
//!    500    59576      14.486ms      10.568ms       3.918ms     27.0%
//! ```
//!
//! A sixty-kilobyte file is now 14ms rather than 95, and the growth in it
//! is linear in both columns. What is left of the flat cost is about six
//! milliseconds of prelude — resolve, split, typecheck and what remains of
//! the flow pass — and it is the next thing to spend effort on, because it
//! is now the whole of the budget below at every size a person types.
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
///
/// **Resolved against the prelude, because `Analysis::of` is.** It was not,
/// and that one difference is why the `codegen` column below used to read
/// 99%: `Resolver::new` compiles the file alone, `Resolver::with_prelude`
/// compiles it with §17.4.1's library — 150-odd more definitions and 1,200
/// more expressions — and every front-end pass then walks all of them. The
/// subtraction charged that difference to the emitter, which is the one
/// pass it is not.
fn front_end_only(source: &str) -> bool {
    let Ok(program) = zdc_parser::parse(source) else {
        return false;
    };
    let prelude = zdc_lib::load();
    let Ok(hir) = zdc_resolve::Resolver::with_prelude(prelude.program(), &program).resolve() else {
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
