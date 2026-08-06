//! One pass of the compiler over one file, and everything it produced.
//!
//! The whole pipeline re-runs on every keystroke — parse, resolve,
//! typecheck, and code generation with the bundle thrown away. That is not
//! incremental and does not claim to be: it is correct by construction
//! because it is the same code path `zdc check` runs, which is the whole
//! basis of §14's claim that the editor and the command line cannot
//! disagree. Running the emitter is what makes that claim true rather than
//! nearly true: §16.3.5's injection refusals, §16.3.6's shape checks and
//! §16.5's placement refusals are codegen's, and while they were skipped
//! here the editor showed a clean file for programs `zdc build` rejects.
//!
//! It costs, measured in `tests/latency.rs`: a six-kilobyte file — twice
//! the largest checked-in example — is under a millisecond end to end, of
//! which the emitter is about two thirds. The emitter is close to
//! quadratic in the size of the view, so a sixty-kilobyte file is a tenth
//! of a second; that is the size at which typing would feel it, and the
//! answer then is a linear emitter rather than an editor that has stopped
//! running one of the passes. A file large enough for that to matter would
//! need the compiler to gain incremental passes first; nothing here would
//! be reused.
//!
//! Nothing in this module may panic. A language server that dies takes the
//! editor's diagnostics, hover and highlighting with it and says nothing
//! about why, so a file mid-keystroke — which is usually not a valid
//! program and is sometimes not a program at all — has to come back as
//! diagnostics rather than as an abort.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use zdc_diagnostics::Diagnostic;
use zdc_hir::Hir;
use zdc_lexer::{Span, Token};
use zdc_resolve::Linked;
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
    linked: Option<Linked>,
}

/// A span resolved back to the file that owns it.
///
/// Every span downstream of the linker indexes the combined buffer rather
/// than any one file (`zdc_resolve::modules`), so a span is not a location
/// until it has been through here.
#[derive(Debug, Clone, Copy)]
pub struct Located<'a> {
    /// The file the span came from, or `None` for a document the editor
    /// holds a buffer for but no path to.
    pub path: Option<&'a Path>,
    /// That file's text, which is what `span` indexes.
    pub text: &'a str,
    pub span: Span,
}

impl Analysis {
    /// Analyse a source text. Never panics, whatever the text is.
    pub fn of(text: &str) -> Analysis {
        Analysis::of_document(None, text)
    }

    /// The same, for a document that is somewhere on disk.
    ///
    /// The path is what lets a `use` line be followed (§14D.2): the buffer
    /// stands in for the entry file, and the files it imports are read from
    /// disk. Without it a file with imports would report every imported
    /// name as undefined, which is a worse answer than reading the last
    /// saved version of its neighbours.
    pub fn of_document(path: Option<&Path>, text: &str) -> Analysis {
        let outcome = catch_unwind(AssertUnwindSafe(|| run(path, text)));
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
                linked: None,
            },
        }
    }

    /// The state [`Analysis::of`] lands in when the compiler panics: one
    /// diagnostic about the file as a whole, carrying no span.
    ///
    /// Reachable in production only through a compiler defect, which is
    /// why the editor path out of it has to be reachable from a test —
    /// otherwise the one code path that runs on the worst day is the one
    /// nothing ever exercises.
    #[cfg(test)]
    pub(crate) fn spanless(text: &str, message: &str) -> Analysis {
        Analysis {
            text: text.to_string(),
            lines: LineIndex::new(text),
            diagnostics: vec![Diagnostic::file_error(message)],
            tokens: Vec::new(),
            symbols: SymbolIndex::default(),
            hir: None,
            types: None,
            linked: None,
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

    /// The file a span belongs to, and the span within that file.
    ///
    /// A span is a byte range in the linker's combined buffer, which is
    /// every module's text concatenated. Rendering one against the
    /// document the editor asked about is right only while the program
    /// imports nothing: the moment it writes `use`, a span from an
    /// imported module names an offset in a file that is not on screen.
    /// Every answer this server gives that carries a location goes
    /// through here, so none of them can make that mistake separately.
    ///
    /// The entry file's own text leads the combined buffer, so a span
    /// inside it locates to the buffer the editor sent rather than to the
    /// last saved copy on disk.
    pub fn locate(&self, span: Span) -> Located<'_> {
        match &self.linked {
            Some(linked) => {
                let (path, text, span) = linked.locate(span);
                Located {
                    path: Some(path),
                    text,
                    span,
                }
            }
            // Nothing was linked, so the span already indexes this text.
            // The path is not recorded in that case, and answering with
            // the document the request named is the caller's job.
            None => Located {
                path: None,
                text: &self.text,
                span,
            },
        }
    }

    /// Whether a span points into the document itself rather than into a
    /// file it imports.
    ///
    /// The entry file comes first in the combined buffer, so its own text
    /// is the leading prefix of it. This is the same test the diagnostic
    /// filter in this module makes, written once.
    pub fn in_document(&self, span: Span) -> bool {
        span.start < (self.text.len() as u32).max(1)
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
fn run(path: Option<&Path>, text: &str) -> Analysis {
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
                    notes: Vec::new(),
                    help: None,
                    code: None,
                }],
                tokens: Vec::new(),
                symbols: SymbolIndex::default(),
                hir: None,
                types: None,
                linked: None,
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
                linked: None,
            };
        }
    };

    // A file with imports is not a whole program on its own, so it is
    // linked with what it reaches before any name is resolved (§14D.2).
    // Everything past the entry file's own text belongs to another
    // document, and this one's line index cannot address it, so those
    // diagnostics are dropped rather than pointed somewhere wrong.
    let linked = match path.filter(|_| imports(&program)) {
        Some(path) => zdc_resolve::load_with_entry(path, text.to_string()).ok(),
        None => None,
    };
    let here = text.len() as u32;

    // The editor compiles against the prelude too, so a completion or a
    // hover for a library operation is the same answer `zdc check` gives.
    let prelude = zdc_lib::load();
    let resolved = match &linked {
        Some(linked) => {
            zdc_resolve::Resolver::linked_with_prelude(prelude.program(), linked).resolve()
        }
        None => zdc_resolve::Resolver::with_prelude(prelude.program(), &program).resolve(),
    };
    let (hir, mut diagnostics) = match resolved {
        Ok(hir) => (Some(hir), Vec::new()),
        Err(errors) => (
            None,
            errors
                .into_iter()
                .map(Diagnostic::from)
                .filter(|diagnostic| in_this_file(diagnostic, here))
                .collect(),
        ),
    };

    // §17.1.2's order: the split runs before the checker, because the type
    // of a cross-placement read depends on the crossing. The flow pass runs
    // alongside the checker and both report, so an editor showing a type
    // error also shows the leak it would otherwise hide.
    // The split and the verdict are kept rather than dropped: code
    // generation reads all four of §17.1.3's products, and it runs below.
    let mut solved = None;
    if let Some(hir) = &hir {
        let split = zdc_graph::split(hir);
        diagnostics.extend(
            split
                .errors()
                .cloned()
                .map(Diagnostic::from)
                .filter(|diagnostic| in_this_file(diagnostic, here)),
        );
        if !split.has_errors() {
            let verdict = zdc_graph::ifc(hir, &split);
            diagnostics.extend(
                verdict
                    .diagnostics
                    .iter()
                    .filter(|d| d.is_error())
                    .cloned()
                    .map(Diagnostic::from)
                    .filter(|diagnostic| in_this_file(diagnostic, here)),
            );
            match zdc_types::check(hir, &split) {
                Ok(types) => solved = Some((split, verdict, types)),
                Err(errors) => diagnostics.extend(
                    errors
                        .into_iter()
                        .map(Diagnostic::from)
                        .filter(|diagnostic| in_this_file(diagnostic, here)),
                ),
            }
        }
    }

    // Code generation, with the bundle dropped. Codegen owns diagnostics no
    // earlier pass can give — §16.3.5's injection refusals, §16.3.6's
    // `only_children`/`only_inside` shape checks, §16.5's placement
    // refusals — and running the passes before it and stopping was enough
    // to make the editor show a clean file that `zdc build` rejects. That
    // is the disagreement §14's language-server section says cannot happen,
    // so it happens here as well or the claim is false.
    if let (Some(hir), Some((split, verdict, types))) = (&hir, &solved) {
        // Only a cleared program is emitted, exactly as `zdc build` does
        // it. An uncleared one is not a file the editor calls clean: the
        // flow pass's own refusals are already in `diagnostics` above, and
        // running codegen over a program the pass refused would report a
        // second opinion about a program that has no emission.
        //
        // Matched rather than unwrapped: `Inputs` cannot be built without
        // the flow pass's own permission to emit, and in a long-running
        // server a missing clearance is a file to leave alone rather than
        // a reason to take the process down.
        if let Some(cleared) = verdict.clearance() {
            diagnostics.extend(
                zdc_codegen::check(&zdc_codegen::Inputs {
                    hir,
                    split,
                    verdict,
                    table: types,
                    cleared,
                })
                .into_iter()
                .map(Diagnostic::from)
                .filter(|diagnostic| in_this_file(diagnostic, here)),
            );
        }
    }
    let types = solved.map(|(_, _, types)| types);

    // Indexed over the *linked* program rather than over the entry file's
    // own tree, so that one index covers every file the program reaches.
    // Find-references, rename, the outline and document highlight are all
    // reads of this index, and an index that stopped at the open document
    // would let rename miss the use it was supposed to rewrite, which is
    // worse than not offering rename, because the file is left broken.
    //
    // Widening it is safe for the features that ask about the cursor:
    // the entry file's text leads the combined buffer, so its offsets are
    // unchanged and no imported module's span can collide with one.
    let symbols = match &linked {
        Some(linked) => index(&linked.program, hir.as_ref(), &linked_tokens(linked)),
        None => index(&program, hir.as_ref(), &tokens),
    };

    Analysis {
        text: text.to_string(),
        lines,
        diagnostics,
        tokens,
        symbols,
        hir,
        types,
        linked,
    }
}

/// Every module's tokens, shifted into the combined buffer.
///
/// The loader shifts a token stream exactly like this and then drops it
/// (`zdc_resolve::parse_at`), so this is a second lex rather than a
/// different one. Re-lexing costs a fraction of the passes that follow,
/// and it is what lets the symbol index span every file: the index needs
/// the token stream to tell the three jobs of `is` apart, and it can only
/// do that for text it has tokens for.
///
/// Modules are read in offset order, so the result is already sorted by
/// start offset, which is what the index's bisection requires. A module
/// that does not lex contributes nothing rather than aborting the rest:
/// its own diagnostic is already on its way from the loader.
fn linked_tokens(linked: &zdc_resolve::Linked) -> Vec<Token> {
    let mut out = Vec::new();
    for module in &linked.modules {
        let Ok(tokens) = zdc_lexer::tokenize(&module.source) else {
            continue;
        };
        out.extend(tokens.into_iter().map(|mut token| {
            token.span = Span::new(
                token.span.start.saturating_add(module.offset),
                token.span.end.saturating_add(module.offset),
            );
            token
        }));
    }
    out
}

/// Whether the program borrows anything from another file.
fn imports(program: &zdc_ast::Program) -> bool {
    program
        .decls
        .iter()
        .any(|decl| matches!(decl, zdc_ast::Decl::Use(_)))
}

/// Whether a diagnostic points inside the document being analysed.
///
/// Linking concatenates every module's text and the entry file comes
/// first, so anything past its length belongs to a file this editor window
/// is not showing.
fn in_this_file(diagnostic: &Diagnostic, here: u32) -> bool {
    diagnostic.span.is_none_or(|span| span.start < here.max(1))
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
    /// Not every checked-in example is one of those, and the list shrank
    /// when `zdc check` started running the emitter. `guestbook.zd` and
    /// `voting-board.zd` declare `server` and `durable` state, which the
    /// emitter refuses until M6 (§16.5) — they were only ever "accepted"
    /// because the command they were checked with stopped a pass early.
    /// `blog.zd` writes `use` (§14D.2), `components.zd` declares components
    /// (§14D.1), `leaderboard.zd` has a type error, and `model.zd` is a
    /// module with no `view`. What every example is useful for here is the
    /// harder property, which the test below asserts without exception.
    #[test]
    fn the_examples_the_compiler_accepts_analyse_cleanly() {
        let accepted = [
            "counter.zd",
            "disclosure.zd",
            "events.zd",
            "hello.zd",
            "page.zd",
            "todo.zd",
        ];
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
    ///
    /// Code generation is the last step of it, and its output is dropped.
    /// Leaving it out here would make this test pass while the editor and
    /// the command line disagreed about every refusal codegen owns, which
    /// is exactly the state this comparison exists to rule out.
    fn compiler_diagnostics(src: &str) -> Vec<Diagnostic> {
        let program = match zdc_parser::parse(src) {
            Ok(program) => program,
            Err(error) => return vec![Diagnostic::from(error)],
        };
        let hir = match zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
            .resolve()
        {
            Ok(hir) => hir,
            Err(errors) => return errors.into_iter().map(Diagnostic::from).collect(),
        };
        let split = zdc_graph::split(&hir);
        if split.has_errors() {
            return split.errors().cloned().map(Diagnostic::from).collect();
        }
        let verdict = zdc_graph::ifc(&hir, &split);
        let mut found: Vec<Diagnostic> = verdict
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .map(Diagnostic::from)
            .collect();
        let table = match zdc_types::check(&hir, &split) {
            Ok(table) => table,
            Err(errors) => {
                found.extend(errors.into_iter().map(Diagnostic::from));
                return found;
            }
        };
        // Code generation is the last step, and its output is dropped.
        // Leaving it out would make this test pass while the editor and
        // the command line disagreed about every refusal codegen owns.
        if let (true, Some(cleared)) = (found.is_empty(), verdict.clearance()) {
            found.extend(
                zdc_codegen::check(&zdc_codegen::Inputs {
                    hir: &hir,
                    split: &split,
                    verdict: &verdict,
                    table: &table,
                    cleared,
                })
                .into_iter()
                .map(Diagnostic::from),
            );
        }
        found
    }
}
