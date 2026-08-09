//! Documentation generated from a program's own declarations.
//!
//! Memory safety is a mechanically verified property of this compiler,
//! not a claim: no crate in this workspace may contain `unsafe`.
//!
//! # What this generates, and why it is Markdown
//!
//! Markdown, written to a directory, one page per source file plus an
//! overview. It was chosen over HTML for one reason: **HTML would need
//! design decisions this language deliberately refuses to make.** Every
//! visual choice in ZDeceptron is pushed to `class is` and to the
//! stylesheet a program ships, so a documentation generator that emitted
//! styled HTML would be the one part of the toolchain with opinions about
//! type scale and colour. Markdown composes instead: it renders on a
//! repository host without a build step, it is what a static-site
//! generator already takes as input, and `zdc doc && pandoc` is a shorter
//! path to HTML than a `--format html` flag would be to anything else.
//!
//! A generated page is disposable. Nothing here writes into a source tree
//! it did not create and nothing merges with an existing file, so the
//! output directory can always be deleted and regenerated.
//!
//! # The four things a page says that another language's cannot
//!
//! 1. **Where each piece of state lives.** `client`, `static`, `server`
//!    and `durable` are on the left of a `state` declaration, so the
//!    overview's first table is a program's whole deployment shape, read
//!    off the source. Elsewhere this is spread across a router, an ORM and
//!    a deployment manifest, and no generator reads all three.
//! 2. **Where the network is.** Reading a `server` or `durable` signal
//!    from the browser yields `Remote of T`, and [`prose::from_the_browser`]
//!    asks §14G.1.4's read table — the same function the type checker calls
//!    — rather than restating the rule. A row cannot claim `Text` where the
//!    checker would say `Remote of Text`.
//! 3. **Which endpoints exist.** Nobody declared them. The tier split
//!    derived them from the placements, and the table names the file each
//!    one is emitted to using the emitter's own naming function.
//! 4. **What cannot be read at all.** A `secret` is not readable from the
//!    browser at any type, and the column says so instead of printing the
//!    type a read would have had.
//!
//! # Comments
//!
//! They do not survive lexing — see [`comments`], which explains what that
//! costs, how the harvest works around it without guessing, and what
//! preserving them properly would take.
//!
//! # The prelude documents itself
//!
//! Issue #170's complaint was that the standard library is eight files of
//! ZDeceptron whose only reader is someone who opens them. It cannot be
//! documented by pointing this command at `prelude/list.zd`, because every
//! entry point compiles *against* the library and the file would collide
//! with itself — so [`library::linked`] compiles it as the program instead.
//! `zdc doc --prelude` is that path, and it goes through the same resolver,
//! split and type checker every other page does.
//!
//! # This crate owns the words, and the language server borrows them
//!
//! [`prose`] holds every sentence the compiler says about a declaration,
//! and `zdc-lsp`'s hover calls into it. There is one implementation of
//! "what does `durable` mean", not one here and one there, so the
//! generated documentation and the editor cannot drift apart.
#![forbid(unsafe_code)]

pub mod comments;
pub mod library;
mod pages;
pub mod prose;

use std::path::{Path, PathBuf};

use zdc_graph::TierSplit;
use zdc_hir::Hir;
use zdc_resolve::Linked;
use zdc_types::TypeTable;

/// Everything the front end produced that documentation is written from.
///
/// The type table and the split are not optional, and that is the design:
/// the two claims worth making — where a signal lives, and what a read of
/// it costs — are answers *this compiler already computed*. A generator
/// that took only the HIR would have to recompute them and could disagree
/// with the bundle it documents.
pub struct Inputs<'a> {
    pub hir: &'a Hir,
    pub split: &'a TierSplit,
    pub table: &'a TypeTable,
    /// The linked module set, which is what turns a span back into the
    /// file it came from — and so what lets a page exist per source file
    /// and a comment be found in the text it was written in.
    pub linked: &'a Linked,
    /// What the overview is about, for its title and its first sentence.
    pub subject: Subject<'a>,
}

/// What a set of pages documents.
///
/// The prelude is not a path, and giving it one would be a small lie with a
/// visible consequence: the overview would name a file, a reader would look
/// for it, and the library is eight files compiled as one unit with no
/// entry among them. So the two cases are distinguished here rather than
/// papered over with a placeholder path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject<'a> {
    /// A program, named by the file `zdc doc` was pointed at.
    Program(&'a Path),
    /// The standard library, documented as its own program by
    /// [`library::linked`].
    Prelude,
}

/// One generated page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocFile {
    /// Relative to the output directory. The caller joins and writes,
    /// exactly as it does for a bundle: this crate computes and touches no
    /// filesystem.
    pub path: PathBuf,
    pub text: String,
}

/// Render a checked program as Markdown.
///
/// Infallible by construction. Everything it reads has already been
/// resolved, split and typechecked, so there is no question left that
/// could be answered with an error — and a documentation generator that
/// could fail on a program the compiler accepted would be a second
/// front end.
pub fn render(inputs: &Inputs<'_>) -> Vec<DocFile> {
    pages::render(inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile a source text far enough to document it.
    fn documented(source: &str) -> Vec<DocFile> {
        let program = zdc_parser::parse(source).expect("the fixture parses");
        let linked = Linked::single("fixture.zd", source.to_string(), program);
        let prelude = zdc_lib::load();
        let hir = zdc_resolve::Resolver::linked_with_prelude(prelude.program(), &linked)
            .resolve()
            .expect("the fixture resolves");
        let split = zdc_graph::split(&hir);
        assert!(!split.has_errors(), "the fixture splits");
        let table = zdc_types::check(&hir, &split).expect("the fixture typechecks");
        render(&Inputs {
            hir: &hir,
            split: &split,
            table: &table,
            linked: &linked,
            subject: Subject::Program(Path::new("fixture.zd")),
        })
    }

    /// Compile the standard library the way `zdc doc --prelude` does.
    fn documented_prelude() -> Vec<DocFile> {
        let linked = library::linked();
        let hir = library::resolve(&linked).expect("the prelude resolves");
        let split = zdc_graph::split(&hir);
        assert!(!split.has_errors(), "the prelude splits");
        let table = zdc_types::check(&hir, &split).expect("the prelude typechecks");
        render(&Inputs {
            hir: &hir,
            split: &split,
            table: &table,
            linked: &linked,
            subject: Subject::Prelude,
        })
    }

    /// #170's complaint, answered: the library's surface is readable
    /// without opening the files, and each file keeps its own page.
    #[test]
    fn the_prelude_gets_a_page_per_file_and_an_overview() {
        let files = documented_prelude();
        assert_eq!(files.len(), zdc_lib::SOURCES.len() + 1);
        let index = page(&files, "index.md");
        assert!(index.contains("The ZDeceptron prelude"), "{index}");
        for (path, _) in zdc_lib::SOURCES {
            let name = Path::new(path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("a stem");
            assert!(
                index.contains(&format!("{name}.md")),
                "{index} lacks {name}"
            );
        }
    }

    /// A declaration has to land on the page of the file it was written
    /// in. Every prelude file's spans start at zero before they are
    /// shifted, so this is the assertion that catches losing the shift.
    #[test]
    fn each_prelude_declaration_is_on_the_page_of_the_file_it_was_written_in() {
        let files = documented_prelude();
        assert!(page(&files, "list.md").contains("### `listLength`"));
        assert!(page(&files, "text.md").contains("### `textLength`"));
        assert!(!page(&files, "text.md").contains("### `listLength`"));
        assert!(page(&files, "option.md").contains("### `valueOr`"));
    }

    /// The library is colourless (§17.4.1 step 6), and the overview says
    /// so from the declarations rather than from a constant.
    #[test]
    fn the_prelude_overview_reports_no_state_and_no_endpoints() {
        let index = page(&documented_prelude(), "index.md");
        assert!(index.contains("declares no state"), "{index}");
        assert!(index.contains("derived no endpoints"), "{index}");
        assert!(!index.contains("Read from the browser as"), "{index}");
    }

    fn page(files: &[DocFile], name: &str) -> String {
        files
            .iter()
            .find(|file| file.path.to_string_lossy() == name)
            .unwrap_or_else(|| {
                panic!(
                    "no page {name}; there is {:?}",
                    files.iter().map(|f| &f.path).collect::<Vec<_>>()
                )
            })
            .text
            .clone()
    }

    #[test]
    fn a_program_of_one_file_gets_an_overview_and_one_page() {
        let files = documented("state n is client Whole starting 0\nview\n    Text n\n");
        assert_eq!(files.len(), 2);
        assert!(page(&files, "index.md").contains("Where the state lives"));
        assert!(page(&files, "fixture.md").contains("### `n`"));
    }

    /// The overview must reach the same answer the type checker did, for
    /// the read that made this language worth writing.
    #[test]
    fn a_durable_signal_is_documented_as_remote_and_a_client_one_is_not() {
        let files = documented(
            "state votes is durable Whole starting 0\nstate draft is client Text starting \
             \"\"\nview\n    Text draft\n",
        );
        let index = page(&files, "index.md");
        let votes = index
            .lines()
            .find(|line| line.contains("`votes`") && line.starts_with('|'))
            .expect("a row for votes");
        assert!(votes.contains("Remote of Whole"), "{votes}");

        let draft = index
            .lines()
            .find(|line| line.contains("`draft`") && line.starts_with('|'))
            .expect("a row for draft");
        assert!(!draft.contains("Remote"), "{draft}");
    }

    /// The comment is the only part of a page a human wrote, so losing it
    /// is the failure that matters most.
    #[test]
    fn a_comment_above_a_declaration_reaches_its_page() {
        let files = documented(
            "# Counts the visitors.\nstate n is client Whole starting 0\nview\n    Text n\n",
        );
        assert!(page(&files, "fixture.md").contains("Counts the visitors."));
    }

    /// A program with no crossing must not print an empty table under a
    /// heading that promises one.
    #[test]
    fn a_program_with_no_crossing_says_so_rather_than_printing_an_empty_table() {
        let files = documented("state n is client Whole starting 0\nview\n    Text n\n");
        let index = page(&files, "index.md");
        assert!(index.contains("derived no endpoints"), "{index}");
        assert!(!index.contains("Emitted to"), "{index}");
    }
}
