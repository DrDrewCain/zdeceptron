//! The capabilities a build may ask the compiler for, and the sandbox
//! they are answered inside.
//!
//! **Why this exists at all.** §17.4.8 said build-time `foreign` works
//! "because the build host can import `marked`". It cannot: the build host
//! is `boa_engine`, embedded in the compiler, and it ships ECMAScript
//! builtins only — no `node:fs`, no npm resolution. The claim was true only
//! under the `node` approach §7 forbids, so the case §14C.3b assumed and
//! §14G.1.5 ratified — `state posts is static List of Post from readPosts
//! with directory is "content"` — had no mechanism.
//!
//! **Why capabilities rather than a module loader.** A *runtime* `foreign`
//! calls into a real host — a browser, a serverless runtime — which
//! genuinely has npm and a DOM, and §14E is right about it. A *build-time*
//! call has no host. The compiler is the host. So the honest construct is
//! not "import a module" but "ask the compiler for a capability", and four
//! properties follow that a loader could not offer:
//!
//! * `zdc` stays one binary. Nothing is fetched, resolved or installed.
//! * It is pure Rust under `#![forbid(unsafe_code)]`, so the memory-safety
//!   claim of §7.5 covers the build-time surface too.
//! * It is **sandboxable**, which matters because a build that can read
//!   arbitrary paths is a supply-chain surface. Every path below is
//!   resolved and then required to be inside the project directory.
//! * It is **deterministic**, which §17.4.7 already argues a build must be.
//!   Directory order is the obvious hazard and it is normalised here.
//!
//! **And the cost, stated plainly: the set is closed.** It grows only when
//! the compiler is released. What bounds that cost is that growing it
//! spends no keyword — the capability name is an identifier inside the
//! `build` production, so the fourth capability, `build parts`, cost a
//! match arm and no word out of §14G.7.7's budget.

use std::path::{Path, PathBuf};

use zdc_hir::BuildCapability;
use zdc_runtime::{Ask, Capability, Provided, ProvidedPart};

/// The prefix a refusal carries out through the JavaScript engine.
///
/// A capability reports failure by throwing, and the engine's message is
/// the only channel back. Marking it lets [`crate::evaluate()`] tell a
/// refused path from a program that threw, which are different mistakes
/// and want different diagnostics.
pub const REFUSED: &str = "E11: ";

/// Every capability, in the order [`BuildCapability::ALL`] lists them.
pub fn all() -> Vec<Capability> {
    BuildCapability::ALL
        .into_iter()
        .map(|capability| Capability {
            name: capability.name(),
            answer: match capability {
                BuildCapability::Read => read,
                BuildCapability::List => list,
                BuildCapability::Markdown => markdown,
                BuildCapability::Parts => parts,
            },
        })
        .collect()
}

/// `build read path` — one file's contents.
fn read(ask: Ask<'_>) -> Result<Provided, String> {
    let (root, path) = (ask.root, ask.argument);
    let resolved = resolve(root, path)?;
    if !resolved.is_file() {
        return Err(refusal(format!(
            "`{path}` is not a file in the project directory"
        )));
    }
    match std::fs::read(&resolved) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Ok(Provided::Text(text)),
            Err(_) => Err(refusal(format!(
                "`{path}` is not UTF-8, and `Text` is. A build reads text, not bytes"
            ))),
        },
        Err(error) => Err(refusal(format!("`{path}` could not be read: {error}"))),
    }
}

/// `build list directory` — the files directly inside a directory.
///
/// Three normalisations, each of which would otherwise make a build depend
/// on something that is not the program:
///
/// 1. **Sorted by byte order.** `read_dir` yields whatever the filesystem
///    yields, which differs between machines and between two runs on one
///    machine. A build that inlines a list must inline the same list.
/// 2. **Files only.** A subdirectory is not something `build read` could
///    consume, so including one would put a value in the list that no
///    other capability accepts.
/// 3. **Relative to the project directory**, so what comes out of `list`
///    goes straight into `read` without the program doing path arithmetic
///    the sandbox would then have to re-check.
fn list(ask: Ask<'_>) -> Result<Provided, String> {
    let (root, path) = (ask.root, ask.argument);
    let resolved = resolve(root, path)?;
    if !resolved.is_dir() {
        return Err(refusal(format!(
            "`{path}` is not a directory in the project directory"
        )));
    }

    let entries = std::fs::read_dir(&resolved)
        .map_err(|error| refusal(format!("`{path}` could not be listed: {error}")))?;

    let mut found = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| refusal(format!("`{path}` could not be listed: {error}")))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(refusal(format!(
                "`{path}` holds a name that is not UTF-8, so it has no `Text` to list it as"
            )));
        };
        let relative = join(path, name);
        // Resolved, not written: an entry may be a symlink, and a symlink
        // out of the project is exactly what this refuses. Refused rather
        // than skipped, because a silently shorter list is a build that
        // succeeded while doing something other than what it said.
        let target = resolve(root, &relative)?;
        if target.is_file() {
            found.push(relative);
        }
    }
    found.sort();
    Ok(Provided::List(found))
}

/// `build markdown source` — CommonMark, rendered to HTML, with every
/// script-bearing construct neutralised.
///
/// One extension is enabled, and every other is not. Tables, strikethrough
/// and the rest are each a decision about what the language's markdown
/// *is*, and defaulting them on would make the answer depend on a crate's
/// idea of "common" rather than on a specification.
///
/// # Footnotes, and why they are the one
///
/// #61 asked for footnote markers in a post, produced by the renderer
/// rather than written by hand. Without the extension `[^why]` is not a
/// footnote at all: CommonMark reads it as a link whose text is `^why`
/// and whose destination is the definition line's text, which is a
/// working link to nowhere rather than a construct the renderer declined.
/// That is worse than refusing it, because nothing says so.
///
/// The extension emits a `sup` holding an anchor to the note and a
/// numbered list of the notes at the end, so the marker is reachable from
/// the keyboard and announced as a reference rather than being a small
/// raised number. The destination it writes is a fragment of the same
/// document, which is relative, so it passes the scheme filter below
/// unchanged and adds no new URL surface.
///
/// The other extensions stay off for the reason above and are not
/// weakened by this one: each is its own decision, and this is the one
/// that had an issue behind it.
///
/// # [`neutralise`] is the trusted base
///
/// `Markup` is the one type the renderer parses as HTML, and every
/// `Markup` in the language comes out of one function: [`neutralise`],
/// which this and [`parts`] both go through. It used to be this function's
/// own body and was lifted out when `build parts` arrived, precisely so
/// that there would still be *one* of it — a second renderer with its own
/// copy of the two rules below is a second renderer that can lose one.
/// Everything the type guarantees is guaranteed there or nowhere, so what
/// the renderer does with its input was measured rather than assumed.
/// Verbatim, from `pulldown-cmark`
/// 0.13 with `features = ["html"]`:
///
/// ```text
/// "# Hi\n\n<script>alert(1)</script>\n"      -> "<h1>Hi</h1>\n<script>alert(1)</script>\n"
/// "Inline <img src=x onerror=alert(1)> here." -> "<p>Inline <img src=x onerror=alert(1)> here.</p>\n"
/// "<div onclick=\"alert(1)\">x</div>\n"        -> "<div onclick=\"alert(1)\">x</div>\n"
/// "[click](javascript:alert(1))\n"            -> "<p><a href=\"javascript:alert(1)\">click</a></p>\n"
/// ```
///
/// All four execute. **The fourth is the one that matters**, and it is the
/// reason this is a rewriting pass rather than a flag: it contains no raw
/// HTML at all. `[click](javascript:…)` is ordinary CommonMark link
/// syntax, so an option that disabled raw HTML — had one existed — would
/// have left it untouched. Any treatment of markdown that stops at "turn
/// off inline HTML" ships this hole.
///
/// So two rewrites, over the event stream rather than over the output
/// string. Rewriting events is what makes this checkable: HTML is
/// generated only by [`render`] from events [`neutralise`] has already
/// approved, and there is no pass that parses generated HTML back.
///
/// 1. **Raw HTML becomes text.** [`pulldown_cmark::Event::Html`] and
///    [`pulldown_cmark::Event::InlineHtml`] are re-emitted as
///    [`pulldown_cmark::Event::Text`], which `push_html` escapes. A
///    `<script>` in a post is *shown* — the reader sees the tag — which is
///    the honest rendering of a file that a Markdown author wrote by hand
///    and the compiler has no reason to trust (§18.1: content read at
///    build time is content the author did not necessarily write).
/// 2. **Link and image destinations are scheme-checked.** Only relative
///    URLs and the schemes in [`zdc_hir::URL_SCHEMES`] survive; anything
///    else — `javascript:`, `data:`, `vbscript:` — is replaced wholesale. The
///    link still renders and still says what it said; it simply goes
///    nowhere.
///
/// What this does **not** claim: it is not a general HTML sanitiser,
/// because it never has to be. It is a whitelist over a generator whose
/// output shape is fixed by CommonMark, which is a far smaller problem
/// than sanitising arbitrary HTML.
fn markdown(ask: Ask<'_>) -> Result<Provided, String> {
    Ok(Provided::Markup(render(
        events(ask.argument).map(neutralise),
    )))
}

/// The event stream of one document, under the one set of options.
///
/// Named rather than written twice: `build markdown` and `build parts`
/// must agree about what this language's markdown *is*, and two call sites
/// each passing their own `Options` is exactly how they would stop
/// agreeing.
///
/// **GitHub-flavoured CommonMark, not bare CommonMark.** Footnotes alone
/// was the whole option set, and the gap showed the moment a real post was
/// rendered: a table came out as pipes, `~~a~~` as tildes, a task list as
/// brackets. Every one of those is a thing people write in markdown and
/// expect, and the portfolio this was tested against reaches them through
/// `remark-gfm` — so a document that renders there and not here is the
/// language's problem and not the author's. `ENABLE_GFM` would
/// additionally turn on the admonition blocks GitHub added; the four here
/// are the ones `remark-gfm` itself provides, and matching it exactly is
/// the point.
fn events(source: &str) -> pulldown_cmark::Parser<'_> {
    use pulldown_cmark::Options;

    pulldown_cmark::Parser::new_ext(
        source,
        Options::ENABLE_FOOTNOTES
            | Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS,
    )
}

/// The two rewrites, as one function over one event — the trusted base.
///
/// Extracted from `markdown` when `build parts` arrived, because a second
/// renderer with its own copy of these two rules is a second renderer that
/// can lose one. Everything the module doc above claims is claimed about
/// this function, and both capabilities go through it.
fn neutralise(event: pulldown_cmark::Event<'_>) -> pulldown_cmark::Event<'_> {
    use pulldown_cmark::{Event, Tag};

    match event {
        // Rewrite 1. `push_html` escapes `Event::Text`, so the tag becomes
        // visible characters rather than an element.
        Event::Html(raw) => Event::Text(raw),
        Event::InlineHtml(raw) => Event::Text(raw),
        // Rewrite 2, on the two tags that carry a URL the browser acts on.
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: safe_url(dest_url),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: safe_url(dest_url),
            title,
            id,
        }),
        other => other,
    }
}

/// Approved events to HTML. The only call to `push_html` in the compiler.
fn render<'a>(events: impl Iterator<Item = pulldown_cmark::Event<'a>>) -> String {
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events);
    html
}

/// The info-string word this compiler owns — issue #305.
///
/// A fence whose info string begins with this word is a widget the
/// document is naming; every other fence in the language's markdown is a
/// code block and is rendered as one. It is the language's own name, which
/// is what makes the collision cost as close to nothing as a reserved word
/// gets: a `.md` file that wanted to show ZDeceptron source was already
/// going to write ```` ```zd ````, and that fence now needs a widget name
/// after it or the build says so.
const WIDGET_FENCE: &str = "zd";

/// `build parts source` — one document, split into prose runs and the
/// widgets it names (issue #305).
///
/// # Why a list, and not children on `Prose`
///
/// `Prose` renders one `Markup` and has no children, because interleaving
/// parsed nodes with templated ones would make the sibling offsets every
/// binding is scheduled against depend on how many nodes a *file* parsed
/// into, which is not known at compile time. A list sidesteps that rather
/// than weakening it: each part is its own node under an ordinary `each`,
/// so no parsed subtree ever shares a parent with a templated one.
///
/// # The prose is rendered by the same pass
///
/// Every run goes through [`neutralise`] and [`render`], so a `<script>`
/// in a post is shown rather than run here exactly as it is under `build
/// markdown`, and a `javascript:` destination goes nowhere here too. The
/// splitting adds no new way for a file to reach the DOM: it adds a way
/// for the *program* to put its own component between two runs of prose.
///
/// # The widget name is untrusted input, and is treated as such
///
/// It comes out of a content file, which §18.1 says is content the author
/// did not necessarily write. Two checks, in order, and both are refusals
/// rather than repairs:
///
/// 1. **Shape.** A widget name is a declaration name in this language, so
///    it must read as one. Anything else is refused rather than escaped,
///    because a name that is not a name has no correct rendering.
/// 2. **Membership.** The name must be one the program declares in its
///    `choice Widget`. This is the closed set, and it is why a post naming
///    a widget the program does not offer is a failed build rather than a
///    blank space — a stronger bargain than MDX makes, where an `import`
///    inside a content file can reach anything on disk.
///
/// # What a footnote does across a split
///
/// A run is rendered on its own, so a footnote's marker and its definition
/// have to be in the same run — a definition after a widget fence numbers
/// from one again. That is a real limitation and it is stated rather than
/// worked around: the alternative is rendering the whole document once and
/// cutting the HTML afterwards, which is parsing generated HTML back, and
/// the module doc above is the argument against ever doing that.
fn parts(ask: Ask<'_>) -> Result<Provided, String> {
    use pulldown_cmark::{CodeBlockKind, Event, Tag, TagEnd};

    let mut found: Vec<ProvidedPart> = Vec::new();
    let mut run: Vec<Event<'_>> = Vec::new();
    // The widget name of the fence being read, and what it has said so
    // far. `None` between fences, which is where prose accumulates.
    let mut naming: Option<(String, String)> = None;

    for event in events(ask.argument) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(ref info)))
                if is_widget_fence(info) =>
            {
                let widget = widget_named(info, ask.widgets)?;
                if let Some(prose) = prose_part(std::mem::take(&mut run)) {
                    found.push(prose);
                }
                naming = Some((widget, String::new()));
            }
            Event::Text(ref text) if naming.is_some() => {
                if let Some((_, argument)) = naming.as_mut() {
                    argument.push_str(text);
                }
            }
            Event::End(TagEnd::CodeBlock) if naming.is_some() => {
                let (widget, argument) = naming.take().expect("inside a widget fence");
                found.push(ProvidedPart {
                    markup: String::new(),
                    widget,
                    argument,
                });
            }
            // A widget fence holds text and nothing else, so anything else
            // arriving inside one is dropped rather than smuggled into the
            // argument. Outside one, this is the whole of the prose.
            other => {
                if naming.is_none() {
                    run.push(neutralise(other));
                }
            }
        }
    }
    if let Some(prose) = prose_part(run) {
        found.push(prose);
    }
    Ok(Provided::Parts(found))
}

/// Whether a fence's info string is one this compiler owns.
///
/// The first whitespace-separated word, exactly: ```` ```zdx ```` is not
/// this fence and ```` ```rust ```` never was.
fn is_widget_fence(info: &str) -> bool {
    info.split_whitespace().next() == Some(WIDGET_FENCE)
}

/// The widget a fence names, or the refusal that says why it names none.
///
/// Every failure here stops the build. A document that names a widget is
/// asking for something, and rendering nothing where it asked would be a
/// page that is silently missing its chart — which is the outcome this
/// whole design exists to rule out.
fn widget_named(info: &str, offered: &[String]) -> Result<String, String> {
    let mut words = info.split_whitespace();
    words.next();
    let Some(widget) = words.next() else {
        return Err(refusal(format!(
            "a `{WIDGET_FENCE}` fence names no widget. Write the widget's name after the \
             language, as in ```{WIDGET_FENCE} {}```",
            offered.first().map(String::as_str).unwrap_or("RingChart")
        )));
    };
    if let Some(extra) = words.next() {
        return Err(refusal(format!(
            "the fence naming the widget `{widget}` carries `{extra}` after it, and a widget \
             fence names one widget. What the widget is given goes inside the fence, not on the \
             line that opens it"
        )));
    }
    if !is_widget_name(widget) {
        return Err(refusal(format!(
            "`{}` is not a widget name. A widget is named the way a component is — a capital \
             letter and then letters and digits — and a name that is not one names nothing the \
             program could have declared",
            shown(widget)
        )));
    }
    if offered.is_empty() {
        return Err(refusal(format!(
            "this document names the widget `{widget}`, and this program offers none: it \
             declares no `choice {}`. The set of widgets a document may name is the program's to \
             declare, which is what stops a file reaching for something the program never wrote",
            zdc_hir::WIDGET_CHOICE
        )));
    }
    if !offered.iter().any(|name| name == widget) {
        return Err(refusal(format!(
            "this document names the widget `{widget}`, and this program does not offer it. \
             `choice {}` offers `{}`",
            zdc_hir::WIDGET_CHOICE,
            offered.join("`, `")
        )));
    }
    Ok(widget.to_string())
}

/// Whether a name reads as a declaration name in this language.
///
/// ASCII, because that is what a `component` name is: this is the same
/// shape the lexer accepts for an upper-case identifier, and a widget name
/// that could not be a component name could never have matched one.
fn is_widget_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

/// An untrusted name, bounded, for a diagnostic that has to quote it.
///
/// A refusal names what it refused — a message that says only "that is not
/// a name" is one the author cannot act on. What it must not do is print
/// an unbounded run of a content file into a terminal, so this takes the
/// first line and a bounded prefix of it.
fn shown(name: &str) -> String {
    const LIMIT: usize = 40;
    let line = name.lines().next().unwrap_or_default();
    match line.char_indices().nth(LIMIT) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_string(),
    }
}

/// One run of prose, or `None` if the run rendered to nothing.
///
/// Two widget fences with a blank line between them are two widgets and no
/// prose. An empty `Prose` between them would be an empty `div` in the
/// document, which is a thing a reader can select and a stylesheet can put
/// a margin under, so it is not emitted at all.
fn prose_part(run: Vec<pulldown_cmark::Event<'_>>) -> Option<ProvidedPart> {
    let markup = render(run.into_iter());
    if markup.trim().is_empty() {
        return None;
    }
    Some(ProvidedPart {
        markup,
        widget: String::new(),
        argument: String::new(),
    })
}

/// What a link's destination becomes when it is not one this renders.
const REFUSED_URL: &str = "about:blank#blocked";

/// A destination the browser may be given, or [`REFUSED_URL`].
///
/// Which schemes those are is [`zdc_hir::URL_SCHEMES`], the same set
/// the element table and `runtime/dom.js` read. This half used to hold a
/// list of its own, and the two had drifted two entries apart — not a hole,
/// because neither admitted a scripting scheme, but exactly how one appears
/// when someone adds a scheme to whichever list they happen to be reading.
///
/// The **tokenising** is this half's own, and stays: a destination reaches
/// here as raw markdown text, so ASCII whitespace and control characters
/// are stripped from the whole of it before the scheme is read. A tab
/// inside `java…script:` is how this check is usually got around, and a
/// value that arrived through an attribute cannot carry one.
fn safe_url(url: pulldown_cmark::CowStr<'_>) -> pulldown_cmark::CowStr<'static> {
    let stripped: String = url
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
        .collect();

    if zdc_hir::url_is_safe(&stripped) {
        return pulldown_cmark::CowStr::from(url.into_string());
    }
    pulldown_cmark::CowStr::Borrowed(REFUSED_URL)
}

/// Resolve a path against the project directory, or refuse it.
///
/// **The containment decision is [`zdc_hir::sandbox::refuse`], not a copy
/// of it.** That rule already bounds the other path a program can make the
/// build open — a `use` specifier — and it was written for both: its own
/// module doc names `build read` and `build list` as callers before either
/// existed. Routing here rather than re-deriving the check is what keeps
/// the two from drifting.
///
/// It replaced [`zdc_graph::unusable_path`] at this call site, which is a
/// strengthening in one direction and a loosening in another, both
/// deliberate. `unusable_path` is a **string** rule: it refuses every `..`
/// on sight and cannot see a symbolic link at all, which was harmless
/// while the evaluator had no filesystem and is not harmless now that it
/// has one. `sandbox::refuse` decides containment on the *canonical* path,
/// so a link planted inside the project that points outside it is caught —
/// and a `..` that lands back inside the project is allowed, because where
/// it lands is the question. `unusable_path` keeps its own call site:
/// E0316 is about a path written *into the bundle*, which names no file on
/// the build host and is a different question.
fn resolve(root: &Path, path: &str) -> Result<PathBuf, String> {
    let canonical_root = root.canonicalize().map_err(|error| {
        refusal(format!(
            "the project directory `{}` could not be resolved: {error}",
            root.display()
        ))
    })?;

    let target = canonical_root.join(path);
    if let Some(reason) = zdc_hir::sandbox::refuse(&canonical_root, path, &target) {
        return Err(refusal(format!("`{path}` {}", reason.reason())));
    }

    // Only now, and only for its side effect of resolving links for the
    // caller: containment was decided above, on the canonical path, so
    // this cannot be the check.
    target
        .canonicalize()
        .map_err(|error| refusal(format!("`{path}` is not in the project directory: {error}")))
}

/// Join a directory and a name the way the language spells a path.
///
/// `/` always, never the host separator: the path a build inlines is part
/// of the program's value, and a value that differs between Windows and
/// Linux is not a build-time constant.
fn join(directory: &str, name: &str) -> String {
    let trimmed = directory.trim_end_matches('/');
    if trimmed.is_empty() {
        return name.to_string();
    }
    format!("{trimmed}/{name}")
}

fn refusal(message: String) -> String {
    format!("{REFUSED}{message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
    }

    /// One capability's question, with no widget declared.
    ///
    /// The widget set is the only thing an [`Ask`] carries that is not the
    /// project directory, and every test below except the widget ones is
    /// about a program that declares none — which is every program written
    /// before issue #305.
    fn ask<'a>(root: &'a Path, argument: &'a str) -> Ask<'a> {
        Ask {
            root,
            widgets: &[],
            argument,
        }
    }

    /// The same question, from a program that offers these widgets.
    fn offering<'a>(argument: &'a str, widgets: &'a [String]) -> Ask<'a> {
        Ask {
            root: Path::new("."),
            widgets,
            argument,
        }
    }

    #[test]
    fn a_climbing_path_is_refused_before_it_is_opened() {
        let refused = read(ask(&project(), "../Cargo.toml")).expect_err("must refuse");
        assert!(refused.starts_with(REFUSED), "{refused}");
        assert!(refused.contains("climbs out of the project"), "{refused}");
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let refused = read(ask(&project(), "/etc/hosts")).expect_err("must refuse");
        assert!(refused.contains("is an absolute path"), "{refused}");
    }

    /// **A symbolic link planted inside the project, pointing out of it.**
    ///
    /// This is the case no amount of reading the written path can catch:
    /// `secrets.md` has no `..`, no leading `/` and no scheme, and reads as
    /// an ordinary sibling file. It is caught because containment is decided
    /// on the canonical path, which is the half of
    /// [`zdc_hir::sandbox::refuse`] that does the work — and it is the half
    /// `zdc_graph::unusable_path`, which this call site used to consult,
    /// does not have at all.
    ///
    /// Both capabilities are checked. `list` matters as much as `read`: a
    /// listing that quietly skipped the link would be a build that
    /// succeeded while doing something other than what it said, and one
    /// that returned it would hand the path straight back to `read`.
    #[cfg(unix)]
    #[test]
    fn a_planted_symlink_out_of_the_project_is_refused_by_read_and_by_list() {
        let root =
            std::env::temp_dir().join(format!("zdc-sandbox-{}-{}", std::process::id(), line!()));
        let posts = root.join("posts");
        std::fs::create_dir_all(&posts).expect("a project directory");
        std::fs::write(posts.join("ordinary.md"), "# fine\n").expect("an ordinary post");

        // The secret is outside the project, and the link is inside it.
        let outside = root.with_extension("outside");
        std::fs::write(&outside, "the private key\n").expect("a file outside the project");
        std::os::unix::fs::symlink(&outside, posts.join("secrets.md")).expect("plants the link");

        let read_refusal = read(ask(&root, "posts/secrets.md")).expect_err("`read` must refuse");
        let list_refusal = list(ask(&root, "posts")).expect_err("`list` must refuse");

        // Cleaned up before the assertions, so a failure does not leave the
        // link behind for the next run to trip over.
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);

        assert!(
            read_refusal.contains("points outside the project"),
            "a planted link was followed out of the project: {read_refusal}"
        );
        assert!(
            list_refusal.contains("points outside the project"),
            "a listing walked past a planted link: {list_refusal}"
        );
    }

    #[test]
    fn a_listing_is_sorted_and_relative() {
        let Provided::List(found) = list(ask(&project(), "content")).expect("lists") else {
            panic!("`list` must give a list");
        };
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(found, sorted, "a listing must not depend on the filesystem");
        assert!(
            found.iter().all(|path| path.starts_with("content/")),
            "{found:?}"
        );
    }

    fn rendered(source: &str) -> String {
        let Provided::Markup(html) = markdown(ask(&project(), source)).expect("renders") else {
            panic!("`markdown` must give markup");
        };
        html
    }

    #[test]
    fn markdown_is_commonmark() {
        assert_eq!(
            rendered("# Title\n\ntext\n"),
            "<h1>Title</h1>\n<p>text</p>\n"
        );
    }

    /// The four constructs measured against `pulldown-cmark` 0.13 before
    /// this pass was written. Every one of them executed; none does now.
    ///
    /// These assert on the *absence of the mechanism* — no `<script>`
    /// element, no `on…=` attribute, no `javascript:` destination — rather
    /// than on an exact string, so a rendering change that reintroduced
    /// any of them fails here even if the surrounding markup moved.
    #[test]
    fn a_script_block_in_a_post_is_shown_rather_than_run() {
        let html = rendered("# Hi\n\n<script>alert(1)</script>\n");
        assert!(
            !html.contains("<script"),
            "a raw script block survived: {html}"
        );
        // Shown, not silently dropped: a reader looking at the page can
        // see what the file said.
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn an_inline_event_handler_is_shown_rather_than_attached() {
        for source in [
            "Inline <img src=x onerror=alert(1)> here.\n",
            "<div onclick=\"alert(1)\">x</div>\n",
        ] {
            let html = rendered(source);
            // The characters `onerror=` still appear — escaped, inside a
            // text node, which is the point. What must not appear is a
            // tag: an attribute only exists on an element.
            assert!(!html.contains("<img"), "{html}");
            assert!(!html.contains("<div"), "{html}");
            assert!(html.contains("&lt;"), "the raw tag must be shown: {html}");
        }
    }

    /// The vector that makes this a rewriting pass rather than a flag: it
    /// is ordinary CommonMark link syntax with no raw HTML in it, so
    /// disabling inline HTML would not have touched it.
    #[test]
    fn a_javascript_destination_is_refused_while_the_link_still_renders() {
        for source in [
            "[click](javascript:alert(1))\n",
            // Case, whitespace and control characters are the three usual
            // ways round a scheme check.
            "[click](JaVaScRiPt:alert(1))\n",
            "[click](java\tscript:alert(1))\n",
            "[click](data:text/html,<script>alert(1)</script>)\n",
            "![x](javascript:alert(1))\n",
        ] {
            let html = rendered(source);
            // The property is about the two attributes a browser acts on.
            // `java\tscript:` is not parsed as a link at all, so it has no
            // attribute and simply renders as the text it is; asserting on
            // `REFUSED_URL` would have demanded it be a link first.
            let lowered = html.to_lowercase();
            for attribute in ["href=\"", "src=\""] {
                let mut rest = lowered.as_str();
                while let Some(at) = rest.find(attribute) {
                    let value = &rest[at + attribute.len()..];
                    let value = &value[..value.find('"').unwrap_or(value.len())];
                    assert!(
                        !value.starts_with("javascript:")
                            && !value.starts_with("data:")
                            && !value.starts_with("vbscript:"),
                        "a script destination survived {source:?}: {html}"
                    );
                    rest = &rest[at + attribute.len()..];
                }
            }
        }
    }

    /// The markdown half reads the one scheme set rather than a list of
    /// its own.
    ///
    /// Derived from [`zdc_hir::URL_SCHEMES`] rather than restating it,
    /// so a scheme added there is required to survive a rendered link
    /// without anybody remembering to add a case here — and a scheme
    /// removed there is required to stop surviving one.
    #[test]
    fn a_rendered_link_admits_exactly_the_schemes_the_one_set_names() {
        for scheme in zdc_hir::URL_SCHEMES.iter().copied().chain([
            "ftp",
            "file",
            "blob",
            "javascript",
            "data",
            "vbscript",
        ]) {
            let destination = format!("{scheme}:rest");
            let html = rendered(&format!("[a]({destination})\n"));
            // Built by concatenation rather than by interpolating around
            // the quotes: `check-emitted-strings.sh` reads this file as an
            // emitter source and cannot tell an expectation from an
            // emission, and the rule it enforces is worth more than the
            // convenience of writing the needle inline.
            let kept = html.contains(&["href=", "\"", &destination, "\""].concat());
            assert_eq!(
                kept,
                zdc_hir::URL_SCHEMES.contains(&scheme),
                "`{scheme}` must survive a rendered link exactly when it is one of the \
                 permitted schemes, and this rendered {html}"
            );
            if !kept {
                assert!(
                    html.contains(REFUSED_URL),
                    "a refused destination must still be a link: {html}"
                );
            }
        }
    }

    #[test]
    fn the_links_a_repository_actually_has_still_work() {
        for (source, expected) in [
            ("[a](./notes/b.md)\n", "./notes/b.md"),
            ("[a](/about)\n", "/about"),
            ("[a](#section)\n", "#section"),
            ("[a](https://example.com/x)\n", "https://example.com/x"),
            ("[a](mailto:x@example.com)\n", "mailto:x@example.com"),
            // A colon after a slash is a path, not a scheme.
            ("[a](notes/a:b.md)\n", "notes/a:b.md"),
        ] {
            let html = rendered(source);
            // `{expected:?}` writes the quotes, so this assertion holds no
            // quote of its own next to an interpolation — which is the
            // shape `scripts/check-emitted-strings.sh` refuses, and it
            // refuses it in every emitter source rather than trying to tell
            // a test's needle from a real emission.
            let needle = format!("href={expected:?}");
            assert!(
                html.contains(&needle),
                "{source:?} should keep its destination, got {html}"
            );
        }
    }

    // --- `build parts` (issue #305) ---------------------------------------

    fn offered() -> Vec<String> {
        vec!["RingChart".to_string(), "StackBars".to_string()]
    }

    fn split(source: &str) -> Vec<ProvidedPart> {
        let widgets = offered();
        let Provided::Parts(found) = parts(offering(source, &widgets)).expect("splits") else {
            panic!("`parts` must give parts");
        };
        found
    }

    fn refused_split(source: &str) -> String {
        let widgets = offered();
        parts(offering(source, &widgets)).expect_err("must refuse")
    }

    /// A document with no widget in it is one part, and that part is
    /// exactly what `build markdown` would have given for the same text.
    ///
    /// Asserted against the other capability rather than against a string,
    /// so the two cannot drift: a rendering change that moved the markup
    /// has to move both.
    #[test]
    fn a_document_naming_no_widget_renders_exactly_as_markdown_does() {
        let source = "# Title\n\ntext, and a [link](/about).\n";
        let found = split(source);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].widget, "");
        assert_eq!(found[0].argument, "");
        assert_eq!(found[0].markup, rendered(source));
    }

    /// The whole feature, in one document: prose, a widget, prose.
    #[test]
    fn a_widget_fence_cuts_the_document_and_keeps_its_body_verbatim() {
        let found = split("before\n\n```zd RingChart\nslug: wars\n```\n\nafter\n");
        assert_eq!(found.len(), 3, "{found:?}");

        assert_eq!(found[0].markup, "<p>before</p>\n");
        assert_eq!(found[0].widget, "");

        // The widget part carries no markup at all: it is a node the
        // *program* renders, and nothing parsed reaches the page through
        // it.
        assert_eq!(found[1].markup, "");
        assert_eq!(found[1].widget, "RingChart");
        assert_eq!(found[1].argument, "slug: wars\n");

        assert_eq!(found[2].markup, "<p>after</p>\n");
        assert_eq!(found[2].widget, "");
    }

    /// Every other fence is a code block, which is what it was before this
    /// capability existed and has to stay.
    #[test]
    fn an_ordinary_fence_is_still_a_code_block() {
        let found = split("```js\nalert(1)\n```\n");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].widget, "");
        assert!(found[0].markup.contains("<code"), "{:?}", found[0].markup);
        // Shown, escaped, inside the block — not run.
        assert!(
            found[0].markup.contains("alert(1)"),
            "{:?}",
            found[0].markup
        );
    }

    /// Two widgets with nothing between them are two parts and not three:
    /// an empty `Prose` is an empty `div` a reader can select and a
    /// stylesheet can put a margin under.
    #[test]
    fn a_run_of_prose_that_rendered_to_nothing_is_not_a_part() {
        let found = split("```zd RingChart\n```\n\n```zd StackBars\n```\n");
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].widget, "RingChart");
        assert_eq!(found[1].widget, "StackBars");
    }

    /// A run of prose goes through the same rewriting pass a whole post
    /// does, so splitting a document opens no new path to the DOM.
    ///
    /// This is the property that would silently be lost by rendering the
    /// document once and cutting the HTML afterwards, which is why the
    /// split is over events.
    #[test]
    fn a_prose_run_is_neutralised_exactly_as_a_whole_post_is() {
        let found = split(
            "<script>alert(1)</script>\n\n```zd RingChart\n```\n\n[click](javascript:alert(1))\n",
        );
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(
            !found[0].markup.contains("<script"),
            "{:?}",
            found[0].markup
        );
        assert!(found[0].markup.contains("&lt;script&gt;"), "{found:?}");
        assert!(
            !found[2].markup.to_lowercase().contains("javascript:"),
            "{:?}",
            found[2].markup
        );
        assert!(
            found[2].markup.contains(REFUSED_URL),
            "{:?}",
            found[2].markup
        );
    }

    /// **The closed set, enforced.** A document naming a widget this
    /// program does not offer stops the build, and the refusal says which
    /// widget and which names it could have written instead.
    #[test]
    fn a_widget_the_program_does_not_offer_is_refused_by_name() {
        let refused = refused_split("```zd PieChart\n```\n");
        assert!(refused.starts_with(REFUSED), "{refused}");
        assert!(refused.contains("PieChart"), "{refused}");
        assert!(refused.contains("RingChart`, `StackBars"), "{refused}");
    }

    /// A program that declares no widgets offers none, and says so rather
    /// than reporting an empty list of alternatives.
    #[test]
    fn a_program_declaring_no_widgets_says_that_rather_than_listing_none() {
        let refused = parts(offering("```zd RingChart\n```\n", &[])).expect_err("must refuse");
        assert!(refused.contains("declares no `choice Widget`"), "{refused}");
    }

    /// **A widget name is untrusted input.** It arrives from a file the
    /// author did not necessarily write, and it ends up inlined into the
    /// bundle as a value and quoted into diagnostics. A name that is not a
    /// declaration name is refused rather than escaped, which is what
    /// keeps every one of those paths trivially safe.
    #[test]
    fn a_name_that_is_not_a_declaration_name_is_refused() {
        for name in [
            "ring-chart",
            "ringChart",
            "\"+alert(1)+\"",
            "../../etc/passwd",
            "Ring.Chart",
            "</script><script>",
        ] {
            let refused = refused_split(&format!("```zd {name}\n```\n"));
            assert!(
                refused.contains("is not a widget name"),
                "`{name}` was not refused as a name: {refused}"
            );
        }
        // A line separator is whitespace to Unicode but not to
        // `str::lines`, so it splits the info string into two words rather
        // than making one bad name. Refused either way, which is the
        // property — the message is the other check's.
        let refused = refused_split("```zd Ring\u{2028}Chart\n```\n");
        assert!(refused.starts_with(REFUSED), "{refused}");
    }

    /// A refusal quotes what it refused, because an author cannot act on
    /// "that is not a name" — but it quotes a bounded prefix of one line,
    /// because the source is a content file.
    #[test]
    fn a_refusal_quotes_a_bounded_prefix_of_the_name_it_refused() {
        let long = "x".repeat(400);
        let refused = refused_split(&format!("```zd {long}\n```\n"));
        assert!(refused.contains('…'), "{refused}");
        assert!(refused.len() < 400, "the whole name reached the terminal");
    }

    /// A fence that opens the syntax and then says nothing is a mistake
    /// worth naming, not a part with an empty widget.
    #[test]
    fn a_widget_fence_naming_nothing_is_refused() {
        let refused = refused_split("```zd\n```\n");
        assert!(refused.contains("names no widget"), "{refused}");
        // The suggestion is a name the program actually offers.
        assert!(refused.contains("RingChart"), "{refused}");
    }

    /// The info line names one widget. Anything the file wanted to pass it
    /// goes inside the fence, which is where a reader looks for it.
    #[test]
    fn a_fence_carrying_more_than_a_name_is_refused() {
        let refused = refused_split("```zd RingChart wide\n```\n");
        assert!(refused.contains("names one widget"), "{refused}");
    }

    /// The owned word is exactly `zd`. A fence that merely starts with
    /// those two letters is somebody else's language.
    #[test]
    fn only_the_owned_word_takes_a_fence() {
        assert!(is_widget_fence("zd RingChart"));
        assert!(!is_widget_fence("zdx RingChart"));
        assert!(!is_widget_fence("rust"));
        assert!(!is_widget_fence(""));
    }
}
