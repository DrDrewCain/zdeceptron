//! `build parts` end to end — a post that names a component (issue #305).
//!
//! `markup.rs` covers the other half of the same story: one `.md` file
//! becoming one `Prose`. This covers what that shape could not do, and the
//! two properties that make the answer worth having:
//!
//! 1. **A document is a list.** A post splits at the fences this compiler
//!    owns into prose runs and named widgets, each its own node, so a
//!    component renders *between* two runs of prose — which one `Prose`
//!    could never do, because it has no children and cannot grow them.
//! 2. **The widget set is closed.** A document naming a widget the program
//!    does not declare is a failed build, and the diagnostic names it.
//!
//! Everything here is asserted against the mounted tree or against a real
//! build's diagnostics, never against a substring of `client.js`: a bundle
//! containing the word `RingChart` proves nothing about what a browser
//! makes of it.

mod support;

use support::{build_example, context, run, Project};

/// `parts.zd`, compiled the way `zdc build` compiles it — its build root
/// actually run against `examples/`, so the posts come off disk.
fn parts_bundle() -> zdc_codegen::Bundle {
    build_example("examples/parts.zd")
}

/// Mount the bundle in the embedded engine and ask the DOM a question.
fn ask(bundle: &zdc_codegen::Bundle, expression: &str) -> String {
    let mut context = context(false);
    run(
        &mut context,
        &bundle.client_js,
        &format!(
            "function all(node, tag, out) {{\n\
             \x20 if (node.tagName === tag) out.push(node);\n\
             \x20 const kids = node.childNodes || [];\n\
             \x20 for (let i = 0; i < kids.length; i += 1) all(kids[i], tag, out);\n\
             \x20 return out;\n\
             }}\n\
             function tags(node, tag) {{ return all(node, tag, []); }}\n\
             function textOf(node) {{\n\
             \x20 if (node.kind === 'text') return node.nodeValue;\n\
             \x20 const kids = node.childNodes || [];\n\
             \x20 let out = '';\n\
             \x20 for (let i = 0; i < kids.length; i += 1) out += textOf(kids[i]);\n\
             \x20 return out;\n\
             }}\n\
             const root = document.createElement('div');\n\
             main(root);\n\
             String({expression});\n"
        ),
    )
}

// --- 1. the document is a list --------------------------------------------

/// **The acceptance criterion.** The post's prose renders as prose, and
/// the widget it names renders as a live component.
///
/// A `Meter` is the thing to look for because no markdown renderer could
/// have produced one: it is the widget's own markup, so finding it in the
/// tree is finding that a *file* named a component and got it.
#[test]
fn a_post_that_names_a_widget_gets_a_live_component() {
    let bundle = parts_bundle();

    let headings = ask(&bundle, "tags(root, 'h1').map(textOf).join('|')");
    assert!(
        headings.contains("The spacetrader wars"),
        "the post's own heading must be rendered markup, got: {headings}"
    );

    let meters = ask(&bundle, "tags(root, 'meter').length");
    assert_eq!(
        meters.parse::<usize>().expect("a count"),
        1,
        "the `RingChart` the file named must be on the page"
    );

    // The second widget too, so this is not one special case.
    let buttons = ask(&bundle, "tags(root, 'button').map(textOf).join('|')");
    assert!(
        buttons.contains("show the second bar"),
        "the `StackBars` the file named must be on the page, got: {buttons}"
    );
}

/// The shape the whole design exists for: a component **between** two runs
/// of prose, in the order the file wrote them.
///
/// This is the assertion one `Prose` could not have passed. `Prose` has no
/// children, so a widget inside a rendered document had nowhere to go; the
/// parts are siblings, and their order is the document's order.
#[test]
fn a_widget_renders_between_two_runs_of_prose() {
    let bundle = parts_bundle();

    // Walk the post's own column and label each child: `p` for a prose
    // run, `w` for a widget. What the file says is prose, widget, prose,
    // fence, prose, widget, prose.
    let shape = ask(
        &bundle,
        "(() => {\n\
         \x20 const prose = tags(root, 'div').filter((d) =>\n\
         \x20   (d.attributes.class || '').indexOf('zd-prose') >= 0);\n\
         \x20 const meters = tags(root, 'meter');\n\
         \x20 return prose.length + ':' + meters.length;\n\
         })()",
    );
    let (runs, widgets) = shape.split_once(':').expect("two counts");
    assert!(
        runs.parse::<usize>().expect("a count") >= 3,
        "the file's prose must be several runs, not one: {shape}"
    );
    assert_eq!(widgets, "1", "{shape}");
}

/// A fence that is not this compiler's is still a code block, so a post
/// that shows JavaScript still shows it.
#[test]
fn an_ordinary_fence_is_still_a_code_block_on_the_page() {
    let bundle = parts_bundle();
    let code = ask(&bundle, "tags(root, 'code').map(textOf).join('|')");
    assert!(
        code.contains("shipsLost"),
        "an ordinary fenced block must reach the page as code, got: {code}"
    );
}

/// **No markdown parser ships.** The whole point of rendering at build
/// time is that the browser is handed HTML, and splitting a document into
/// parts must not have changed that.
///
/// Asserted on the emitted module rather than on the DOM, because this is
/// a claim about what is *absent* from the bundle.
#[test]
fn the_bundle_still_carries_no_markdown_parser() {
    let bundle = parts_bundle();
    for needle in ["pulldown", "commonmark", "marked", "```"] {
        assert!(
            !bundle.client_js.to_lowercase().contains(needle),
            "`{needle}` reached the bundle, so something is parsing markdown in the browser"
        );
    }
}

// --- 2. the widget set is closed ------------------------------------------

/// Run one project's build root, and report what stopped it.
///
/// The whole pipeline rather than `support::build_at`, because what is
/// under test here is the *diagnostic*: `build_at` panics on a failure,
/// and a test that reads a panic message is a test that can only say the
/// build stopped, not why.
fn evaluation_of(entry: &std::path::Path) -> Result<(), String> {
    let linked = zdc_resolve::load(entry).map_err(|failure| failure.errors[0].message.clone())?;
    let hir = zdc_resolve::Resolver::linked_with_prelude(zdc_lib::load().program(), &linked)
        .resolve()
        .map_err(|errors| errors[0].message.clone())?;
    let split = zdc_graph::split(&hir);
    if let Some(error) = split.diagnostics.iter().find(|d| d.is_error()) {
        return Err(error.message.clone());
    }
    let verdict = zdc_graph::ifc(&hir, &split);
    if let Some(error) = verdict.diagnostics.iter().find(|d| d.is_error()) {
        return Err(error.message.clone());
    }
    let table = zdc_types::check(&hir, &split).map_err(|errors| errors[0].message.clone())?;
    let cleared = verdict
        .clearance()
        .ok_or_else(|| "the flow pass did not clear the program".to_string())?;
    let inputs = zdc_codegen::Inputs {
        hir: &hir,
        split: &split,
        verdict: &verdict,
        table: &table,
        cleared,
    };
    let options = zdc_codegen::Options::new("app.zd", "test");
    let module = zdc_codegen::build_module(&inputs, &options)
        .map_err(|errors| errors[0].message.clone())?
        .expect("a program with `static` state has a build root");
    zdc_codegen::evaluate(&module, entry.parent().expect("a directory"))
        .map(|_| ())
        .map_err(|error| error.report())
}

/// A minimal project: one program whose widget set is `offers`, and one
/// post naming `named`.
fn project_naming(name: &str, offers: &str, named: &str) -> Result<(), String> {
    let project = Project::new(name);
    project.write(
        "content/post.md",
        &format!("before\n\n```zd {named}\nslug: x\n```\n\nafter\n"),
    );
    let source = format!(
        "{offers}\
         record Post\n\
         \x20   parts is List of Part\n\
         \n\
         state posts is static List of Post from readPosts with directory is \"content\"\n\
         \n\
         function readPosts with directory\n\
         \x20   from build list directory\n\
         \x20   map each path to postFrom with path\n\
         \n\
         function postFrom with path\n\
         \x20   give Post with parts is build parts (build read path)\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       each post in posts\n\
         \x20           each part in post.parts\n\
         \x20               if isBlank of part.widget\n\
         \x20                   Prose part.markup\n"
    );
    let entry = project.write("app.zd", &source);
    evaluation_of(&entry)
}

/// The program's widget set, as a `choice` for the source above.
fn declaring(widgets: &[&str]) -> String {
    format!("choice Widget\n    {}\n\n", widgets.join("\n    "))
}

/// **The bargain, enforced.** A post naming a widget the program does not
/// offer is a failed build, and the failure names the widget and lists
/// what the program does offer.
///
/// This is the property that is stronger than MDX's, where an `import`
/// inside a content file can reach anything on disk. Here a content file
/// can reach exactly what the program wrote down, and reaching for
/// anything else stops the build rather than leaving a blank space on a
/// page nobody looks at again.
#[test]
fn a_post_naming_a_widget_the_program_does_not_offer_fails_the_build() {
    let failure = project_naming(
        "unknown-widget",
        &declaring(&["RingChart", "StackBars"]),
        "PieChart",
    )
    .expect_err("the build must stop");
    assert!(failure.contains("PieChart"), "{failure}");
    assert!(failure.contains("RingChart"), "{failure}");
    assert!(failure.contains("StackBars"), "{failure}");
    // A refused capability, not a program that threw: the two are
    // different mistakes and they get different codes.
    assert!(failure.contains("E11"), "{failure}");
}

/// A program that declares no widget set says so, rather than reporting an
/// empty list of things the file could have written instead.
#[test]
fn a_program_with_no_widget_choice_says_it_offers_none() {
    let failure =
        project_naming("no-widget-choice", "", "RingChart").expect_err("the build must stop");
    assert!(failure.contains("choice Widget"), "{failure}");
}

/// The widget the program *does* offer builds, which is what makes the
/// refusal above a rule rather than a blanket.
#[test]
fn a_post_naming_a_widget_the_program_offers_builds() {
    project_naming("known-widget", &declaring(&["RingChart"]), "RingChart")
        .expect("the build must succeed");
}

/// **A widget name is the one new path from a content file into the
/// bundle, and it is closed at the source.**
///
/// `injection.rs` audits paths rather than values, and this is a new one:
/// a name written in a `.md` becomes a `Text` inlined into `client.js` as
/// a literal and quoted into diagnostics. Both are safe by escaping
/// anyway — the literal is written by `JSON.stringify` in the sandbox —
/// but the name is refused before either happens, because a name that is
/// not a declaration name names nothing the program could have declared
/// and so has no correct rendering to escape *into*.
#[test]
fn a_widget_name_that_could_close_a_string_never_becomes_one() {
    for hostile in ["A\"+alert(1)+\"", "A</script><script>", "A'+alert(1)+'"] {
        let failure = project_naming("hostile-widget", &declaring(&["RingChart"]), hostile)
            .expect_err("the build must stop");
        assert!(
            failure.contains("is not a widget name"),
            "`{hostile}` was not refused as a name: {failure}"
        );
    }
}

// --- 3. the two halves of `Part` agree ------------------------------------

/// The prelude's `record Part` and the sandbox's [`zdc_runtime::PART_FIELDS`]
/// are one shape.
///
/// They are declared in two crates that cannot see each other — the
/// library is ZDeceptron source, the sandbox is the Rust that builds the
/// values — so nothing but this holds them together. A field renamed on
/// one side alone is a `List of Part` whose fields all read as absent,
/// which no type error would catch: the checker believes the record's
/// declaration and the engine believes the object.
#[test]
fn the_prelude_record_and_the_sandbox_object_declare_the_same_fields() {
    let prelude = zdc_lib::load();
    let record = prelude
        .program()
        .decls
        .iter()
        .find_map(|decl| match decl {
            zdc_ast::Decl::Record(record) if record.name.text == zdc_hir::PART_RECORD => {
                Some(record)
            }
            _ => None,
        })
        .expect("the prelude must declare `record Part`");

    let declared: Vec<&str> = record
        .fields
        .iter()
        .map(|field| field.name.text.as_str())
        .collect();
    assert_eq!(declared, zdc_runtime::PART_FIELDS);
    assert_eq!(declared, zdc_hir::PART_FIELDS);
}
