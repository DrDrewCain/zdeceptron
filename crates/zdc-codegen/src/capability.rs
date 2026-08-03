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
//! `build` production, so a fourth capability costs a match arm.

use std::path::{Path, PathBuf};

use zdc_hir::BuildCapability;
use zdc_runtime::{Capability, Provided};

/// The prefix a refusal carries out through the JavaScript engine.
///
/// A capability reports failure by throwing, and the engine's message is
/// the only channel back. Marking it lets [`crate::evaluate`] tell a
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
            },
        })
        .collect()
}

/// `build read path` — one file's contents.
fn read(root: &Path, path: &str) -> Result<Provided, String> {
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
fn list(root: &Path, path: &str) -> Result<Provided, String> {
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
/// No extensions are enabled. Tables, footnotes and the rest are each a
/// decision about what the language's markdown *is*, and defaulting them
/// on would make the answer depend on a crate's idea of "common" rather
/// than on a specification.
///
/// # This function is the trusted base
///
/// `Markup` is the one type the renderer parses as HTML and this is the
/// one function that produces a `Markup`. Everything the type guarantees
/// is guaranteed here or nowhere, so what the renderer does with its input
/// was measured rather than assumed. Verbatim, from `pulldown-cmark`
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
/// generated only by `push_html` from events this function has already
/// approved, and there is no pass that parses generated HTML back.
///
/// 1. **Raw HTML becomes text.** [`Event::Html`] and [`Event::InlineHtml`]
///    are re-emitted as [`Event::Text`], which `push_html` escapes. A
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
fn markdown(_root: &Path, source: &str) -> Result<Provided, String> {
    use pulldown_cmark::{Event, Tag};

    let rewritten = pulldown_cmark::Parser::new(source).map(|event| match event {
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
    });

    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, rewritten);
    Ok(Provided::Markup(html))
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

    #[test]
    fn a_climbing_path_is_refused_before_it_is_opened() {
        let refused = read(&project(), "../Cargo.toml").expect_err("must refuse");
        assert!(refused.starts_with(REFUSED), "{refused}");
        assert!(refused.contains("climbs out of the project"), "{refused}");
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let refused = read(&project(), "/etc/hosts").expect_err("must refuse");
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

        let read_refusal = read(&root, "posts/secrets.md").expect_err("`read` must refuse");
        let list_refusal = list(&root, "posts").expect_err("`list` must refuse");

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
        let Provided::List(found) = list(&project(), "content").expect("lists") else {
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
        let Provided::Markup(html) = markdown(&project(), source).expect("renders") else {
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
}
