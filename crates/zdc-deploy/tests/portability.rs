//! The portability claim, asserted rather than asserted-to.
//!
//! Spec §8 says one artifact runs everywhere. The research this crate was
//! built from found that half true: ECMA-429 standardises the interior of a
//! handler and no entrypoint at all. These tests pin down which half is
//! which, so that if the portable half ever stops being portable the build
//! fails instead of the claim quietly becoming false.

mod support;

use boa_engine::{Context, Source};
use support::{compile_example, deploy, file, program};
use zdc_deploy::{Options, Target};

/// **The claim.** Compile one program, generate it for all four targets,
/// and diff the handler bodies.
///
/// If this ever fails, the portable core is not portable and that is the
/// most important thing anyone working on this crate could know.
#[test]
fn the_handler_bodies_are_byte_identical_on_every_target() {
    let bundle = compile_example("examples/guestbook.zd");
    assert!(
        !bundle.functions.is_empty(),
        "a program with no server functions cannot demonstrate anything"
    );

    let deployments: Vec<(Target, zdc_deploy::Deployment)> = Target::ALL
        .into_iter()
        .map(|target| {
            let program = program(&bundle);
            let deployment = zdc_deploy::generate(&program, &Options::new(target, "test-app"))
                .unwrap_or_else(|refusal| panic!("{}: {}", target.slug(), refusal.message));
            (target, deployment)
        })
        .collect();

    for function in &bundle.functions {
        for (target, deployment) in &deployments {
            let emitted = file(deployment, &function.path);
            assert_eq!(
                emitted.contents,
                function.source,
                "{} rewrote {} instead of copying it. The handler body is the whole \
                 portability claim; a target that edits one has broken it.",
                target.slug(),
                function.path
            );
        }
    }

    // And every target ships the same set of them.
    let expected: Vec<&str> = bundle
        .functions
        .iter()
        .map(|function| function.path.as_str())
        .collect();
    for (target, deployment) in &deployments {
        let mut shipped: Vec<&str> = deployment
            .files
            .iter()
            .map(|file| file.path.as_str())
            .filter(|path| path.starts_with("functions/"))
            .collect();
        shipped.sort();
        let mut wanted = expected.clone();
        wanted.sort();
        assert_eq!(
            shipped,
            wanted,
            "{} ships a different endpoint set",
            target.slug()
        );
    }
}

/// The adapter layered on top of the handlers is portable too. Only the
/// entry and the store binding are allowed to differ.
#[test]
fn the_router_and_the_cell_helpers_are_byte_identical_on_every_target() {
    let bundle = compile_example("examples/guestbook.zd");
    let mut seen: Vec<(Target, String, String)> = Vec::new();
    for target in Target::ALL {
        let program = program(&bundle);
        let deployment = zdc_deploy::generate(&program, &Options::new(target, "test-app"))
            .expect("every target accepts guestbook.zd");
        seen.push((
            target,
            file(&deployment, "_zd/router.js").contents.clone(),
            file(&deployment, "_zd/cells.js").contents.clone(),
        ));
    }
    let (first, router, cells) = seen[0].clone();
    for (target, other_router, other_cells) in &seen[1..] {
        assert_eq!(
            *other_router,
            router,
            "{} and {} disagree about the router",
            first.slug(),
            target.slug()
        );
        assert_eq!(
            *other_cells,
            cells,
            "{} and {} disagree about the cell helpers",
            first.slug(),
            target.slug()
        );
    }
}

/// The portable half uses only the minimum common web API.
///
/// This is a grep, and it is worth saying what it can and cannot do: it
/// catches a Node built-in or a platform global creeping into the shared
/// file, which is the mistake that actually happens. It cannot prove
/// conformance, because there is no WinterTC conformance suite to run —
/// wintertc.org lists no test-suite work item and the GitHub organisation
/// has no such repository.
#[test]
fn the_portable_half_names_no_platform_api() {
    let bundle = compile_example("examples/guestbook.zd");
    let program = program(&bundle);
    let deployment = zdc_deploy::generate(&program, &Options::new(Target::Cloudflare, "test-app"))
        .expect("cloudflare accepts guestbook.zd");

    let forbidden = [
        "awslambda",
        "process.env",
        "Deno.",
        "require(",
        "node:",
        "Buffer",
        "cloudflare:",
        "__dirname",
    ];
    for path in [
        "_zd/router.js",
        "_zd/cells.js",
        "_zd/endpoints.js",
        "_zd/config.js",
    ] {
        let source = &file(&deployment, path).contents;
        for name in forbidden {
            assert!(
                !source.contains(name),
                "{path} names `{name}`, which is not in ECMA-429. The portable half has to \
                 stay portable; put it in the entry shim."
            );
        }
    }
}

/// The emitted handlers themselves name no platform API either — which is
/// what makes copying them rather than rewriting them possible.
#[test]
fn the_emitted_handlers_name_no_platform_api() {
    // `voting-board.zd` would be the interesting third case — it is the one
    // with a durable `Map` — but it does not compile yet: §16.3.6's leading
    // text slot for `Row` and `Column` is unratified and the emitter refuses
    // rather than inventing the semantics.
    for example in ["examples/guestbook.zd", "examples/todo.zd"] {
        let bundle = compile_example(example);
        for function in &bundle.functions {
            for name in [
                "awslambda",
                "process.env",
                "Deno.",
                "require(",
                "node:",
                "import ",
            ] {
                assert!(
                    !function.source.contains(name),
                    "{example}: {} names `{name}`",
                    function.path
                );
            }
        }
    }
}

/// Every generated JavaScript file parses as an ES module.
///
/// The shims are hand-written and copied verbatim, so nothing else would
/// notice a syntax error in them until a platform did. Parsing is all this
/// can do — the engine has no `Response`, no `ReadableStream` and no
/// `Deno`, so evaluation would prove only that the shim is unrunnable
/// here, which is already known.
#[test]
fn every_generated_module_parses() {
    let bundle = compile_example("examples/guestbook.zd");
    for target in Target::ALL {
        let program = program(&bundle);
        let deployment = zdc_deploy::generate(&program, &Options::new(target, "test-app"))
            .expect("every target accepts guestbook.zd");
        let mut checked = 0;
        for generated in &deployment.files {
            if !generated.path.ends_with(".js") && !generated.path.ends_with(".mjs") {
                continue;
            }
            checked += 1;
            let mut context = Context::default();
            let source = Source::from_bytes(generated.contents.as_bytes());
            boa_engine::Module::parse(source, None, &mut context).unwrap_or_else(|error| {
                panic!(
                    "{}: {} does not parse: {error}",
                    target.slug(),
                    generated.path
                )
            });
        }
        assert!(
            checked >= 8,
            "{} generated only {checked} modules; the scan is not covering the adapter",
            target.slug()
        );
    }
}

/// A program with no crossing generates an empty endpoint table rather than
/// a broken one — the same principle as `hello.zd` shipping no `rpc.js`.
#[test]
fn a_program_with_no_server_work_still_generates_a_deployment() {
    let (bundle, deployment) = deploy("examples/hello.zd", Target::Cloudflare);
    assert!(bundle.functions.is_empty(), "hello.zd has no crossing");
    assert!(file(&deployment, "_zd/endpoints.js")
        .contents
        .contains("export const endpoints = {};"));
    assert!(bundle.durable.is_empty());
    assert!(bundle.environment.is_empty());
}
