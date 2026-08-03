//! The two roots that cannot import, and the names they have to declare.
//!
//! The client bundle imports `variant` from `dom.js` and prints the `$`
//! prelude helpers in its own preamble. Neither other root can do either:
//! §17.4.8 runs the build root in the compiler's sandbox, which has no
//! `dom.js` in it, and §8.2 gives a server root `$env` and `$store` from a
//! platform adapter and nothing else.
//!
//! So both printed calls to names nothing defined. A `static` holding a
//! variant printed `variant('Busy')` and the build stopped with E10; a
//! `static` reaching any prelude primitive with a helper form — `length
//! of` is the shortest — stopped the same way. A server root printed the
//! same calls and would have thrown a `ReferenceError` on the first
//! request instead, which is the quieter of the two failures.

mod support;

use support::{build_module_of, compile_source, context, flatten, run};

/// Run a build root the way `zdc build` does, and give back what it
/// computed for one `static` signal.
fn computed(source: &str, name: &str) -> String {
    let module = build_module_of(source, "test.zd")
        .expect("this program declares `static` state, so it has a build root");
    zdc_codegen::evaluate(&module, std::path::Path::new("."))
        .unwrap_or_else(|error| panic!("the build root did not run: {}", error.report()))
        .values
        .remove(name)
        .unwrap_or_else(|| panic!("`{name}` is `static`, so the build root computes it"))
}

const STATUS: &str = "choice Status\n\
                      \x20   Idle\n\
                      \x20   Busy\n\
                      state status is static Status starting Busy\n\
                      view\n\
                      \x20   Column\n\
                      \x20       when status\n\
                      \x20           Idle show Text \"idle\"\n\
                      \x20           Busy show Text \"busy\"\n";

/// The demonstration, at the layer the defect was reported from: the
/// value is *computed*, not supplied by hand.
#[test]
fn a_static_holding_a_variant_is_computed_by_the_build_root() {
    assert_eq!(computed(STATUS, "status"), r#"{"tag":"Busy","fields":[]}"#);
}

/// And it is the shape `whenInto` dispatches on. Asserted against the
/// runtime's own `variant` rather than against a written-out literal, so
/// the build root's copy cannot drift away from the one `dom.js` ships
/// without this failing.
#[test]
fn the_build_roots_variant_builds_what_the_runtimes_variant_builds() {
    let mut context = context(false);
    let from_runtime = run(&mut context, "", "JSON.stringify(variant('Busy'))");
    assert_eq!(computed(STATUS, "status"), from_runtime);
}

/// A `static` reaching a prelude primitive that compiles to a helper. The
/// same gap, and not about variants at all.
#[test]
fn a_static_reaching_a_prelude_helper_is_computed_by_the_build_root() {
    let computed = computed(
        "state size is static Whole starting length of \"hello\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text size\n",
        "size",
    );
    assert_eq!(computed, "5");
}

/// A build root that needs neither declares neither, so a program that
/// only reads text at build time still prints the module it printed
/// before.
#[test]
fn a_build_root_that_needs_nothing_declares_nothing() {
    let module = build_module_of(
        "state greeting is static Text starting \"hello\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text greeting\n",
        "test.zd",
    )
    .expect("this program declares `static` state");
    assert!(
        !module.source.contains("const variant"),
        "nothing here constructs a variant:\n{}",
        module.source
    );
    assert!(
        !module.source.contains("$textLength"),
        "nothing here reaches a helper:\n{}",
        module.source
    );
}

/// The server half of the same gap. A handler that constructs a variant
/// has to be able to run, and running it is the assertion — a grep for
/// `const variant` would pass on a definition that was shadowed or
/// misplaced relative to its first use.
#[test]
fn a_server_root_that_constructs_a_variant_defines_it() {
    let bundle = compile_source(
        "choice Status\n\
         \x20   Idle\n\
         \x20   Busy\n\
         function pick with flag\n\
         \x20   if flag\n\
         \x20       give Busy\n\
         \x20   give Idle\n\
         state flag is durable Truth starting yes\n\
         state status is server Status from pick with flag\n\
         view\n\
         \x20   Column\n\
         \x20       when status\n\
         \x20           Loading show Text \"wait\"\n\
         \x20           Failed with error show Text \"bad\"\n\
         \x20           Ready with s show Text \"ok\"\n",
    );
    let function = bundle
        .functions
        .iter()
        .find(|function| function.name == "status")
        .expect("the `server` signal is an endpoint");
    let mut context = boa_engine::Context::default();
    let value = context
        .eval(boa_engine::Source::from_bytes(
            format!("{}\nJSON.stringify(pick(true))", flatten(&function.source)).as_bytes(),
        ))
        .unwrap_or_else(|e| panic!("the handler did not run: {e}\n\n{}", function.source))
        .to_string(&mut context)
        .expect("a string")
        .to_std_string_escaped();
    assert_eq!(value, r#"{"tag":"Busy","fields":[]}"#);
}

/// The server half for helpers, which is the same gap once more.
#[test]
fn a_server_root_that_reaches_a_prelude_helper_defines_it() {
    let bundle = compile_source(
        "function sizeOf with t\n\
         \x20   give length of t\n\
         state raw is durable Text starting \"hello\"\n\
         state size is server Whole from sizeOf with raw\n\
         view\n\
         \x20   Column\n\
         \x20       when size\n\
         \x20           Loading show Text \"wait\"\n\
         \x20           Failed with error show Text \"bad\"\n\
         \x20           Ready with n show Text n\n",
    );
    let function = bundle
        .functions
        .iter()
        .find(|function| function.name == "size")
        .expect("the `server` signal is an endpoint");
    let mut context = boa_engine::Context::default();
    let value = context
        .eval(boa_engine::Source::from_bytes(
            format!("{}\nString(sizeOf('hello'))", flatten(&function.source)).as_bytes(),
        ))
        .unwrap_or_else(|e| panic!("the handler did not run: {e}\n\n{}", function.source))
        .to_string(&mut context)
        .expect("a string")
        .to_std_string_escaped();
    assert_eq!(value, "5");
}

/// Each endpoint declares its own, rather than the first one emitted
/// carrying them for everybody. The emitter's symbol sets are cumulative,
/// so a difference taken against a running total would have left the
/// second endpoint declaring nothing.
#[test]
fn two_endpoints_that_reach_the_same_helper_both_declare_it() {
    let bundle = compile_source(
        "function sizeOf with t\n\
         \x20   give length of t\n\
         state alpha is durable Text starting \"one\"\n\
         state beta is durable Text starting \"two\"\n\
         state a is server Whole from sizeOf with alpha\n\
         state b is server Whole from sizeOf with beta\n\
         view\n\
         \x20   Column\n\
         \x20       when a\n\
         \x20           Loading show Text \"wait\"\n\
         \x20           Failed with error show Text \"bad\"\n\
         \x20           Ready with n show Text n\n\
         \x20       when b\n\
         \x20           Loading show Text \"wait\"\n\
         \x20           Failed with error show Text \"bad\"\n\
         \x20           Ready with n show Text n\n",
    );
    assert_eq!(bundle.functions.len(), 2, "one endpoint each");
    for function in &bundle.functions {
        assert!(
            function.source.contains("const $textLength"),
            "`{}` calls `$textLength` and has to declare it:\n{}",
            function.name,
            function.source
        );
    }
}
