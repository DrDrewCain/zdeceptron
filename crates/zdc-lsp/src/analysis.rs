//! One pass of the compiler over one file, and everything it produced.
//!
//! The whole pipeline re-runs on every keystroke. That is not incremental
//! and does not claim to be: it is correct by construction because it is
//! the same code path `zdc check` runs, and at the size of file this
//! language is for it costs a fraction of a millisecond. A file large
//! enough for that to matter would need the compiler to gain incremental
//! passes first; nothing here would be reused.
//!
//! Nothing in this module may panic. A language server that dies takes the
//! editor's diagnostics, hover and highlighting with it and says nothing
//! about why, so a file mid-keystroke — which is usually not a valid
//! program and is sometimes not a program at all — has to come back as
//! diagnostics rather than as an abort.

use std::panic::{catch_unwind, AssertUnwindSafe};

use zdc_diagnostics::Diagnostic;
use zdc_hir::Hir;
use zdc_lexer::Token;
use zdc_types::TypeTable;

use crate::lines::LineIndex;
use crate::symbols::{index, SymbolIndex};

/// Everything the compiler could say about one revision of one file.
pub struct Analysis {
    text: String,
    lines: LineIndex,
    diagnostics: Vec<Diagnostic>,
    tokens: Vec<Token>,
    symbols: SymbolIndex,
    hir: Option<Hir>,
    types: Option<TypeTable>,
}

impl Analysis {
    /// Analyse a source text. Never panics, whatever the text is.
    pub fn of(text: &str) -> Analysis {
        let outcome = catch_unwind(AssertUnwindSafe(|| run(text)));
        match outcome {
            Ok(analysis) => analysis,
            // A panic here is a compiler bug rather than a program error,
            // and the programmer cannot act on it — but a silent, dead
            // language server is worse than a diagnostic that admits it.
            Err(_) => Analysis {
                text: text.to_string(),
                lines: LineIndex::new(text),
                diagnostics: vec![Diagnostic::file_error(
                    "The compiler could not analyse this file. This is a defect in the compiler, \
                     not in the file.",
                )],
                tokens: Vec::new(),
                symbols: SymbolIndex::default(),
                hir: None,
                types: None,
            },
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn lines(&self) -> &LineIndex {
        &self.lines
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn symbols(&self) -> &SymbolIndex {
        &self.symbols
    }

    /// The resolved program, if every name in it resolved.
    pub fn hir(&self) -> Option<&Hir> {
        self.hir.as_ref()
    }

    /// Every inferred type, if the program also typechecked.
    pub fn types(&self) -> Option<&TypeTable> {
        self.types.as_ref()
    }
}

/// Parse, resolve, and typecheck, reporting every diagnostic the first
/// failing pass produced.
///
/// The passes are ordered as `zdc check` orders them and for the same
/// reason: a name that points nowhere has no type to check, so running the
/// checker anyway would only repeat the resolver's errors in worse words.
/// Resolution and inference each report everything they find; the parser
/// reports one error, because it stops at the first — that is a property of
/// the parser, not a choice made here.
fn run(text: &str) -> Analysis {
    let lines = LineIndex::new(text);

    let tokens = match zdc_lexer::tokenize(text) {
        Ok(tokens) => tokens,
        Err(error) => {
            return Analysis {
                text: text.to_string(),
                lines,
                diagnostics: vec![Diagnostic {
                    message: error.message,
                    span: Some(error.span),
                    help: None,
                }],
                tokens: Vec::new(),
                symbols: SymbolIndex::default(),
                hir: None,
                types: None,
            }
        }
    };

    let program = match zdc_parser::Parser::new(tokens.clone()).program() {
        Ok(program) => program,
        Err(error) => {
            return Analysis {
                text: text.to_string(),
                lines,
                diagnostics: vec![Diagnostic::from(error)],
                // The token stream survives a parse error, so a file
                // being typed into keeps its colours instead of going
                // plain for as long as it is incomplete.
                tokens,
                symbols: SymbolIndex::default(),
                hir: None,
                types: None,
            };
        }
    };

    let (hir, mut diagnostics) = match zdc_resolve::Resolver::new(&program).resolve() {
        Ok(hir) => (Some(hir), Vec::new()),
        Err(errors) => (None, errors.into_iter().map(Diagnostic::from).collect()),
    };

    let types = match &hir {
        Some(hir) => match zdc_types::check(hir) {
            Ok(types) => Some(types),
            Err(errors) => {
                diagnostics.extend(errors.into_iter().map(Diagnostic::from));
                None
            }
        },
        None => None,
    };

    let symbols = index(&program, hir.as_ref(), &tokens);

    Analysis {
        text: text.to_string(),
        lines,
        diagnostics,
        tokens,
        symbols,
        hir,
        types,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correct_program_produces_no_diagnostics() {
        let analysis =
            Analysis::of("state count is client Whole starting 0\nview\n    Text count\n");
        assert!(
            analysis.diagnostics().is_empty(),
            "unexpected: {:?}",
            analysis.diagnostics()
        );
        assert!(analysis.hir().is_some());
        assert!(analysis.types().is_some());
    }

    /// The resolver reports everything it finds, and so must this.
    #[test]
    fn every_unresolved_name_is_reported_not_only_the_first() {
        let analysis = Analysis::of(
            "state a is client Whole from one\n\
             state b is client Whole from two\n\
             state c is client Whole from three\n",
        );
        assert_eq!(analysis.diagnostics().len(), 3);
    }

    #[test]
    fn every_type_error_is_reported_not_only_the_first() {
        let analysis = Analysis::of(
            "state a is client Whole starting \"text\"\n\
             state b is client Truth starting 1\n",
        );
        assert_eq!(analysis.diagnostics().len(), 2);
    }

    /// A parse error stops the pipeline, so the type checker never runs on
    /// a tree that does not exist.
    #[test]
    fn a_parse_error_does_not_also_produce_resolution_errors() {
        let analysis = Analysis::of("state votes is Map of Id to Int starting empty\n");
        assert_eq!(analysis.diagnostics().len(), 1);
        assert!(analysis.hir().is_none());
    }

    /// A file being typed into is usually not a program. Every one of
    /// these is a real intermediate state of writing `counter.zd`, plus
    /// several files that are not ZDeceptron at all.
    #[test]
    fn nonsense_produces_diagnostics_rather_than_a_panic() {
        let mid_keystroke = [
            "",
            " ",
            "\n\n\n",
            "s",
            "st",
            "state",
            "state ",
            "state c",
            "state count",
            "state count i",
            "state count is",
            "state count is c",
            "state count is client",
            "state count is client Who",
            "state count is client Whole",
            "state count is client Whole start",
            "state count is client Whole starting",
            "state count is client Whole starting 0\nv",
            "state count is client Whole starting 0\nview",
            "state count is client Whole starting 0\nview\n",
            "state count is client Whole starting 0\nview\n    ",
            "state count is client Whole starting 0\nview\n    Te",
            "state count is client Whole starting 0\nview\n    Text ",
            "state count is client Whole starting 0\nview\n    Text co",
            // Not ZDeceptron at all.
            "{\"json\": [1, 2, 3]}",
            "<html><body>hi</body></html>",
            "\u{0}\u{1}\u{2}",
            "\u{1f600}\u{1f600}\u{1f600}",
            "((((((((((",
            "\t\t\tstate\t\tx",
            "state x is client Text starting \"unterminated",
            "# just a comment",
            "state \u{4e2d}\u{6587} is client Text starting \"\u{4e2d}\"",
        ];

        for src in mid_keystroke {
            let analysis = Analysis::of(src);
            // Whatever it says, it must have finished saying it.
            let _ = analysis.symbols().at(0);
            let _ = analysis.diagnostics();
        }
    }

    /// Every span the compiler emits must land inside the text it was
    /// produced from, or the editor is asked to underline nothing.
    #[test]
    fn every_diagnostic_span_lies_within_the_file() {
        let sources = [
            "state votes is Map of Id to Int starting empty\n",
            "state a is client Whole from missing\n",
            "state a is client Whole starting \"text\"\n",
            "view\n    Text (1 + 2\n",
            "view Text",
        ];
        for src in sources {
            let analysis = Analysis::of(src);
            for diagnostic in analysis.diagnostics() {
                let Some(span) = diagnostic.span else {
                    continue;
                };
                assert!(
                    span.end as usize <= src.len(),
                    "span {span:?} runs past {} bytes of {src:?}",
                    src.len()
                );
                assert!(span.start <= span.end, "inverted span {span:?} in {src:?}");
            }
        }
    }

    /// Deep nesting is a stack overflow in a recursive-descent parser, and
    /// a stack overflow cannot be caught. The parser's depth limits are
    /// what stop it; this checks the limits still hold through this crate,
    /// whose own walks recurse over the same tree.
    #[test]
    fn deeply_nested_source_is_reported_rather_than_followed() {
        let deep = format!("state a is client Whole starting {}1", "(".repeat(5_000));
        let analysis = Analysis::of(&deep);
        assert!(!analysis.diagnostics().is_empty());

        let indented: String = (0..5_000)
            .map(|level| format!("{}Column\n", " ".repeat(level * 4)))
            .collect();
        let analysis = Analysis::of(&format!("view\n{indented}"));
        assert!(!analysis.diagnostics().is_empty());
    }

    /// The examples that `zdc check` accepts are analysed with the same
    /// verdict, all the way through to inferred types.
    ///
    /// Not every checked-in example is one of those. Four are aspirational
    /// — `blog.zd` writes `use` (§14D.2), `components.zd` declares
    /// components (§14D.1), `todo.zd` writes `append` and `remove`, and
    /// `leaderboard.zd` has a type error — and all four predate this
    /// crate. What they are useful for here is the harder property, which
    /// the test below asserts on every example without exception.
    #[test]
    fn the_examples_the_compiler_accepts_analyse_cleanly() {
        let accepted = ["counter.zd", "guestbook.zd", "hello.zd", "voting-board.zd"];
        let mut seen = 0;
        for (path, src) in examples() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if !accepted.contains(&name) {
                continue;
            }
            let analysis = Analysis::of(&src);
            assert!(
                analysis.diagnostics().is_empty(),
                "{name}: {:?}",
                analysis.diagnostics()
            );
            assert!(analysis.types().is_some(), "{name} produced no types");
            seen += 1;
        }
        assert_eq!(seen, accepted.len(), "an accepted example went missing");
    }

    /// The server reports exactly what `zdc check` reports, on every
    /// example including the four it rejects. A diagnostic can therefore
    /// never appear in the editor and not on the command line, or the
    /// reverse, whatever state the file is in.
    #[test]
    fn the_diagnostics_are_the_ones_the_compiler_produces() {
        for (path, src) in examples() {
            let expected = compiler_diagnostics(&src);
            let found: Vec<_> = Analysis::of(&src).diagnostics().to_vec();
            assert_eq!(found, expected, "{}", path.display());
        }
    }

    fn examples() -> Vec<(std::path::PathBuf, String)> {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&directory).expect("the examples directory") {
            let path = entry.expect("a directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("zd") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("a readable example");
            found.push((path, src));
        }
        found
    }

    /// The pipeline `zdc check` runs, written out rather than called, so
    /// this is a comparison against the other implementation and not
    /// against itself.
    fn compiler_diagnostics(src: &str) -> Vec<Diagnostic> {
        let program = match zdc_parser::parse(src) {
            Ok(program) => program,
            Err(error) => return vec![Diagnostic::from(error)],
        };
        let hir = match zdc_resolve::Resolver::new(&program).resolve() {
            Ok(hir) => hir,
            Err(errors) => return errors.into_iter().map(Diagnostic::from).collect(),
        };
        match zdc_types::check(&hir) {
            Ok(_) => Vec::new(),
            Err(errors) => errors.into_iter().map(Diagnostic::from).collect(),
        }
    }
}
