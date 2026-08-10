#![forbid(unsafe_code)]

//! Rendering for compiler diagnostics.
//!
//! Spec §7.3: diagnostics are a primary deliverable. Because the grammar
//! admits exactly one phrasing per construct (§4.1), every syntax error
//! must be able to name that phrasing.
//!
//! Naming it is not the same as explaining it, and the two have different
//! costs. Barik et al. measured that reading error messages consumes
//! 13–25% of a developer's fixations and that reading difficulty predicts
//! task time, so what a diagnostic says inline is budgeted: the claim, the
//! spans, and one line pointing at [`explain`]. The rule itself — why it
//! exists, and a worked repair — lives in [`explain`] and is printed on
//! request by `zdc explain <CODE>`.

pub mod explain;
pub mod json;

use std::collections::BTreeMap;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind, Source};
use zdc_lexer::Span;

pub use explain::{explain, Explanation, INLINE_MESSAGE_BUDGET};

/// Whether a diagnostic stops the build.
///
/// The distinction already existed in the code list — `W0330` and `W0331`
/// are warnings and everything else is an error — and existed again as
/// `zdc_graph::Severity`, but it stopped at this crate's door: every
/// `Diagnostic` was rendered as an error and the CLI filtered warnings out
/// rather than printing them, so two of the compiler's diagnostics could
/// not reach a reader at all.
///
/// Carrying the level on the diagnostic is what lets one renderer print
/// both, one serialiser distinguish them, and a [`Policy`] change one into
/// the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// The program is rejected.
    Error,
    /// The program compiles, and the compiler has something to say about
    /// it.
    Warning,
}

impl Level {
    /// The level a code carries before any [`Policy`] is applied.
    ///
    /// Read from the code rather than from a table beside it: the spec
    /// spells the level into the first character of every code it
    /// allocates (§7.3), so `W0330` is a warning by the same act that
    /// named it. A table would be a second place for the answer to live
    /// and a second place for it to go stale.
    pub fn of(code: &str) -> Level {
        if code.starts_with('W') {
            Level::Warning
        } else {
            Level::Error
        }
    }

    /// The word a report is introduced by.
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
        }
    }

    pub fn is_error(self) -> bool {
        self == Level::Error
    }
}

/// What a reader has decided about one code.
///
/// The three settings are the three things a reader can want from a
/// warning, and they are named for what they do to the *level* rather than
/// for a command-line spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// Do not report it at all.
    Silence,
    /// Report it as a warning: the default, written down so that a code
    /// silenced project-wide can be turned back on for one invocation.
    Warn,
    /// Report it as an error, so it stops the build.
    Deny,
}

/// Which diagnostics are reported, and at what level.
///
/// **An error is never demoted and never silenced.** A `Policy` can raise
/// a warning to an error and can drop a warning entirely; it cannot turn a
/// rejection into a remark. The asymmetry is the whole point: a compiler
/// with a flag that silences errors is a compiler whose exit code means
/// nothing, and every rule in this language exists because the alternative
/// was a program that runs and is wrong. Warnings are the ones a reader is
/// entitled to disagree about, so warnings are the ones this type can
/// move.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Every warning becomes an error. What `--deny-warnings` sets.
    deny_warnings: bool,
    /// Per-code decisions, which beat `deny_warnings` because they are the
    /// more specific statement.
    by_code: BTreeMap<String, Setting>,
}

impl Policy {
    pub fn new() -> Policy {
        Policy::default()
    }

    /// Report every warning as an error.
    pub fn deny_warnings(mut self) -> Policy {
        self.deny_warnings = true;
        self
    }

    /// Decide one code, overriding [`Policy::deny_warnings`] for it.
    pub fn set(mut self, code: impl Into<String>, setting: Setting) -> Policy {
        self.by_code.insert(code.into(), setting);
        self
    }

    /// The level this diagnostic is reported at, or `None` when the policy
    /// silences it.
    ///
    /// A diagnostic with no code cannot be named on a command line, so it
    /// is reported at the level it arrived with. That is not a gap to fill
    /// later: `--allow` takes a code, and a policy that silenced by
    /// message text would be silencing whatever the message becomes.
    pub fn level_of(&self, diagnostic: &Diagnostic) -> Option<Level> {
        if diagnostic.level.is_error() {
            return Some(Level::Error);
        }
        match diagnostic.code.and_then(|code| self.by_code.get(code)) {
            Some(Setting::Silence) => None,
            Some(Setting::Warn) => Some(Level::Warning),
            Some(Setting::Deny) => Some(Level::Error),
            None if self.deny_warnings => Some(Level::Error),
            None => Some(Level::Warning),
        }
    }

    /// Apply the policy in place, reporting whether the diagnostic
    /// survived it.
    pub fn apply(&self, diagnostic: &mut Diagnostic) -> bool {
        match self.level_of(diagnostic) {
            Some(level) => {
                diagnostic.level = level;
                true
            }
            None => false,
        }
    }
}

/// The policy for this process, set once before anything prints.
///
/// A process-wide setting for the same reason [`disable_colour`] is one:
/// it is decided by the invocation, before any pass runs, and threading it
/// through every function that might eventually print would say nothing
/// the flag does not. [`Policy`] itself is an ordinary value, so a test
/// exercises the rules without touching this.
static POLICY: std::sync::OnceLock<Policy> = std::sync::OnceLock::new();

/// Fix the process's diagnostic policy. The first call wins; a second is
/// ignored, because a policy that changed half way through a run would
/// report two files under two rules.
pub fn set_policy(policy: Policy) {
    let _ = POLICY.set(policy);
}

/// The process's diagnostic policy, which reports every warning as a
/// warning until [`set_policy`] says otherwise.
pub fn policy() -> &'static Policy {
    POLICY.get_or_init(Policy::default)
}

/// A diagnostic either points at a byte span within a known source text
/// (a parse error), or has no location at all (a file-level error: the
/// file could not be found, read, or decoded). These are deliberately
/// distinct at the type level — `Option<Span>`, not a sentinel span like
/// `Span::new(0, 0)`, which would render a caret pointing at a byte that
/// does not exist.
///
/// A diagnostic may also carry **notes**: further spans, each with its own
/// message, rendered as additional labels on the same report. Spec §7.3
/// asks the information-flow pass to "show the path along which the secret
/// would have escaped", and a path is inherently more than one span. One
/// label per step is what makes an escape readable rather than merely
/// reported (§17.2.2(d), §17.3.8).
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Option<Span>,
    /// What the caret says about the span it points at.
    ///
    /// This used to be the literal string `here`, for every diagnostic in
    /// the compiler. `here` is where the caret already is, so the one line
    /// with room to say what the compiler found was spending it on a word
    /// the reader could not act on: for a `state` declaration missing its
    /// placement the caret sits under a *type*, and saying so is the whole
    /// repair.
    ///
    /// `None` draws the underline with no words beside it. That is the
    /// deliberate fallback rather than a generic phrase, because the
    /// alternative to a label that says something is silence, not a
    /// synonym for `here`.
    pub label: Option<String>,
    /// Further spans, in the order they should be read. Rendered as
    /// secondary labels, so `ariadne` draws the whole path at once.
    pub notes: Vec<(Span, String)>,
    pub help: Option<String>,
    /// An edit that would make the line parse, rendered as the whole
    /// corrected line rather than as a description of it.
    pub suggestion: Option<Suggestion>,
    /// The spec code, for the diagnostics that have one. A code is what
    /// makes progressive disclosure possible: it is the handle the reader
    /// passes to `zdc explain`, and it is stable across every rewording of
    /// the message.
    pub code: Option<&'static str>,
    /// Whether this stops the build.
    ///
    /// Set from the producing pass — `zdc_graph::Severity` for a graph
    /// finding, [`Level::of`] for anything else that carries a code — and
    /// then possibly changed by a [`Policy`]. Every other pass in the
    /// compiler produces errors only, so the field is `Error` for them by
    /// fact rather than by default.
    pub level: Level,
}

/// One edit, expressed against the source rather than as prose.
///
/// The compiler knows the byte range and the text that belongs in it; the
/// reader wants the line. Carrying the edit and rendering the line from
/// the source means the shown line is the reader's own, character for
/// character, and that the same value is what an editor's quick fix would
/// apply (§7.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// The byte range this replaces. An empty range is an insertion.
    pub span: Span,
    /// What goes in that range.
    pub replacement: String,
}

impl Diagnostic {
    /// A diagnostic about a file rather than about a location within one:
    /// the file could not be read, was not found, or is not valid UTF-8.
    pub fn file_error(message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            message: message.into(),
            span: None,
            label: None,
            notes: Vec::new(),
            help: None,
            suggestion: None,
            code: None,
            level: Level::Error,
        }
    }
}

/// A parse error names the single valid form of what it was reading
/// (§4.1), so it always has a code and usually has something to say about
/// the span it points at. Both are carried rather than reconstructed here:
/// the parser is the pass that knows which construct it was in the middle
/// of, and a renderer guessing from the message would be guessing.
impl From<zdc_parser::ParseError> for Diagnostic {
    fn from(e: zdc_parser::ParseError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            label: e
                .label
                .or_else(|| explain::caret(e.code).map(str::to_string)),
            notes: Vec::new(),
            help: Some(explain::inline_help(e.code)),
            suggestion: e.suggestion.map(|s| Suggestion {
                span: s.span,
                replacement: s.replacement,
            }),
            code: Some(e.code),
            // The parser rejects; it never remarks.
            level: Level::Error,
        }
    }
}

/// Module loading parses, so a parse error reaches most readers as one of
/// these. Everything the parser knew is carried rather than rebuilt.
impl From<zdc_resolve::ResolveError> for Diagnostic {
    fn from(e: zdc_resolve::ResolveError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            label: e
                .label
                .or_else(|| e.code.and_then(explain::caret).map(str::to_string)),
            notes: Vec::new(),
            help: e.code.map(explain::inline_help),
            suggestion: e.suggestion.map(|s| Suggestion {
                span: s.span,
                replacement: s.replacement,
            }),
            code: e.code,
            level: Level::Error,
        }
    }
}

/// A type error already carries its own help text, because §7.3 asks a
/// diagnostic to name what was expected, what was found, and where — and
/// for the exhaustiveness rules the "why" belongs in help rather than in
/// the message.
impl From<zdc_types::TypeError> for Diagnostic {
    fn from(e: zdc_types::TypeError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            label: None,
            notes: Vec::new(),
            help: e.help,
            suggestion: None,
            code: None,
            level: Level::Error,
        }
    }
}

/// The placement and information-flow passes carry a spec code and, more
/// importantly, a **path**: §17.2.10 prints "reached: hourly → ingest →
/// name" and §17.3.8 prints the steps a secret would take to escape.
/// Neither is expressible as one span, which is why `notes` exists.
///
/// The help line is generated rather than carried. A coded diagnostic's
/// prose lives in [`explain`], in one place, so there is nowhere for the
/// inline text and the full rule to drift apart — and the inline form
/// stays inside the budget by construction rather than by review.
impl From<zdc_graph::GraphError> for Diagnostic {
    fn from(e: zdc_graph::GraphError) -> Self {
        // `render` already prefixes "Error:", so the code is bracketed
        // rather than re-spelling the word: `Error: [E-IFC-05] …`.
        Diagnostic {
            message: format!("[{}] {}", e.code, e.message),
            span: Some(e.span),
            // The caret label comes from the code rather than from the
            // reporting site. A coded finding's primary span is always the
            // same *kind* of thing for a given code, so the label is a
            // fact about the rule, and putting it beside the rule's other
            // prose keeps the wording in one place (§7.3).
            label: explain::caret(e.code).map(str::to_string),
            notes: e.notes,
            help: Some(explain::inline_help(e.code)),
            suggestion: None,
            code: Some(e.code),
            // The pass already decided, and it carries the answer as a
            // field. `Level::of` is the fallback for a producer that has
            // no severity of its own.
            level: match e.severity {
                zdc_graph::Severity::Error => Level::Error,
                zdc_graph::Severity::Warning => Level::Warning,
            },
        }
    }
}

/// A claim the program contradicted — issue #169.
///
/// # Why a broken test is a diagnostic and not a report of its own
///
/// The alternative was the shape every other language uses: a test
/// framework with its own output format, its own idea of what a failure
/// looks like, and its own vocabulary. A reader of this compiler has
/// already learnt one shape — the claim, the span, the repair (§7.3) —
/// and a second one would be a second thing to learn for the same
/// information.
///
/// So a false expectation renders through exactly the path a type error
/// renders through: same code, same caret, same `zdc explain` handle. The
/// two values go in `notes`… no — they go in the message, because a note
/// needs a span of its own and neither side of an `is` has one that is
/// worth pointing at separately.
impl From<zdc_codegen::Broken> for Diagnostic {
    fn from(e: zdc_codegen::Broken) -> Self {
        // The claim is quoted rather than paraphrased. It is the sentence
        // the programmer wrote, and a report that reworded it would be
        // reporting on something they cannot search their file for.
        let mut message = format!("[{}] the claim `{}` is false.", e.code, e.claim);
        if let Some((left, right)) = &e.sides {
            // The two sides, on the message rather than as a second span.
            // What the reader needs is the pair of *values*, and a value
            // has no place in the file to point at — the expression that
            // produced it is already under the caret.
            message.push_str(&format!(
                " Left is {}; right is {}.",
                abbreviated(left),
                abbreviated(right)
            ));
        }
        Diagnostic {
            // `of` rather than a literal `Error`, for the reason it
            // exists: the level is spelled into the code (§7.3), and
            // `E-TEST-01` is the code this carries. Writing the level out
            // again here would be a second place for it to disagree with
            // the code beside it.
            level: Level::of(e.code),
            message,
            span: Some(e.span),
            label: explain::caret(e.code).map(str::to_string),
            notes: Vec::new(),
            help: Some(explain::inline_help(e.code)),
            suggestion: None,
            code: Some(e.code),
        }
    }
}

/// A rendered value, short enough to sit in a headline — issue #169.
///
/// A claim about a thousand-element list would otherwise put a thousand
/// elements in the message, which is over [`INLINE_MESSAGE_BUDGET`] by two
/// orders of magnitude and unreadable besides. The head of the value is
/// what tells a reader whether they are looking at the wrong *shape* or
/// the wrong *contents*, which is the question a headline should answer;
/// anything past that is a question for the program, not the message.
///
/// Cut on a character boundary, because a rendered value can hold any text
/// the program computed.
fn abbreviated(value: &str) -> String {
    const KEEP: usize = 72;
    if value.chars().count() <= KEEP {
        return value.to_string();
    }
    let head: String = value.chars().take(KEEP).collect();
    format!("{head}\u{2026}")
}

impl From<zdc_codegen::CodegenError> for Diagnostic {
    fn from(e: zdc_codegen::CodegenError) -> Self {
        Diagnostic {
            message: e.message,
            span: Some(e.span),
            label: None,
            notes: Vec::new(),
            help: None,
            suggestion: None,
            code: None,
            level: Level::Error,
        }
    }
}

/// Whether diagnostics should carry colour.
///
/// `NO_COLOR` set to anything at all turns it off — the convention is
/// presence, not value, so `NO_COLOR=0` means the same as `NO_COLOR=1`
/// and a caller who wants colour unsets it. See <https://no-color.org>.
///
/// [`disable_colour`] is the other half: a `--no-color` flag has to work
/// on a machine whose environment says nothing (#153).
pub fn colour_enabled() -> bool {
    !FORCED_OFF.load(std::sync::atomic::Ordering::Relaxed) && std::env::var_os("NO_COLOR").is_none()
}

/// Turn colour off for the rest of the process, whatever the environment
/// says. What `--no-color` calls.
pub fn disable_colour() {
    FORCED_OFF.store(true, std::sync::atomic::Ordering::Relaxed);
}

static FORCED_OFF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Which form a rendered diagnostic takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// A report drawn for a person to read: the source line, a caret, and
    /// colour when the terminal wants it.
    #[default]
    Human,
    /// One JSON object per diagnostic, one per line. See [`json`] for the
    /// shape and for why it is line-delimited.
    Json,
}

static FORMAT: std::sync::OnceLock<Format> = std::sync::OnceLock::new();

/// Fix the output form for this process. The first call wins, for the same
/// reason [`set_policy`]'s does: a run that changed form half way through
/// would produce a stream no consumer could read.
pub fn set_format(format: Format) {
    let _ = FORMAT.set(format);
}

/// The output form, which is [`Format::Human`] until [`set_format`] says
/// otherwise.
pub fn format() -> Format {
    *FORMAT.get_or_init(Format::default)
}

/// Render a diagnostic as a report against the source text.
///
/// A spanless (file-level) diagnostic has no source text to snippet and no
/// byte range to point a caret at, so it is formatted directly rather than
/// forcing a fake span through `ariadne`.
///
/// Colour follows [`colour_enabled`]. Use [`render_in_colour`] to decide
/// per call — which the tests do, because the environment is process-wide
/// and they run in parallel.
///
/// The output form follows [`format`], so a caller that already prints a
/// diagnostic prints it as JSON under `--format json` without knowing that
/// the option exists. That is the reason the choice is made here rather
/// than at each of the fifteen call sites: a machine-readable mode that
/// half the compiler honoured would be worse than none.
pub fn render(src: &str, path: &str, diagnostic: &Diagnostic) -> String {
    match format() {
        Format::Human => render_in_colour(src, path, diagnostic, colour_enabled()),
        Format::Json => json::line(src, path, diagnostic),
    }
}

/// The same, with colour decided by the caller.
pub fn render_in_colour(src: &str, path: &str, diagnostic: &Diagnostic, colour: bool) -> String {
    let diagnostic = &Diagnostic {
        message: printable(&diagnostic.message),
        span: diagnostic.span,
        label: diagnostic.label.as_deref().map(printable),
        notes: diagnostic
            .notes
            .iter()
            .map(|(span, note)| (*span, printable(note)))
            .collect(),
        help: diagnostic.help.as_deref().map(printable),
        suggestion: diagnostic.suggestion.as_ref().map(|s| Suggestion {
            span: s.span,
            replacement: printable(&s.replacement),
        }),
        code: diagnostic.code,
        level: diagnostic.level,
    };
    let src = &printable(src);
    let path = &printable(path);

    let Some(span) = diagnostic.span else {
        return render_file_error(path, diagnostic);
    };

    let range: std::ops::Range<usize> = span.into();

    // `Span` is a byte range — the lexer produces byte offsets and every
    // pass carries them unchanged — while `ariadne` counts characters by
    // default. Left alone, a single `#` comment containing an em dash
    // slides every caret in the file, which seven of the eight checked-in
    // examples would do.
    // A site with nothing to add draws the underline and no words. `here`
    // was worse than silence: it occupied the one line that could have
    // named what the caret covers, and it said where the caret already is.
    //
    // The empty string rather than no message at all, because `ariadne`
    // draws the underline as part of the arrow that carries a message and
    // omits the whole row when there is none. Dropping the underline was
    // not the trade being made: a reader without colour would lose the
    // only mark saying which characters the diagnostic is about.
    let start = range.start;
    let caret = Label::new((path, range))
        .with_color(Color::Red)
        .with_message(diagnostic.label.as_deref().unwrap_or(""));

    let kind = match diagnostic.level {
        Level::Error => ReportKind::Error,
        Level::Warning => ReportKind::Warning,
    };
    let mut builder = Report::build(kind, path, start)
        .with_config(
            Config::default()
                .with_index_type(IndexType::Byte)
                .with_color(colour),
        )
        .with_message(&diagnostic.message)
        .with_label(caret);

    // The repair is printed as the whole line the reader would have to
    // have written, spliced out of their own source, rather than described
    // in prose. Prose describing an edit still leaves the edit to be made.
    if let Some(suggestion) = &diagnostic.suggestion {
        if let Some(line) = apply(src, suggestion) {
            builder = builder.with_note(format!("the line as it would be accepted: {line}"));
        }
    }

    // Notes are ordered: step one of an escape path must render above step
    // two. `ariadne` orders labels by their span, not by insertion, so the
    // order is restated in the message rather than left to the layout.
    for (step, (span, message)) in diagnostic.notes.iter().enumerate() {
        let range: std::ops::Range<usize> = (*span).into();
        builder = builder.with_label(
            Label::new((path, range))
                .with_message(format!("{}. {message}", step + 1))
                .with_color(Color::Yellow)
                .with_order(step as i32),
        );
    }

    if let Some(help) = &diagnostic.help {
        builder = builder.with_help(help);
    }

    let mut buffer = Vec::new();
    builder
        .finish()
        .write((path, Source::from(src)), &mut buffer)
        .expect("writing to an in-memory buffer cannot fail");

    String::from_utf8(buffer).expect("ariadne emits valid UTF-8")
}

/// Every C0 control except tab and newline replaced by `?`, byte for byte.
///
/// A diagnostic quotes the program back at whoever is reading it: the
/// message interpolates the program's own names and string literals, and
/// the snippet is the source line itself. A `.zd` string literal is
/// `"[^"\n]*"`, which admits U+001B — so `state a is client Text starting
/// "\u{1b}[2J\u{1b}[H"` is a *compiler diagnostic* that clears the
/// reader's terminal, and one carrying `\u{1b}]0;…\u{7}` retitles the
/// window. A file that fails to compile is exactly the file least likely
/// to have been read first.
///
/// The substitution is one byte for one byte, and every byte it touches is
/// below 0x80, so it can never fall inside a multi-byte sequence and every
/// [`Span`] in the file still points where it did. A renderer that
/// stripped these instead would slide every caret after the first one.
fn printable(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    for byte in bytes.iter_mut() {
        let control = *byte < 0x20 || *byte == 0x7f;
        if control && *byte != b'\t' && *byte != b'\n' {
            *byte = b'?';
        }
    }
    String::from_utf8(bytes).expect("only sub-0x80 bytes were replaced, by another such byte")
}

/// The one source line a suggestion touches, with the edit made.
///
/// Returns `None` when the edit does not fall inside `src` or would cut a
/// character in half. A suggestion is an aid, so a suggestion the renderer
/// cannot place is dropped rather than allowed to panic or to print a line
/// that is not the reader's: the diagnostic itself is still correct
/// without it.
///
/// Only one line is spliced because a repair that spans lines is not a
/// line a reader can copy, and no site produces one.
fn apply(src: &str, suggestion: &Suggestion) -> Option<String> {
    let start = suggestion.span.start as usize;
    let end = suggestion.span.end as usize;
    if end < start || end > src.len() {
        return None;
    }
    if !src.is_char_boundary(start) || !src.is_char_boundary(end) {
        return None;
    }

    let line_start = src[..start].rfind('\n').map_or(0, |at| at + 1);
    let line_end = src[end..].find('\n').map_or(src.len(), |at| end + at);
    if line_end < end {
        return None;
    }

    let mut line = String::new();
    line.push_str(&src[line_start..start]);
    line.push_str(&suggestion.replacement);
    line.push_str(&src[end..line_end]);
    Some(line)
}

/// Render a file-level diagnostic: message and path, no snippet, no caret.
fn render_file_error(path: &str, diagnostic: &Diagnostic) -> String {
    use std::fmt::Write as _;

    // Capitalised to match `ariadne`'s own heading for the spanned case,
    // so the two rendering paths introduce a finding the same way.
    let heading = match diagnostic.level {
        Level::Error => "Error",
        Level::Warning => "Warning",
    };
    let mut out = String::new();
    let _ = writeln!(out, "{heading}: {}", diagnostic.message);
    let _ = writeln!(out, "  --> {path}");
    if let Some(help) = &diagnostic.help {
        let _ = writeln!(out, "  help: {help}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ariadne` colors the highlighted source line character-by-character,
    /// which splits multi-character substrings with ANSI escapes. Strip
    /// them so tests can assert on plain text.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn rendered_output_contains_the_source_line_and_message() {
        let src = "state votes is Map of Id to Int starting empty";
        let error = zdc_parser::parse(src).unwrap_err();
        let out = render(src, "example.zd", &Diagnostic::from(error));
        assert!(out.contains("example.zd"), "missing path:\n{out}");
        assert!(out.contains("client"), "missing the valid forms:\n{out}");
    }

    /// A `.zd` string literal is `"[^"\n]*"`, so it admits U+001B. The
    /// snippet a diagnostic quotes is the source line itself, and a
    /// terminal reading `\u{1b}[2J` clears itself. This is the one path
    /// where a string a program wrote reaches something that *interprets*
    /// it without ever passing through the emitter.
    #[test]
    fn a_source_file_cannot_write_escape_sequences_to_the_terminal() {
        let src = "state a is client Text starting \"\u{1b}[2J\u{1b}]0;pwned\u{7}\"\nnope\n";
        let error = zdc_parser::parse(src).unwrap_err();
        let out = render(src, "example.zd", &Diagnostic::from(error));
        let colour = out.matches('\u{1b}').count();
        assert!(colour > 0, "ariadne's own colours were stripped:\n{out}");
        assert!(
            !out.contains("\u{1b}[2J") && !out.contains("\u{1b}]0;"),
            "the program's escape sequences reached the terminal:\n{out:?}"
        );
    }

    /// #153. Piping a diagnostic into a file or a CI log should not embed
    /// escape sequences, and `NO_COLOR` is the convention for saying so.
    ///
    /// Asserted against the explicit parameter rather than the environment
    /// variable: the variable is process-global and these tests run in
    /// parallel, so a test that set it would be testing the scheduler.
    /// `render` reads the environment; `render_in_colour` is what it reads
    /// the environment *for*.
    #[test]
    fn a_diagnostic_rendered_without_colour_carries_no_escape_sequences() {
        let src = "state a is client Text starting
nope
";
        let error = zdc_parser::parse(src).unwrap_err();
        let diagnostic = Diagnostic::from(error);

        let plain = render_in_colour(src, "example.zd", &diagnostic, false);
        assert!(
            !plain.contains('\u{1b}'),
            "no escape sequence may survive with colour off:\n{plain:?}"
        );

        // The diagnostic still says everything it said — losing colour must
        // not lose the caret, the path or the message.
        assert!(
            plain.contains("example.zd"),
            "the path is still named:\n{plain}"
        );
        assert!(plain.contains('│'), "the caret is still drawn:\n{plain}");

        let coloured = render_in_colour(src, "example.zd", &diagnostic, true);
        assert!(
            coloured.contains('\u{1b}'),
            "colour is still the default when it is asked for:\n{coloured:?}"
        );
    }

    /// The substitution is byte for byte, so a caret still lands on the
    /// token the diagnostic is about. Stripping instead would slide every
    /// span after the first control character.
    ///
    /// The caret line used to be found by looking for the word `here`,
    /// which is what every caret in the compiler said. It now says what
    /// the site knew, so the line is found by the label this diagnostic
    /// supplies.
    #[test]
    fn replacing_a_control_character_does_not_move_the_caret() {
        let src = "# \u{1b}[31m comment\nstate a is client Whole starting nope\n";
        let offending = src.find("nope").expect("the token is in the source") as u32;
        let d = Diagnostic {
            message: "`nope` is not defined.".to_string(),
            span: Some(zdc_lexer::Span::new(offending, offending + 4)),
            label: Some("no declaration introduces this name".to_string()),
            notes: Vec::new(),
            help: None,
            suggestion: None,
            code: None,
            level: Level::Error,
        };
        let plain = strip_ansi(&render(src, "example.zd", &d));
        assert!(
            plain
                .lines()
                .any(|line| line.contains("no declaration introduces this name")),
            "the caret carries no label:\n{plain}"
        );
        assert!(
            plain.contains("state a is client Whole starting nope"),
            "the source line moved:\n{plain}"
        );
    }

    /// A site with nothing to add draws the underline and no words. The
    /// alternative considered and rejected was a generic phrase, which is
    /// what `here` already was.
    #[test]
    fn a_diagnostic_with_no_label_draws_an_underline_and_no_words() {
        let src = "state a is client Whole starting nope\n";
        let offending = src.find("nope").expect("the token is in the source") as u32;
        let d = Diagnostic {
            message: "`nope` is not defined.".to_string(),
            span: Some(zdc_lexer::Span::new(offending, offending + 4)),
            label: None,
            notes: Vec::new(),
            help: None,
            suggestion: None,
            code: None,
            level: Level::Error,
        };
        let plain = strip_ansi(&render(src, "example.zd", &d));

        let underline = plain
            .lines()
            .find(|line| line.contains('┬'))
            .expect("the span must still be underlined");
        assert_eq!(
            underline.chars().filter(|c| *c == '─').count(),
            3,
            "the underline must span `nope` and nothing else:\n{plain}"
        );
        assert!(
            !plain.contains("here"),
            "an unlabelled caret must say nothing at all:\n{plain}"
        );
        let arrow = plain
            .lines()
            .find(|line| line.contains('╰'))
            .expect("the arrow the underline is drawn as part of");
        assert_eq!(
            arrow.trim_end().chars().last(),
            Some('─'),
            "the arrow must end with no words after it:\n{plain}"
        );
    }

    /// The suggestion is the reader's own line with the edit made, spliced
    /// out of the source rather than assembled from prose the parser wrote.
    #[test]
    fn a_suggestion_renders_the_readers_line_with_the_edit_made() {
        let src = "state votes is Map of Id to Int starting empty\n";
        let at = src.find("Map").expect("the type is in the source") as u32;
        let d = Diagnostic {
            message: "no placement.".to_string(),
            span: Some(zdc_lexer::Span::new(at, at + 3)),
            label: None,
            notes: Vec::new(),
            help: None,
            suggestion: Some(Suggestion {
                span: zdc_lexer::Span::new(at, at),
                replacement: "client ".to_string(),
            }),
            code: None,
            level: Level::Error,
        };
        let plain = strip_ansi(&render(src, "example.zd", &d));

        assert!(
            plain.contains("state votes is client Map of Id to Int starting empty"),
            "the corrected line was not rendered:\n{plain}"
        );
    }

    /// A suggestion the renderer cannot place is dropped, because a
    /// diagnostic that panics is worse than one with no repair on it.
    #[test]
    fn a_suggestion_pointing_outside_the_source_is_dropped_rather_than_fatal() {
        let src = "state a is client Whole starting 1\n";
        let d = Diagnostic {
            message: "no placement.".to_string(),
            span: Some(zdc_lexer::Span::new(0, 5)),
            label: None,
            notes: Vec::new(),
            help: None,
            suggestion: Some(Suggestion {
                span: zdc_lexer::Span::new(9_000, 9_001),
                replacement: "client ".to_string(),
            }),
            code: None,
            level: Level::Error,
        };
        let plain = strip_ansi(&render(src, "example.zd", &d));

        assert!(
            !plain.contains("as it would be accepted"),
            "an unplaceable suggestion was printed anyway:\n{plain}"
        );
        assert!(
            plain.contains("no placement."),
            "the diagnostic itself must survive:\n{plain}"
        );
    }

    /// A diagnostic interpolates the program's own text into its message —
    /// `environment "…"` names its key — so the message needs the same
    /// treatment the snippet gets.
    #[test]
    fn a_message_quoting_the_program_cannot_write_escape_sequences_either() {
        let d = Diagnostic {
            message: "`\u{1b}[2J` is not defined.".to_string(),
            span: Some(zdc_lexer::Span::new(0, 5)),
            label: None,
            notes: Vec::new(),
            help: Some("Try \u{1b}]0;pwned\u{7}.".to_string()),
            suggestion: None,
            code: None,
            level: Level::Error,
        };
        let out = render("state votes", "example.zd", &d);
        assert!(
            !out.contains("\u{1b}[2J"),
            "message escapes leaked:\n{out:?}"
        );
        assert!(!out.contains("\u{1b}]0;"), "help escapes leaked:\n{out:?}");
    }

    #[test]
    fn help_text_is_included_when_present() {
        let d = Diagnostic {
            message: "Something went wrong.".to_string(),
            span: Some(zdc_lexer::Span::new(0, 5)),
            label: None,
            notes: Vec::new(),
            help: Some("Try writing `starting empty`.".to_string()),
            suggestion: None,
            code: None,
            level: Level::Error,
        };
        let out = render("state votes", "example.zd", &d);
        assert!(out.contains("Try writing"), "missing help:\n{out}");
    }

    /// §7.3 asks a rejected program to be shown *the path* along which a
    /// value would have escaped. One span cannot draw a path, so every
    /// note gets its own numbered label on the same report.
    #[test]
    fn every_note_is_rendered_as_its_own_numbered_label() {
        let src = "secret state key is server Text from environment \"K\"\nstate leak is client Text from key\n";
        let declared = src.find("key").expect("the declaration") as u32;
        let used = src.rfind("key").expect("the use") as u32;
        let d = Diagnostic {
            message: "`leak` is not declared secret.".to_string(),
            span: Some(Span::new(used, used + 3)),
            label: None,
            notes: vec![
                (Span::new(declared, declared + 3), "declared secret".into()),
                (Span::new(used, used + 3), "read here".into()),
            ],
            help: None,
            suggestion: None,
            code: None,
            level: Level::Error,
        };
        let plain = strip_ansi(&render(src, "leak.zd", &d));

        assert!(plain.contains("1. declared secret"), "{plain}");
        assert!(plain.contains("2. read here"), "{plain}");
    }

    #[test]
    fn spanned_diagnostics_still_render_the_source_snippet() {
        // Regression check: introducing the spanless case must not change
        // the normal (spanned) rendering path.
        let src = "state votes is Map of Id to Int starting empty";
        let error = zdc_parser::parse(src).unwrap_err();
        let out = render(src, "example.zd", &Diagnostic::from(error));
        let plain = strip_ansi(&out);
        assert!(
            plain.contains("Map"),
            "expected the offending source snippet to be quoted:\n{out}"
        );
        assert!(plain.contains('│'), "expected a source-line gutter:\n{out}");
    }

    /// Spans are byte offsets. A file with any character outside ASCII —
    /// an em dash in a comment is enough — must still put the caret under
    /// the token the diagnostic is about.
    #[test]
    fn a_caret_lands_correctly_in_a_file_containing_non_ascii() {
        let src = "# an em dash — right here\nstate a is client Whole starting nope\n";
        let offending = src.find("nope").expect("the token is in the source") as u32;
        let d = Diagnostic {
            message: "`nope` is not defined.".to_string(),
            span: Some(zdc_lexer::Span::new(offending, offending + 4)),
            // Labelled, because the joint the underline is measured from is
            // drawn only for a caret that has something to say.
            label: Some("no declaration introduces this name".to_string()),
            notes: Vec::new(),
            help: None,
            suggestion: None,
            code: None,
            level: Level::Error,
        };
        let plain = strip_ansi(&render(src, "example.zd", &d));

        let underline = plain
            .lines()
            .find(|line| line.contains('┬'))
            .expect("a caret line");
        let source = plain
            .lines()
            .find(|line| line.contains("starting nope"))
            .expect("the offending source line");

        // Read both columns off the same rendered text: they line up only
        // if the byte range was interpreted as bytes.
        let underline_at = underline
            .chars()
            .position(|c| c == '─')
            .expect("an underline");
        let token_at = source
            .char_indices()
            .position(|(at, _)| source[at..].starts_with("nope"))
            .expect("the token on its line");

        assert_eq!(
            underline_at, token_at,
            "the underline is under the wrong characters:\n{plain}"
        );
    }

    #[test]
    fn spanless_diagnostics_render_message_and_path_without_a_snippet() {
        let d = Diagnostic::file_error("Could not read nope.zd: No such file or directory");
        let out = render("", "nope.zd", &d);

        assert!(out.contains("nope.zd"), "missing path:\n{out}");
        assert!(
            out.contains("No such file or directory"),
            "missing the underlying cause:\n{out}"
        );
        assert!(
            !out.contains('┬'),
            "spanless diagnostics must not draw a caret:\n{out}"
        );
        assert!(
            !out.contains('│'),
            "spanless diagnostics must not draw a source-line gutter:\n{out}"
        );
    }

    #[test]
    fn rendering_a_spanless_diagnostic_does_not_panic() {
        // Regardless of what `src` is passed (it is irrelevant for a
        // file-level error), rendering must not panic.
        let d = Diagnostic::file_error("boom");
        let _ = render("anything, or nothing at all", "path.zd", &d);
        let _ = render("", "path.zd", &d);
    }
}
