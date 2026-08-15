//! What each target generates, what it refuses, and what it admits to.

mod support;

use support::{compile_example, deploy, file, has, program};
use zdc_deploy::{
    Atomicity, LambdaFront, LiveSync, Options, Plan, Program, StreamBudget, Target, VercelRuntime,
};

fn options(target: Target) -> Options {
    Options::new(target, "guestbook")
}

fn refusal(example: &str, options: &Options) -> String {
    let bundle = compile_example(example);
    let program = program(&bundle);
    match zdc_deploy::generate(&program, options) {
        Ok(_) => panic!("expected {} to be refused", options.target.slug()),
        Err(refusal) => refusal.message,
    }
}

// ---------------------------------------------------------------- refusals

/// The refusal the whole design turns on: live sync behind an ALB cannot
/// work, so it is a build error naming the limitation rather than a stream
/// that silently never arrives.
#[test]
fn live_sync_behind_an_alb_is_a_build_error_naming_the_limitation() {
    let mut settings = options(Target::Lambda);
    settings.front = LambdaFront::Alb;
    let message = refusal("examples/guestbook.zd", &settings);

    assert!(
        message.contains("visits"),
        "the message names no durable key: {message}"
    );
    assert!(message.contains("Application Load Balancer"), "{message}");
    assert!(message.contains("1 MB"), "{message}");
    assert!(message.contains("Transfer-Encoding"), "{message}");
    assert!(
        message.contains("function-url"),
        "a refusal has to say what to do instead: {message}"
    );
}

/// The same front is *not* refused for a program that never streams.
/// Refusing a combination that works would be as wrong as allowing one that
/// does not.
#[test]
fn an_alb_is_allowed_for_a_program_with_no_durable_state() {
    let bundle = compile_example("examples/hello.zd");
    let mut settings = options(Target::Lambda);
    settings.front = LambdaFront::Alb;
    let deployment = {
        let program = program(&bundle);
        zdc_deploy::generate(&program, &settings).expect("hello.zd never streams")
    };
    let entry = &file(&deployment, "lambda.mjs").contents;
    assert!(
        !entry.contains("streamifyResponse"),
        "the ALB entry must not try to stream"
    );
    assert!(
        entry.contains("statusDescription"),
        "an ALB response needs one"
    );
}

#[test]
fn a_stream_with_no_idle_timeout_is_refused_on_lambda() {
    let mut settings = options(Target::Lambda);
    settings.idle_seconds = 0;
    let message = refusal("examples/guestbook.zd", &settings);
    assert!(message.contains("bills the full duration"), "{message}");
    assert!(message.contains("--idle-seconds"), "{message}");
}

#[test]
fn an_idle_timeout_that_can_never_fire_is_refused() {
    let mut settings = options(Target::Vercel);
    settings.idle_seconds = 400;
    let message = refusal("examples/guestbook.zd", &settings);
    assert!(
        message.contains("300"),
        "the message names the ceiling: {message}"
    );
}

#[test]
fn a_zero_poll_interval_is_refused() {
    let mut settings = options(Target::Deno);
    settings.poll_seconds = 0;
    let message = refusal("examples/guestbook.zd", &settings);
    assert!(message.contains("busy loop"), "{message}");
}

/// Azure is a deliberate exclusion, and the error says why rather than
/// listing the alternatives and leaving the user to wonder.
#[test]
fn azure_is_named_as_a_deliberate_exclusion() {
    let message = Target::parse("azure").expect_err("azure is not a target");
    assert!(message.contains("230 seconds"), "{message}");
    assert!(message.contains("atomic increment"), "{message}");
    assert!(
        message.contains("cloudflare, lambda, vercel, deno"),
        "{message}"
    );
}

// ------------------------------------------------------------ capabilities

#[test]
fn each_target_reports_its_documented_stream_ceiling() {
    let bundle = compile_example("examples/guestbook.zd");
    let budget = |settings: &Options| -> StreamBudget {
        let program = program(&bundle);
        zdc_deploy::generate(&program, settings)
            .expect("accepted")
            .capabilities
            .stream
    };

    assert!(matches!(
        budget(&options(Target::Cloudflare)),
        StreamBudget::Unlimited { .. }
    ));
    assert!(matches!(
        budget(&options(Target::Deno)),
        StreamBudget::Unlimited { .. }
    ));
    assert!(matches!(
        budget(&options(Target::Lambda)),
        StreamBudget::Seconds { seconds: 900, .. }
    ));
    assert!(matches!(
        budget(&options(Target::Vercel)),
        StreamBudget::Seconds { seconds: 300, .. }
    ));

    let mut pro = options(Target::Vercel);
    pro.plan = Plan::Paid;
    assert!(matches!(
        budget(&pro),
        StreamBudget::Seconds { seconds: 800, .. }
    ));

    let mut edge = options(Target::Vercel);
    edge.runtime = VercelRuntime::Edge;
    edge.plan = Plan::Paid;
    assert!(
        matches!(budget(&edge), StreamBudget::Seconds { seconds: 300, .. }),
        "Edge caps at 300 s regardless of plan"
    );
}

/// Live sync is pushed where a push channel exists and polled where one does
/// not, and the report says which — because "live sync: yes" would hide the
/// difference between a Durable Object and a two-second poll.
#[test]
fn the_report_distinguishes_a_push_channel_from_a_poll() {
    let bundle = compile_example("examples/guestbook.zd");
    let sync = |target: Target| -> LiveSync {
        let program = program(&bundle);
        zdc_deploy::generate(&program, &options(target))
            .expect("accepted")
            .capabilities
            .live_sync
    };
    assert!(matches!(sync(Target::Cloudflare), LiveSync::Push { .. }));
    assert!(matches!(sync(Target::Deno), LiveSync::Push { .. }));
    assert!(matches!(sync(Target::Lambda), LiveSync::Poll { .. }));
    assert!(matches!(sync(Target::Vercel), LiveSync::Poll { .. }));
}

#[test]
fn every_target_can_increment_atomically_and_says_how() {
    let bundle = compile_example("examples/guestbook.zd");
    for target in Target::ALL {
        let program = program(&bundle);
        let capabilities = zdc_deploy::generate(&program, &options(target))
            .expect("accepted")
            .capabilities;
        let mechanism = match &capabilities.atomicity {
            Atomicity::Native { mechanism }
            | Atomicity::CompareAndSet { mechanism }
            | Atomicity::Serialised { mechanism } => *mechanism,
        };
        assert!(
            mechanism.len() > 30,
            "{} claims atomicity without naming a mechanism",
            target.slug()
        );
    }
}

/// Lambda's billing model is the single biggest cost risk in the design, so
/// the report has to lead with it rather than bury it.
#[test]
fn the_lambda_report_warns_about_being_billed_after_the_client_leaves() {
    let (_, deployment) = deploy("examples/guestbook.zd", Target::Lambda);
    let report = deployment.capabilities.report();
    assert!(report.contains("not interrupted when the invoking client's connection is broken"));
    assert!(report.contains("wall clock"));
    assert!(report.contains("Idle timeout | 60 s"));
}

/// The numbers the report promises are the numbers the stream obeys.
#[test]
fn the_generated_config_carries_the_reported_timings() {
    let bundle = compile_example("examples/guestbook.zd");
    // An emptied `Target::ALL` would satisfy the loop below over nothing;
    // four platforms are what `zdc deploy` claims to support.
    assert_eq!(Target::ALL.len(), 4, "{:?}", Target::ALL);
    for target in Target::ALL {
        let program = program(&bundle);
        let deployment = zdc_deploy::generate(&program, &options(target)).expect("accepted");
        let config = &file(&deployment, "_zd/config.js").contents;
        let capabilities = &deployment.capabilities;
        assert!(config.contains(&format!("idleSeconds: {}", capabilities.idle_seconds)));
        assert!(config.contains(&format!(
            "heartbeatSeconds: {}",
            capabilities.heartbeat_seconds
        )));
        assert!(config.contains(&format!(
            "maxStreamSeconds: {}",
            capabilities.stream.ceiling_seconds()
        )));
    }
}

/// The heartbeat has to clear the shortest idle timeout anything in the path
/// enforces — 30 seconds on an edge-optimized API Gateway endpoint.
#[test]
fn the_heartbeat_clears_the_tightest_documented_idle_timeout() {
    let bundle = compile_example("examples/guestbook.zd");
    let mut settings = options(Target::Lambda);
    settings.front = LambdaFront::ApiGatewayRestEdge;
    let program = program(&bundle);
    let capabilities = zdc_deploy::generate(&program, &settings)
        .expect("accepted")
        .capabilities;
    assert!(
        capabilities.heartbeat_seconds * 2 <= 30,
        "a {} s heartbeat does not survive a 30 s idle timeout",
        capabilities.heartbeat_seconds
    );
    assert!(capabilities.report().contains("30 seconds"));
}

// ------------------------------------------------------------------ shims

/// The honest boundary, stated as a number. These are ceilings, not targets:
/// if a shim grows past one, the claim that the per-target surface is small
/// deserves re-examining rather than a bigger ceiling.
#[test]
fn the_per_target_shim_stays_small() {
    let bundle = compile_example("examples/guestbook.zd");
    for (target, ceiling) in [
        (Target::Cloudflare, 150),
        (Target::Lambda, 200),
        (Target::Vercel, 80),
        (Target::Deno, 120),
    ] {
        let program = program(&bundle);
        let shim = zdc_deploy::generate(&program, &options(target))
            .expect("accepted")
            .capabilities
            .shim;
        assert!(shim.entry_lines > 0 && shim.store_lines > 0);
        assert!(
            shim.total() <= ceiling,
            "{}'s shim is {} lines, over the {ceiling}-line ceiling: {}",
            target.slug(),
            shim.total(),
            shim.report()
        );
    }
}

/// The entry is the part ECMA-429 does not define, and on three of four
/// targets it is the same web-standard shape. Lambda is the exception, and
/// the exception is the finding.
#[test]
fn only_lambda_needs_a_non_standard_entrypoint() {
    let bundle = compile_example("examples/guestbook.zd");
    for (target, entry, marker) in [
        (Target::Cloudflare, "worker.js", "async fetch(request, env)"),
        (Target::Vercel, "api/index.js", "fetch(request)"),
        (Target::Deno, "main.js", "Deno.serve"),
    ] {
        let program = program(&bundle);
        let deployment = zdc_deploy::generate(&program, &options(target)).expect("accepted");
        let source = &file(&deployment, entry).contents;
        assert!(
            source.contains(marker),
            "{}: {entry} lost its shape",
            target.slug()
        );
        assert!(!source.contains("awslambda"));
    }

    let (_, lambda) = deploy("examples/guestbook.zd", Target::Lambda);
    let entry = &file(&lambda, "lambda.mjs").contents;
    assert!(entry.contains("awslambda.streamifyResponse"));
    assert!(
        entry.contains("out.once('drain'"),
        "the Node back-pressure loop is the type mismatch; do not lose it"
    );
}

// ----------------------------------------------------------------- secrets

/// No generated configuration file carries a secret value — only a
/// reference to the platform's own store.
#[test]
fn generated_configuration_references_secrets_and_never_carries_them() {
    let bundle = compile_example("examples/guestbook.zd");
    assert_eq!(bundle.environment, vec!["GREETING_API_KEY".to_string()]);
    assert!(
        !bundle.manifest_json.contains("GREETING_API_KEY"),
        "§16.3.12 assertion C: an environment key name must not reach the browser"
    );

    let (_, cloudflare) = deploy("examples/guestbook.zd", Target::Cloudflare);
    for line in file(&cloudflare, "wrangler.toml").contents.lines() {
        if line.contains("GREETING_API_KEY") {
            assert!(
                line.trim_start().starts_with('#'),
                "wrangler.toml sets a secret rather than naming one: {line}"
            );
        }
    }

    let (_, lambda) = deploy("examples/guestbook.zd", Target::Lambda);
    let template = &file(&lambda, "template.yaml").contents;
    assert!(template.contains(
        "GREETING_API_KEY: '{{resolve:secretsmanager:zd/test-app/secrets:SecretString:\
         GREETING_API_KEY}}'"
    ));

    // The two targets whose secrets live entirely outside the repository
    // must not mention the key in config at all.
    for (target, config) in [(Target::Vercel, "vercel.json"), (Target::Deno, "deno.json")] {
        let (_, deployment) = deploy("examples/guestbook.zd", target);
        assert!(!file(&deployment, config)
            .contents
            .contains("GREETING_API_KEY"));
    }
}

// ------------------------------------------------------------ per-target

#[test]
fn cloudflare_binds_a_sqlite_durable_object_and_static_assets() {
    let (_, deployment) = deploy("examples/guestbook.zd", Target::Cloudflare);
    let wrangler = &file(&deployment, "wrangler.toml").contents;
    assert!(wrangler.contains("main = \"worker.js\""));
    assert!(wrangler.contains("[assets]\ndirectory = \"./public\"\nbinding = \"ASSETS\""));
    assert!(wrangler.contains("[[durable_objects.bindings]]\nname = \"ZD_STORE\""));
    assert!(
        wrangler.contains("new_sqlite_classes = [\"ZdStore\"]"),
        "new key-value-backed namespaces are no longer created"
    );
    assert!(file(&deployment, "_zd/store.js")
        .contents
        .contains("export class ZdStore"));
}

#[test]
fn lambda_configures_response_streaming_and_a_two_part_key() {
    let (_, deployment) = deploy("examples/guestbook.zd", Target::Lambda);
    let template = &file(&deployment, "template.yaml").contents;
    assert!(
        template.contains("InvokeMode: RESPONSE_STREAM"),
        "buffered mode cannot stream at all"
    );
    assert!(template.contains("Timeout: 900"));
    // The store's physical model: (signal key, subkey), so an indexed
    // increment is a native atomic add on a cell of its own.
    assert!(template.contains("AttributeName: k\n          KeyType: HASH"));
    assert!(template.contains("AttributeName: s\n          KeyType: RANGE"));
    assert!(file(&deployment, "_zd/store.js")
        .contents
        .contains("if_not_exists(#n, :zero) + :delta"));
    assert!(has(&deployment, "package.json"));
}

/// Vercel KV and Vercel Postgres do not exist any more. Generating config
/// for either would be writing against 2024 documentation.
#[test]
fn vercel_generates_nothing_for_products_that_no_longer_exist() {
    let (_, deployment) = deploy("examples/guestbook.zd", Target::Vercel);
    for generated in &deployment.files {
        let lowered = generated.contents.to_lowercase();
        for gone in [
            "@vercel/kv",
            "@vercel/postgres",
            "postgres_url",
            "kv_rest_api_url",
        ] {
            assert!(
                !lowered.contains(gone),
                "{} mentions {gone}, which Vercel no longer sells",
                generated.path
            );
        }
    }
    let vercel = &file(&deployment, "vercel.json").contents;
    assert!(vercel.contains("\"maxDuration\": 300"));
    assert!(vercel.contains("\"source\": \"/_zd/(.*)\""));
    assert!(file(&deployment, "_zd/store.js")
        .contents
        .contains("UPSTASH_REDIS_REST_URL"));
}

/// `maxDuration` cannot be set when the runtime is `edge`, so it is not.
#[test]
fn the_vercel_edge_runtime_is_configured_in_the_module_not_in_the_json() {
    let bundle = compile_example("examples/guestbook.zd");
    let mut settings = options(Target::Vercel);
    settings.runtime = VercelRuntime::Edge;
    let program = program(&bundle);
    let deployment = zdc_deploy::generate(&program, &settings).expect("accepted");
    assert!(!file(&deployment, "vercel.json")
        .contents
        .contains("maxDuration"));
    assert!(file(&deployment, "api/index.js")
        .contents
        .contains("export const config = { runtime: 'edge' };"));
}

/// Deno KV's `watch` takes an explicit key list, not a prefix. The version
/// cell is how a `Map` signal is watched at all, so it is worth a test.
#[test]
fn deno_watches_a_version_key_per_signal_because_watch_takes_no_prefix() {
    let (_, deployment) = deploy("examples/guestbook.zd", Target::Deno);
    let store = &file(&deployment, "_zd/store.js").contents;
    assert!(store.contains("kv.watch(keys.map(version))"));
    assert!(
        store.contains("type: 'sum'"),
        "the version cell is bumped atomically"
    );
    assert!(
        store.contains("Deno.KvU64"),
        "and `sum` is the one place KvU64 is the right type"
    );
    let deno_json = &file(&deployment, "deno.json").contents;
    assert!(deno_json.contains("\"unstable\": [\"kv\"]"));
}

/// The two endpoint kinds do not share a calling convention, and the table
/// is where that is recorded.
#[test]
fn the_endpoint_table_records_the_calling_convention_of_each_endpoint() {
    let (_, deployment) = deploy("examples/guestbook.zd", Target::Cloudflare);
    let table = &file(&deployment, "_zd/endpoints.js").contents;
    assert!(table.contains("'greeting': { handler: $0, inputs: ['name'], command: false }"));
    assert!(table.contains("'visits.incr': { handler: $2, inputs: [], command: true }"));
}

/// A deployment is a function of its inputs. Two runs of the same program
/// must produce the same bytes, or every diff is noise.
#[test]
fn generation_is_deterministic() {
    let bundle = compile_example("examples/guestbook.zd");
    assert_eq!(Target::ALL.len(), 4, "{:?}", Target::ALL);
    for target in Target::ALL {
        let once = {
            let program: Program<'_> = program(&bundle);
            zdc_deploy::generate(&program, &options(target)).expect("accepted")
        };
        let twice = {
            let program: Program<'_> = program(&bundle);
            zdc_deploy::generate(&program, &options(target)).expect("accepted")
        };
        assert_eq!(
            once.files,
            twice.files,
            "{} is not deterministic",
            target.slug()
        );
    }
}

// ------------------------------------------------------------ cache (#137)

/// Every target says something about caching, and each says it in the
/// mechanism its own platform reads. The list of what may be cached is the
/// compiler's, so this asserts the *route* from bundle to config rather
/// than a hash nobody can predict.
#[test]
fn every_target_carries_the_cache_policy_in_its_own_mechanism() {
    let bundle = compile_example("examples/guestbook.zd");
    let hashed = bundle
        .immutable
        .first()
        .expect("the generated stylesheet carries a content hash")
        .clone();
    assert!(
        hashed.starts_with("styles.") && hashed.ends_with(".css"),
        "{hashed}"
    );
    assert_eq!(Target::ALL.len(), 4, "{:?}", Target::ALL);

    // Cloudflare: `_headers`, inside the assets directory `env.ASSETS`
    // serves, because `wrangler.toml` has no header table at all.
    let (_, cloudflare) = deploy("examples/guestbook.zd", Target::Cloudflare);
    let headers = &file(&cloudflare, "public/_headers").contents;
    assert!(
        headers.contains(&format!(
            "/{hashed}\n  Cache-Control: public, max-age=31536000, immutable"
        )),
        "{headers}"
    );
    assert!(
        !headers.contains("\n/index.html\n"),
        "the document must not be immutable, so it may carry no rule:\n{headers}"
    );

    // Vercel: a `headers` block in `vercel.json`, which is the only place
    // this target reads one from.
    let (_, vercel) = deploy("examples/guestbook.zd", Target::Vercel);
    let json = &file(&vercel, "vercel.json").contents;
    assert!(
        json.contains(&format!("\"source\": \"/{hashed}\"")),
        "{json}"
    );
    assert!(
        json.contains("\"value\": \"public, max-age=31536000, immutable\""),
        "{json}"
    );

    // Deno: the entry serves `public/` itself, so it is told both halves.
    let (_, deno) = deploy("examples/guestbook.zd", Target::Deno);
    let cache = &file(&deno, "_zd/cache.js").contents;
    assert!(cache.contains(&format!("'/{hashed}',")), "{cache}");
    assert!(
        cache.contains("public, max-age=31536000, immutable"),
        "{cache}"
    );
    assert!(
        cache.contains("public, max-age=0, must-revalidate"),
        "{cache}"
    );
    assert!(
        file(&deno, "main.js")
            .contents
            .contains("cacheControl(path)"),
        "the entry must apply what the table says"
    );

    // Lambda: nothing this tool writes is in the path of `public/`, so the
    // policy is stated to the person who puts it behind CloudFront.
    let (_, lambda) = deploy("examples/guestbook.zd", Target::Lambda);
    let report = &file(&lambda, "CAPABILITIES.md").contents;
    assert!(
        report.contains("public, max-age=31536000, immutable")
            && report.contains("public, max-age=0, must-revalidate"),
        "the report must name both halves for a target whose static host is \
         configured by hand:\n{report}"
    );
}
