//! A generative harness that drives the whole front end at inputs nobody
//! would write, and asserts only that it comes back.
//!
//! **The property is total, not partial.** Every pass must terminate with
//! either a value or a diagnostic on every input, valid or not. A panic is
//! a defect even on garbage: `zdc lsp` runs these passes on a half-typed
//! buffer after every keystroke, so an `unwrap` on malformed input is a
//! language server that dies while the user is mid-word. A stack overflow
//! is worse — it raises `SIGABRT`, which `catch_unwind` cannot contain, so
//! it takes this test binary down with it rather than failing one case.
//! That is the intended behaviour of this file: the failure is loud.
//!
//! Four generators, all seeded, so a failure reproduces exactly:
//!
//! * **Mutation** of the real examples — bit flips, deleted runs,
//!   duplicated runs, truncation. This is what a half-saved file looks
//!   like.
//! * **Token salad** from the language's own vocabulary, which reaches
//!   parse states that random bytes never do.
//! * **Spines and nests** — the shapes that used to abort: long infix
//!   chains, projection chains, indentation, prefix operators.
//! * **Hostile text** — very long identifiers, unusual Unicode, combining
//!   marks with nothing to combine with, right-to-left overrides, NUL.
//!
//! Running longer: `ZDC_FUZZ_CASES=200000 cargo test -p zdc-cli --test
//! fuzz`. The default is sized to finish in a couple of seconds so it runs
//! on every commit; the soak is for when the parser or a pass changes
//! shape.

use std::panic::{catch_unwind, AssertUnwindSafe};

const EXAMPLES: [&str; 6] = [
    include_str!("../../../examples/guestbook.zd"),
    include_str!("../../../examples/todo.zd"),
    include_str!("../../../examples/components.zd"),
    include_str!("../../../examples/counter.zd"),
    include_str!("../../../examples/leaderboard.zd"),
    include_str!("../../../examples/voting-board.zd"),
];

/// Every word and sigil the language has, plus a few that look like they
/// belong and do not. Salad drawn from this reaches parse states that
/// random bytes never reach.
const VOCABULARY: [&str; 74] = [
    "state",
    "is",
    "client",
    "server",
    "durable",
    "static",
    "starting",
    "from",
    "secret",
    "function",
    "with",
    "give",
    "view",
    "when",
    "show",
    "each",
    "in",
    "if",
    "otherwise",
    "set",
    "to",
    "add",
    "subtract",
    "append",
    "remove",
    "keep",
    "where",
    "sort",
    "by",
    "map",
    "take",
    "first",
    "on",
    "click",
    "component",
    "children",
    "use",
    "for",
    "record",
    "choice",
    "of",
    "environment",
    "and",
    "or",
    "not",
    "at",
    "yes",
    "no",
    "empty",
    "List",
    "Map",
    "Text",
    "Whole",
    "Decimal",
    "Truth",
    "Column",
    "Row",
    "Text",
    "Heading",
    "Button",
    "Input",
    "Loading",
    "Failed",
    "Ready",
    "+",
    "-",
    "*",
    "/",
    "(",
    ")",
    "[",
    "]",
    ",",
    ".",
];

/// Deterministic, small, and not a dependency. A fuzz corpus that cannot
/// be reproduced from its seed is an anecdote.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next() % bound as u64) as usize
    }

    /// By value rather than by reference, which every alphabet here can
    /// afford: they are all `Copy`. Returning `&T` made the three `&str`
    /// callers depend on an inference improvement newer than the minimum
    /// Rust version this workspace declares — `push_str` wants a `&str`,
    /// that expectation reached the `&'a T` return type first, and `T`
    /// came out as `str` rather than `&str`. Newer rustc recovers; 1.89
    /// reports six errors. `T` is now fixed by the expected type with no
    /// reference to see through.
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

/// Parse, resolve, split, typecheck, and check information flow — in the
/// order `zdc check` runs them, so the corpus exercises the pass pipeline
/// and not just the parser.
///
/// Every pass after the parser is fed only what the pass before it
/// accepted, which is the same discipline the driver uses; feeding a pass
/// a tree it would never see would be testing a contract nothing has.
fn front_end(src: &str) {
    let Ok(program) = zdc_parser::parse(src) else {
        return;
    };
    let Ok(hir) = zdc_resolve::Resolver::new(&program).resolve() else {
        return;
    };
    let split = zdc_graph::split(&hir);
    let _ = zdc_graph::ifc(&hir, &split);
    let _ = zdc_types::check(&hir, &split);
}

/// Run one case and turn a panic into a readable failure naming the seed
/// and the input, so the case is reproducible from the report alone.
fn survives(label: &str, seed: u64, src: &str) {
    let outcome = catch_unwind(AssertUnwindSafe(|| front_end(src)));
    if outcome.is_err() {
        let shown: String = src.chars().take(400).collect();
        panic!(
            "{label} panicked at seed {seed}.\n\
             The front end must reject an input, never panic on one.\n\
             --- input ({} bytes, first 400 chars) ---\n{shown}",
            src.len()
        );
    }
}

fn cases(default: usize) -> usize {
    std::env::var("ZDC_FUZZ_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// A byte-level corruption of a file that used to be valid.
fn mutate(rng: &mut Rng, src: &str) -> String {
    let mut bytes: Vec<u8> = src.as_bytes().to_vec();
    if bytes.is_empty() {
        return String::new();
    }
    for _ in 0..1 + rng.below(8) {
        match rng.below(5) {
            0 => {
                let at = rng.below(bytes.len());
                bytes[at] ^= 1 << rng.below(8);
            }
            1 => {
                let at = rng.below(bytes.len());
                let len = 1 + rng.below(64.min(bytes.len() - at));
                bytes.drain(at..at + len);
            }
            2 => {
                let at = rng.below(bytes.len());
                let len = 1 + rng.below(64.min(bytes.len() - at));
                let run: Vec<u8> = bytes[at..at + len].to_vec();
                bytes.splice(at..at, run);
            }
            3 => bytes.truncate(rng.below(bytes.len())),
            _ => {
                let at = rng.below(bytes.len());
                bytes.insert(at, rng.pick(b"\n\t \0[]().,\"#"));
            }
        }
        if bytes.is_empty() {
            break;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn salad(rng: &mut Rng) -> String {
    let mut out = String::new();
    for _ in 0..1 + rng.below(120) {
        out.push_str(rng.pick(&VOCABULARY));
        match rng.below(6) {
            0 => out.push('\n'),
            1 => {
                out.push('\n');
                for _ in 0..rng.below(5) {
                    out.push_str("    ");
                }
            }
            _ => out.push(' '),
        }
    }
    out.push('\n');
    out
}

/// The shapes that used to abort, at sizes on both sides of the limit.
fn spine(rng: &mut Rng) -> String {
    let depth = [1, 2, 255, 256, 257, 300, 5_000][rng.below(7)];
    match rng.below(6) {
        0 => format!(
            "state x is client Whole starting {}1\n",
            "1 + ".repeat(depth)
        ),
        1 => format!("function f with x\n    give x{}\n", ".f".repeat(depth)),
        2 => format!("function f with x\n    give x{}\n", " at 0".repeat(depth)),
        3 => format!(
            "function f\n    give {}1{}\n",
            "(".repeat(depth),
            ")".repeat(depth)
        ),
        4 => format!("function f\n    give {}yes\n", "not ".repeat(depth)),
        _ => {
            let mut src = String::from("view\n");
            for level in 1..=depth.min(2_000) {
                src.push_str(&" ".repeat(level * 4));
                src.push_str("Column\n");
            }
            src
        }
    }
}

/// Text a lexer has to have an opinion about.
fn hostile(rng: &mut Rng) -> String {
    let pieces = [
        "\u{202e}",  // right-to-left override
        "\u{200b}",  // zero width space
        "\u{0301}",  // a combining mark with nothing to combine with
        "\u{feff}",  // a byte-order mark in the middle of a line
        "\u{1f600}", // outside the basic multilingual plane
        "\u{0}",     // NUL
        "\r",        // a lone carriage return
        "\u{a0}",    // non-breaking space, which is not indentation
        "\u{2028}",  // line separator
        "é",         // precomposed
        "e\u{301}",  // decomposed, equal to the above under NFC and not as bytes
        "\"",        // an unterminated string, half the time
        "#",         // a comment that eats the rest of the line
        "\t",
    ];
    let mut out = String::new();
    for _ in 0..1 + rng.below(60) {
        match rng.below(3) {
            0 => out.push_str(rng.pick(&pieces)),
            1 => out.push_str(rng.pick(&VOCABULARY)),
            _ => {
                let len = [1, 8, 4_096, 200_000][rng.below(4)];
                out.push_str(&"z".repeat(len));
            }
        }
        out.push(rng.pick(&[' ', '\n', ' ']));
    }
    out.push('\n');
    out
}

#[test]
fn corrupting_a_valid_program_never_panics() {
    let total = cases(4_000);
    for seed in 0..total as u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let src = mutate(&mut rng, EXAMPLES[seed as usize % EXAMPLES.len()]);
        survives("a corrupted example", seed, &src);
    }
}

#[test]
fn token_salad_never_panics() {
    let total = cases(4_000);
    for seed in 0..total as u64 {
        let mut rng = Rng(seed.wrapping_mul(0xd1b5_4a32_d192_ed03) | 1);
        let src = salad(&mut rng);
        survives("token salad", seed, &src);
    }
}

#[test]
fn spines_and_nests_never_abort() {
    let total = cases(400);
    for seed in 0..total as u64 {
        let mut rng = Rng(seed.wrapping_mul(0xa076_1d64_78bd_642f) | 1);
        let src = spine(&mut rng);
        survives("a spine", seed, &src);
    }
}

#[test]
fn hostile_text_never_panics() {
    let total = cases(600);
    for seed in 0..total as u64 {
        let mut rng = Rng(seed.wrapping_mul(0xff51_afd7_ed55_8ccd) | 1);
        let src = hostile(&mut rng);
        survives("hostile text", seed, &src);
    }
}

/// The degenerate inputs that have no seed: a pass that assumes a
/// non-empty program is a pass that panics on a new file.
#[test]
fn degenerate_files_never_panic() {
    let inputs = [
        "",
        "\n",
        "   ",
        "\t\t\t",
        "#\n",
        "# only a comment, and no newline at the end",
        "\u{feff}",
        "\0",
        "\r\n\r\n",
        "view\n",
        "view",
        "function f\n",
        "state\n",
        "component C with x\n",
        "use \"./self\" for f\n",
    ];
    for (index, src) in inputs.iter().enumerate() {
        survives("a degenerate file", index as u64, src);
    }
}
