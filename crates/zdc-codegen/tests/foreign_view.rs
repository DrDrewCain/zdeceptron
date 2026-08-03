//! `foreign … gives view` — the DOM-owning FFI (spec §14E.1, §14E.3).
//!
//! Two halves, and the second is the one that cannot be right by accident.
//!
//! The first half is emission: a foreign written in element position
//! becomes a `<div>` the template already carries plus one bind beside it,
//! so the form costs §16.2 R2's template-cloning model nothing. That is
//! checkable by reading the emitted string.
//!
//! The second half is **teardown**, and reading the emission cannot check
//! it. A foreign that is never destroyed leaks a WebGL context, a chart's
//! animation frame, an observer — none of which is visible in the DOM, in
//! the output, or in any assertion about the generated text. So the tests
//! below run the *real emitted module* against the *shipped runtime* in
//! the embedded engine and ask the module itself what happened to it.
//!
//! The one that matters most is
//! `a_disposed_foreign_is_never_updated_again`. Disposal cannot retract an
//! effect a flush has already queued, so "unsubscribed" and "will not run"
//! are different claims, and only the second one is safe.

mod support;

use support::{compile_source, context, refusals, resolve_refusals, run};

use boa_engine::{Context, Source};

/// A foreign module that records everything done to it.
///
/// The emitted bundle imports its export, and `flatten` strips the import
/// line — so the binding is installed here, under exactly the local name
/// the emitter chose. Recording rather than asserting: a test that only
/// counted `destroy` calls could not tell "destroyed once" from
/// "destroyed, then updated once more", which is the failure this file
/// exists to catch.
const RECORDER: &str = r#"
globalThis.$log = [];
globalThis.$live = 0;
globalThis.gauge = (node, props) => {
  const id = $log.filter((e) => e.startsWith('create')).length;
  $log.push('create:' + id + ':' + JSON.stringify(props));
  $live += 1;
  node.setAttribute('data-gauge', String(id));
  return {
    update(next) {
      $log.push('update:' + id + ':' + JSON.stringify(next));
    },
    destroy() {
      $log.push('destroy:' + id);
      $live -= 1;
    },
  };
};
"#;

/// A context with the runtime, the shim, and the recorder in it.
fn recording_context() -> Context {
    let mut context = context(false);
    context
        .eval(Source::from_bytes(RECORDER.as_bytes()))
        .expect("the recorder evaluates");
    context
}

/// Deliver a click to the button carrying a label.
///
/// A write reaches a signal only through a handler the program declared:
/// the emitter binds a setter when, and only when, something writes to
/// that signal (§16.3.2), so a test cannot reach in and set one. Driving
/// through the button is what the reader would do anyway.
const CLICK: &str = "\
const $click = (label) => {\n\
\x20 const $found = walk($c)\n\
\x20   .filter((n) => n.tagName === 'button')\n\
\x20   .find((n) => serialize(n).includes('>' + label + '<'));\n\
\x20 if ($found === undefined) throw new Error('no button labelled ' + label);\n\
\x20 $found.fire('click');\n\
};\n";

/// Mount `source`, run `driver`, and return the recorded log.
fn drive(source: &str, driver: &str) -> String {
    let bundle = compile_source(source);
    let mut context = recording_context();
    run(
        &mut context,
        &bundle.client_js,
        &format!(
            "const $c = document.createElement('div'); main($c);\n\
             {CLICK}{driver}\n$log.join(' | ')"
        ),
    )
}

/// A gauge inside an `if`, with a signal it reads and a way to hide it.
const IN_A_BRANCH: &str = "\
foreign gauge is client
    from \"./gauge.js\" as \"mount\"
    takes value is Whole
    gives view

state shown is client Truth starting yes
state level is client Whole starting 1

view
    Column
        if shown
            gauge value is level
        Button \"toggle\"
            on click
                set shown to not shown
        Button \"bump\"
            on click
                add 1 to level
";

/// A gauge per row, with buttons that reshape the list in one write.
///
/// `shrink` is the important one: it removes the last row *and* changes
/// the two that stay, in a single batch. That is the ordering in which a
/// disposed row's effect is already queued when the row is disposed.
const IN_A_LIST: &str = "\
foreign gauge is client
    from \"./gauge.js\" as \"mount\"
    takes value is Whole
    gives view

state levels is client List of Whole starting [1, 2, 3]

view
    Column
        each level in levels
            gauge value is level
        Button \"shrink\"
            on click
                set levels to [9, 8]
        Button \"empty\"
            on click
                set levels to empty
        Button \"grow\"
            on click
                set levels to [4, 5, 6]
        Button \"one\"
            on click
                set levels to [1]
        Button \"drop\"
            on click
                remove 3 from levels
";

/// Every row's foreign reads the **same** signal, and one handler writes
/// that signal *and* shortens the list.
///
/// This is the only shape in which "a disposed foreign is never updated
/// again" has any content. When a row reads only its own item, nothing
/// writes to a removed row's signal and no update could arrive. When rows
/// share an outer signal, the write invalidates every row's effect and the
/// list write disposes one of them — in the same batch, so the disposed
/// row's effect is already in the flush's drain list when it is disposed.
/// Unsubscribing does not retract it; only the guard does.
const SHARING_A_SIGNAL: &str = "\
foreign gauge is client
    from \"./gauge.js\" as \"mount\"
    takes value is Whole
    gives view

state levels is client List of Whole starting [1, 2, 3]
state scale is client Whole starting 1

view
    Column
        each level in levels
            gauge value is scale
        Button \"cut\"
            on click
                set levels to [1, 2]
                add 1 to scale
";

// --- the emission -----------------------------------------------------

/// The boundary in: a `<div>` in the static markup, and one bind beside it.
///
/// Not an anchor pair. `each` and `when` need two comments because the
/// compiler does not know at parse time how many roots their contents
/// have; a foreign's extent is exactly one element, and an element is what
/// the template model already clones and walks to.
#[test]
fn a_view_foreign_is_a_div_in_the_template_and_a_bind_beside_it() {
    let js = compile_source(IN_A_BRANCH).client_js;
    assert!(
        js.contains("<div></div>"),
        "the foreign's node is not in the static markup:\n{js}"
    );
    assert!(
        !js.contains("anchors()"),
        "the foreign was given an anchor pair rather than a node:\n{js}"
    );
    assert!(
        js.contains("foreign(") && js.contains("gauge, () => ({'value': level()})"),
        "the bind is not the declared shape:\n{js}"
    );
}

/// One property per `takes` argument, in **declaration** order — so the
/// object a module receives is the declaration's own shape however the
/// program chose to write the arguments.
#[test]
fn the_props_object_follows_the_declaration_not_the_call() {
    let js = compile_source(
        "\
foreign plot is client
    from \"./plot.js\" as \"mount\"
    takes width is Whole, label is Text
    gives view

view
    Column
        plot label is \"a\", width is 2
",
    )
    .client_js;
    assert!(
        js.contains("() => ({'width': 2, 'label': 'a'})"),
        "the props are not in declaration order:\n{js}"
    );
}

/// §14E.2: linked into whichever bundles actually call it. A declaration
/// nothing writes is not imported, so a `client` library costs nothing to
/// declare and not use.
#[test]
fn a_foreign_nothing_writes_is_not_imported() {
    let js = compile_source(
        "\
foreign gauge is client
    from \"./gauge.js\" as \"mount\"
    takes value is Whole
    gives view

view
    Column
        Text \"nothing here\"
",
    )
    .client_js;
    assert!(
        !js.contains("./gauge.js"),
        "an unwritten foreign was linked in anyway:\n{js}"
    );
}

// --- creation and reactive update --------------------------------------

/// The foreign is created once, with its declared arguments as a plain
/// object.
#[test]
fn a_view_foreign_is_created_with_its_arguments() {
    let log = drive(IN_A_BRANCH, "");
    assert_eq!(log, "create:0:{\"value\":1}");
}

/// **Reactivity is `update`, never re-invocation.**
///
/// This is the whole reason the form exists. Re-running `create` on a
/// signal write would drop and rebuild whatever the module owns — and a
/// WebGL context recreated on every keystroke is the failure this feature
/// was designed to prevent, not a performance note.
#[test]
fn a_signal_write_updates_rather_than_recreating() {
    let log = drive(IN_A_BRANCH, "$click('bump'); $click('bump');");
    assert_eq!(
        log, "create:0:{\"value\":1} | update:0:{\"value\":2} | update:0:{\"value\":3}",
        "a write must reach `update` and must not create a second instance"
    );
}

// --- teardown ----------------------------------------------------------

/// Disposal of an `if` branch tears the foreign down.
///
/// No new bookkeeping made this true: the branch already renders inside
/// `owned(...)`, and `onCleanup` pushes onto that same disposer list.
#[test]
fn destroy_runs_when_an_if_branch_is_disposed() {
    let log = drive(IN_A_BRANCH, "$click('toggle');");
    assert_eq!(log, "create:0:{\"value\":1} | destroy:0");
}

/// The same, for an `each` row that leaves the list.
#[test]
fn destroy_runs_when_an_each_row_leaves_the_list() {
    let log = drive(IN_A_LIST, "$click('drop');");
    assert_eq!(
        log, "create:0:{\"value\":1} | create:1:{\"value\":2} | create:2:{\"value\":3} | destroy:2",
        "the departed row's foreign, and only it, is torn down"
    );
}

/// The node leaves the document when its branch goes.
///
/// Asserted separately from `destroy` because they are separate claims: a
/// runtime could call `destroy` and leave the element parented, or detach
/// the element and never tell the module.
#[test]
fn the_node_leaves_the_document_when_its_branch_goes() {
    let bundle = compile_source(IN_A_BRANCH);
    let mut context = recording_context();
    let report = run(
        &mut context,
        &bundle.client_js,
        &format!(
            "const $c = document.createElement('div'); main($c);\n\
             {CLICK}\
             const $node = walk($c).find((n) => n.attributes && n.attributes['data-gauge']);\n\
             if ($node === undefined) throw new Error('the foreign never got a node');\n\
             const $before = $node.parentNode === null ? 'detached' : 'attached';\n\
             $click('toggle');\n\
             const $after = $node.parentNode === null ? 'detached' : 'attached';\n\
             $before + ' -> ' + $after"
        ),
    );
    assert_eq!(report, "attached -> detached");
}

/// Showing the branch again builds a *new* foreign rather than reviving
/// the destroyed one.
#[test]
fn a_foreign_is_created_again_when_its_branch_comes_back() {
    let log = drive(IN_A_BRANCH, "$click('toggle'); $click('toggle');");
    assert_eq!(
        log, "create:0:{\"value\":1} | destroy:0 | create:1:{\"value\":1}",
        "a remount is a new instance, not a resurrected one"
    );
    assert_eq!(
        drive(IN_A_BRANCH, "$click('toggle'); $click('toggle');"),
        "create:0:{\"value\":1} | destroy:0 | create:1:{\"value\":1}"
    );
}

/// Every instance is torn down when the whole list empties, so nothing is
/// left holding a context after the region it belonged to is gone.
#[test]
fn emptying_a_list_tears_down_every_row() {
    let bundle = compile_source(IN_A_LIST);
    let mut context = recording_context();
    let live = run(
        &mut context,
        &bundle.client_js,
        &format!(
            "const $c = document.createElement('div'); main($c);\n\
             {CLICK}$click('empty');\n\
             String($live)"
        ),
    );
    assert_eq!(live, "0", "a row left holding its foreign after removal");
}

/// **The one that cannot be right by accident.**
///
/// A write and a removal in the same flush is the case where
/// "unsubscribed" is not enough. `clearSources` unsubscribes an effect for
/// the *future*; an effect already in the drain list still runs. So the
/// row's update effect can be invoked after `eachInto` has already
/// disposed the row — calling `update` on a handle whose `destroy` has
/// run.
///
/// The failure is invisible from outside: a destroyed chart updated once
/// more usually throws inside the module, where the program never looks,
/// or silently writes to a canvas that is no longer in the document. It is
/// caught here by asking the module to record the order rather than by
/// asking the DOM what it looks like.
#[test]
fn a_disposed_foreign_is_never_updated_again() {
    let log = drive(SHARING_A_SIGNAL, "$click('cut');");
    assert_eq!(
        log,
        "create:0:{\"value\":1} | create:1:{\"value\":1} | create:2:{\"value\":1} | \
         destroy:2 | update:0:{\"value\":2} | update:1:{\"value\":2}",
        "row 2 is disposed and must not be updated afterwards"
    );

    // Stated again as the property rather than as the transcript, because
    // the transcript would still pass if `destroy:2` moved to the end.
    assert_eq!(
        instances_updated_after_destroy(&log),
        Vec::<String>::new(),
        "an instance was updated after its own destroy:\n{log}"
    );
}

/// The same claim over every reshaping this program can perform, so it is
/// a property rather than one scripted removal.
#[test]
fn no_instance_is_updated_after_its_own_destroy() {
    for driver in ["$click('cut');", "$click('cut'); $click('cut');"] {
        check_no_update_after_destroy(SHARING_A_SIGNAL, driver);
    }

    // And over the list program, whose rows read their own item.
    for driver in [
        "$click('shrink');",
        "$click('one'); $click('grow');",
        "$click('empty'); $click('one');",
        "$click('grow'); $click('one');",
        "$click('drop'); $click('grow');",
    ] {
        check_no_update_after_destroy(IN_A_LIST, driver);
    }
}

/// Every instance the log shows being updated after its own `destroy`.
///
/// The log entries are `what:id:payload`, so the id is what sits between
/// the first two colons.
fn instances_updated_after_destroy(log: &str) -> Vec<String> {
    let mut destroyed: Vec<String> = Vec::new();
    let mut offenders: Vec<String> = Vec::new();
    for entry in log.split(" | ") {
        let mut parts = entry.split(':');
        let (Some(what), Some(id)) = (parts.next(), parts.next()) else {
            continue;
        };
        match what {
            "destroy" => destroyed.push(id.to_string()),
            "update" if destroyed.iter().any(|d| d == id) => offenders.push(id.to_string()),
            _ => {}
        }
    }
    offenders
}

/// Drive `source` with `driver`, and require both that something was torn
/// down and that nothing was updated afterwards. The first half is what
/// stops this passing vacuously on a program that disposes nothing.
fn check_no_update_after_destroy(source: &str, driver: &str) {
    let log = drive(source, driver);
    assert!(
        log.contains("destroy:"),
        "`{driver}` tore nothing down, so it proves nothing:\n{log}"
    );
    assert_eq!(
        instances_updated_after_destroy(&log),
        Vec::<String>::new(),
        "an instance was updated after its own destroy, under `{driver}`:\n{log}"
    );
}

/// A write that arrives after disposal reaches neither `update` nor a
/// second `create`.
///
/// Named for what it checks rather than for the mechanism, because the
/// mechanism has two parts and only one of them is independently
/// observable. `dom.js` registers `destroy` through `onCleanup` *after*
/// its update effect, so a disposer list run in registration order
/// unsubscribes before it tears down; and the effect carries a `disposed`
/// flag for the runs a flush has already queued.
///
/// Swapping the registration order does not change any log this test can
/// read — nothing runs between `destroy()` and `clearSources()` — so the
/// ordering is defence in depth whose effect the flag masks. Stated here
/// rather than asserted by a test that would pass either way.
#[test]
fn a_write_after_disposal_reaches_neither_update_nor_create() {
    let log = drive(IN_A_BRANCH, "$click('toggle'); $click('bump');");
    assert_eq!(log, "create:0:{\"value\":1} | destroy:0");
}

// --- what the compiler refuses -----------------------------------------

/// "Anywhere" includes the places with no DOM, so it is not a weaker claim
/// than `is client` but a stronger and false one.
#[test]
fn gives_view_is_refused_on_a_site_that_may_have_no_dom() {
    let sites = ["server", "anywhere"];
    let mut checked = 0;
    for site in sites {
        let errors = resolve_refusals(&format!(
            "foreign gauge is {site}\n\
             \x20   from \"./gauge.js\" as \"mount\"\n\
             \x20   takes value is Whole\n\
             \x20   gives view\n"
        ));
        assert!(
            errors.iter().any(|e| e.contains("gives a view")
                && e.contains("can only be linked into the client bundle")),
            "`is {site}` was accepted for a DOM-owning foreign: {errors:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, sites.len(), "not every site was exercised");
    assert_eq!(
        checked, 2,
        "the two non-client sites are the whole of the rule"
    );
}

/// A foreign owns its node and everything under it, so markup written
/// inside would be markup the module is free to delete. Refused rather
/// than dropped.
#[test]
fn nothing_may_be_written_under_a_view_foreign() {
    let errors = refusals(
        "\
foreign gauge is client
    from \"./gauge.js\" as \"mount\"
    takes value is Whole
    gives view

view
    Column
        gauge value is 1
            Text \"inside\"
",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("owns this node and everything inside it")),
        "children under a foreign were accepted: {errors:?}"
    );
}

/// The boundary in is a plain JavaScript object, so what crosses is
/// scalars and lists of scalars. Everything else has an encoding the
/// module would have to know and the compiler is free to change.
#[test]
fn only_a_scalar_or_a_list_of_scalars_crosses_in() {
    let refused = ["Map of Text to Whole", "Option of Text", "Remote of Whole"];
    let mut checked = 0;
    for ty in refused {
        let errors = refusals(&format!(
            "foreign gauge is client\n\
             \x20   from \"./gauge.js\" as \"mount\"\n\
             \x20   takes value is {ty}\n\
             \x20   gives view\n\
             \n\
             view\n\
             \x20   Column\n\
             \x20       Text \"a\"\n"
        ));
        assert!(
            errors.iter().any(|e| e.contains("has no plain form")),
            "`{ty}` was accepted as a view foreign's argument: {errors:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, refused.len(), "not every type was exercised");
    assert_eq!(
        checked, 3,
        "the fixture list shrank without the assertion moving"
    );
}

#[test]
fn a_scalar_and_a_list_of_scalars_are_accepted() {
    let accepted = ["Text", "Whole", "Decimal", "Truth", "List of Whole"];
    let mut checked = 0;
    for ty in accepted {
        let source = format!(
            "foreign gauge is client\n\
             \x20   from \"./gauge.js\" as \"mount\"\n\
             \x20   takes value is {ty}\n\
             \x20   gives view\n\
             \n\
             view\n\
             \x20   Column\n\
             \x20       Text \"a\"\n"
        );
        support::try_compile(&source, "test.zd")
            .unwrap_or_else(|e| panic!("`{ty}` was refused: {:?}", e[0].message));
        checked += 1;
    }
    assert_eq!(checked, accepted.len(), "not every type was exercised");
    assert_eq!(
        checked, 5,
        "the four scalars and one list are the whole of the rule"
    );
}

/// A `gives view` foreign hands back no ZDeceptron value, so there is
/// nothing an expression could use. It is written as a view element and
/// nowhere else (§4.1: one phrasing per construct).
#[test]
fn a_view_foreign_cannot_be_called_for_a_result() {
    let errors = refusals(
        "\
foreign gauge is client
    from \"./gauge.js\" as \"mount\"
    takes value is Whole
    gives view

state n is client Whole starting 1
state m is client Whole from gauge with value is n

view
    Column
        Text m
",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("gives a view") && e.contains("hands back no value")),
        "a view foreign was callable in expression position: {errors:?}"
    );
}

/// A `foreign` that gives a value is not a view element, and the
/// diagnostic says which of the two mistakes was made.
#[test]
fn a_value_foreign_is_not_a_view_element() {
    let errors = resolve_refusals(
        "\
foreign lengthOf is anywhere
    from \"./m.js\" as \"len\"
    takes value is Text
    gives Whole

view
    Column
        lengthOf value is \"a\"
",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("gives a value") && e.contains("rather than written as a view")),
        "the diagnostic does not name the right mistake: {errors:?}"
    );
}

/// The parameter list is the declaration's, whichever position the foreign
/// is written in.
#[test]
fn a_view_foreign_element_is_checked_against_its_declaration() {
    let missing = refusals(
        "\
foreign gauge is client
    from \"./gauge.js\" as \"mount\"
    takes value is Whole
    gives view

view
    Column
        gauge
",
    );
    assert!(
        missing
            .iter()
            .any(|e| e.contains("missing an argument for `value`")),
        "a missing argument was accepted: {missing:?}"
    );

    let wrong = refusals(
        "\
foreign gauge is client
    from \"./gauge.js\" as \"mount\"
    takes value is Whole
    gives view

view
    Column
        gauge value is \"text\"
",
    );
    assert!(
        wrong.iter().any(|e| e.contains("`Whole` is expected here")),
        "an argument of the wrong type was accepted: {wrong:?}"
    );

    let unknown = refusals(
        "\
foreign gauge is client
    from \"./gauge.js\" as \"mount\"
    takes value is Whole
    gives view

view
    Column
        gauge value is 1, colour is \"red\"
",
    );
    assert!(
        unknown
            .iter()
            .any(|e| e.contains("has no parameter named `colour`")),
        "an undeclared argument was accepted: {unknown:?}"
    );
}

/// §14E.3 row 1, as `E-IFC-13`: a secret may cross into a foreign only
/// where the call sits in server context, and a `foreign … is client` is
/// never in server context.
///
/// The case no other rule covers. The value does not cross a boundary the
/// compiler emits — it leaves through the module's own import — so neither
/// the view sink nor client state ever sees it.
#[test]
fn a_secret_may_not_reach_a_client_foreign() {
    let errors = refusals(
        "\
secret state apiKey is server Text from environment \"KEY\"

foreign hashOf is client
    from \"./hash.js\" as \"digest\"
    takes input is Text
    gives Text

state shown is server Text from hashOf with input is apiKey

view
    Column
        Text \"hello\"
",
    );
    assert!(
        errors.iter().any(|e| e.contains("is `foreign … is client`")
            && e.contains("handed to JavaScript running in the browser")),
        "a secret reached a client foreign: {errors:?}"
    );
}

/// The same value through a `server` foreign is accepted, so the rule is
/// about where the module is linked rather than about foreigns at large.
#[test]
fn a_secret_may_reach_a_server_foreign() {
    support::try_compile(
        "\
secret state apiKey is server Text from environment \"KEY\"

foreign hashOf is server
    from \"./hash.js\" as \"digest\"
    takes input is Text
    gives Text

secret state shown is server Text from hashOf with input is apiKey

view
    Column
        Text \"hello\"
",
        "test.zd",
    )
    .unwrap_or_else(|e| panic!("a server foreign was refused a secret: {:?}", e[0].message));
}
