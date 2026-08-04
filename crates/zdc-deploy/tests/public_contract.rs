use std::collections::BTreeSet;

use zdc_deploy::{
    generate, LambdaFront, Options, Plan, Program, Target, VercelRuntime, COMPATIBILITY_DATE,
};

#[test]
fn deploy_targets_round_trip_through_the_cli_vocabulary() {
    let expected = [
        (Target::Cloudflare, "cloudflare", "Cloudflare Workers"),
        (Target::Lambda, "lambda", "AWS Lambda"),
        (Target::Vercel, "vercel", "Vercel Functions"),
        (Target::Deno, "deno", "Deno Deploy"),
    ];

    assert_eq!(Target::ALL.len(), expected.len());
    for (position, (target, slug, title)) in expected.into_iter().enumerate() {
        assert_eq!(Target::ALL[position], target);
        assert_eq!(target.slug(), slug);
        assert_eq!(target.title(), title);
        assert_eq!(Target::parse(slug), Ok(target));
    }
}

#[test]
fn invalid_deploy_target_lists_every_supported_alternative() {
    let message = Target::parse("azure").unwrap_err();

    for target in Target::ALL {
        assert!(
            message.contains(target.slug()),
            "missing {target:?}: {message}"
        );
    }
    assert!(message.contains("Azure Functions"));
    assert!(message.contains("deliberately absent"));
}

#[test]
fn lambda_fronts_round_trip_and_invalid_input_lists_them_all() {
    let expected = [
        (LambdaFront::FunctionUrl, "function-url"),
        (
            LambdaFront::ApiGatewayRestRegional,
            "api-gateway-rest-regional",
        ),
        (LambdaFront::ApiGatewayRestEdge, "api-gateway-rest-edge"),
        (LambdaFront::Alb, "alb"),
    ];

    assert_eq!(LambdaFront::ALL.len(), expected.len());
    for (front, slug) in expected {
        assert_eq!(front.slug(), slug);
        assert_eq!(LambdaFront::parse(slug), Ok(front));
    }

    let message = LambdaFront::parse("gateway").unwrap_err();
    for front in LambdaFront::ALL {
        assert!(
            message.contains(front.slug()),
            "missing {front:?}: {message}"
        );
    }
}

#[test]
fn vercel_runtimes_and_plans_round_trip_through_exact_words() {
    // Sized first: a round trip asserted only inside these loops is
    // satisfied by an empty table, and an empty table is exactly what a
    // refactor that dropped a variant would leave behind.
    assert_eq!(VercelRuntime::ALL.len(), 2);
    assert_eq!(Plan::ALL.len(), 2);
    for runtime in VercelRuntime::ALL {
        assert_eq!(VercelRuntime::parse(runtime.slug()), Ok(runtime));
    }
    for plan in Plan::ALL {
        assert_eq!(Plan::parse(plan.slug()), Ok(plan));
    }

    for invalid in ["", "Fluid", "node", "edge "] {
        assert!(
            VercelRuntime::parse(invalid).is_err(),
            "accepted `{invalid}`"
        );
    }
    for invalid in ["", "Free", "pro", "paid "] {
        assert!(Plan::parse(invalid).is_err(), "accepted `{invalid}`");
    }
}

#[test]
fn deployment_options_have_stable_conservative_defaults() {
    let options = Options::new(Target::Lambda, "sample-app");

    assert_eq!(options.target, Target::Lambda);
    assert_eq!(options.app, "sample-app");
    assert_eq!(options.front, LambdaFront::FunctionUrl);
    assert_eq!(options.runtime, VercelRuntime::Fluid);
    assert_eq!(options.plan, Plan::Free);
    assert_eq!(options.idle_seconds, 60);
    assert_eq!(options.poll_seconds, 2);
}

#[test]
fn live_sync_depends_only_on_the_presence_of_a_durable_key() {
    let durable = vec!["shared".to_string()];
    let environment = vec!["TOKEN".to_string()];
    let without_durable = Program {
        functions: &[],
        durable: &[],
        environment: &environment,
    };
    let with_durable = Program {
        functions: &[],
        durable: &durable,
        environment: &[],
    };

    assert!(!without_durable.live_sync());
    assert!(with_durable.live_sync());
}

#[test]
fn compatibility_date_is_a_pinned_iso_calendar_date() {
    let pieces: Vec<_> = COMPATIBILITY_DATE.split('-').collect();

    assert_eq!(pieces.len(), 3);
    assert_eq!(pieces[0].len(), 4);
    assert_eq!(pieces[1].len(), 2);
    assert_eq!(pieces[2].len(), 2);
    assert!(pieces.iter().all(|piece| piece.parse::<u32>().is_ok()));
}

#[test]
fn every_target_generates_sorted_unique_portable_core_files() {
    let program = Program {
        functions: &[],
        durable: &[],
        environment: &[],
    };

    for target in Target::ALL {
        let deployment = generate(&program, &Options::new(target, "empty-app"))
            .unwrap_or_else(|refusal| panic!("{target:?}: {}", refusal.message));
        let paths: Vec<_> = deployment
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        let unique: BTreeSet<_> = paths.iter().copied().collect();

        assert!(paths.windows(2).all(|pair| pair[0] < pair[1]), "{target:?}");
        assert_eq!(unique.len(), paths.len(), "duplicate path for {target:?}");
        for required in [
            "CAPABILITIES.md",
            "_zd/cells.js",
            "_zd/config.js",
            "_zd/endpoints.js",
            "_zd/router.js",
        ] {
            assert!(unique.contains(required), "{target:?} omitted {required}");
        }
    }
}

#[test]
fn generated_files_never_use_absolute_or_parent_paths() {
    let program = Program {
        functions: &[],
        durable: &[],
        environment: &[],
    };

    // Every target, and the count says so: a path rule asserted only
    // inside this loop would pass over an empty target list.
    assert_eq!(Target::ALL.len(), 4);
    for target in Target::ALL {
        let deployment = generate(&program, &Options::new(target, "safe-app")).unwrap();
        for file in deployment.files {
            assert!(!file.path.starts_with('/'), "{target:?}: {}", file.path);
            assert!(
                !file.path.split('/').any(|segment| segment == ".."),
                "{target:?}: {}",
                file.path
            );
        }
    }
}
