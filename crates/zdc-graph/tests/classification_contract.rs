use zdc_ast::Placement;
use zdc_graph::authority::Flow;
use zdc_graph::integrity::Authority;
use zdc_graph::{
    classify, classify_write, unusable_path, CommandKey, Crossing, Ctx, MutCrossing, MutOp,
    PathKeySeg, ReadContext, Region, SignalPlacement, BUILD, CLIENT,
};
use zdc_hir::{ArenaId, DefId};

/// Every placement, taken from the enum rather than restated here.
///
/// It **was** restated here, as five of the six variants, and both
/// classifiers were extended for `remembered` while this list was not — so
/// the two tests below called themselves total over every placement while
/// never once passing `Remembered` to either. Both expected tables ended
/// in a wildcard that answered `remote` and `command` for it, where
/// `classify` and `classify_write` answer `direct` and `local`, so the
/// combinations they skipped are exactly the ones they would have failed.
const PLACEMENTS: [SignalPlacement; 6] = SignalPlacement::ALL;

#[test]
fn every_inhabitable_context_has_a_stable_type_context() {
    assert_eq!(
        Ctx::ALL.map(Ctx::read_context),
        [
            ReadContext::Static,
            ReadContext::Client,
            ReadContext::Client,
            ReadContext::ViewRootedServer,
            ReadContext::TriggerRootedServer,
        ]
    );
}

#[test]
fn context_descriptions_cover_the_five_public_contexts() {
    assert_eq!(Ctx::STATIC_BUILD.describe(), "build-time evaluation");
    assert_eq!(Ctx::CLIENT_VIEW.describe(), "the browser");
    assert_eq!(Ctx::CLIENT_TRIGGER.describe(), "a client-placed trigger");
    assert_eq!(
        Ctx::SERVER_VIEW.describe(),
        "a server invocation the view asked for"
    );
    assert_eq!(Ctx::SERVER_TRIGGER.describe(), "a trigger handler");
}

/// Three regions carry five spellings, and which spelling lands in which
/// region is the first thing a sixth placement has to answer.
///
/// The pairs are written out because each is a claim; the `assert_eq!` on
/// the length is what stops the list quietly falling behind
/// `Placement::ALL`, which is how `remembered` came to be absent from it.
#[test]
fn syntax_placements_map_to_their_runtime_regions() {
    let mapping = [
        (Placement::Client, SignalPlacement::Client, Region::Client),
        (Placement::Static, SignalPlacement::Static, Region::Static),
        (Placement::Server, SignalPlacement::Server, Region::Server),
        (Placement::Durable, SignalPlacement::Durable, Region::Server),
        // The store is the browser's, so the code that reads and writes it
        // runs where the browser is — which is the whole difference from
        // `durable`, and the reason this is `Client` and not `Server`.
        (
            Placement::Remembered,
            SignalPlacement::Remembered,
            Region::Client,
        ),
    ];
    assert_eq!(
        mapping.len(),
        Placement::ALL.len(),
        "a placement has no region here"
    );

    for (syntax, signal, region) in mapping {
        assert_eq!(zdc_graph::root::placement_of(syntax), signal);
        assert_eq!(zdc_graph::root::region_of(signal), region);
    }
    // The one placement with no syntax to map from (§14G.3a).
    assert_eq!(
        zdc_graph::root::region_of(SignalPlacement::DurablePerVisitor),
        Region::Server
    );
}

#[test]
fn read_classifier_is_total_over_every_public_context_and_placement() {
    // The product is what makes this total, so both sides are sized:
    // a classifier asserted only inside these loops is satisfied by an
    // empty context list or an empty placement list.
    assert_eq!(Ctx::ALL.len(), 5);
    assert_eq!(PLACEMENTS.len(), 6);
    for context in Ctx::ALL {
        for placement in PLACEMENTS {
            let crossing = classify(context, placement);
            let expected = match (context, placement) {
                (Ctx::STATIC_BUILD, SignalPlacement::Static) => "direct",
                (Ctx::STATIC_BUILD, _) => "E0301",
                (Ctx::CLIENT_VIEW | Ctx::CLIENT_TRIGGER, SignalPlacement::Client) => "direct",
                (Ctx::CLIENT_VIEW | Ctx::CLIENT_TRIGGER, SignalPlacement::Static) => "inline",
                // Above the wildcard, because the wildcard says `remote`
                // and the browser's own store is not remote from the
                // browser: it answers synchronously and nothing crosses.
                (Ctx::CLIENT_VIEW | Ctx::CLIENT_TRIGGER, SignalPlacement::Remembered) => "direct",
                (Ctx::CLIENT_VIEW | Ctx::CLIENT_TRIGGER, _) => "remote",
                (Ctx::SERVER_VIEW | Ctx::SERVER_TRIGGER, SignalPlacement::Static) => "inline",
                (Ctx::SERVER_VIEW | Ctx::SERVER_TRIGGER, SignalPlacement::Server) => "direct",
                (Ctx::SERVER_VIEW, SignalPlacement::Client) => "lift",
                (Ctx::SERVER_TRIGGER, SignalPlacement::Client) => "E0302",
                // A `remembered` value reaches a server derivation the way
                // a `client` one does — the browser sends it — and by the
                // same route it does not reach a trigger, which has no
                // browser to ask.
                (Ctx::SERVER_VIEW, SignalPlacement::Remembered) => "lift",
                (Ctx::SERVER_TRIGGER, SignalPlacement::Remembered) => "E0302",
                (Ctx::SERVER_VIEW | Ctx::SERVER_TRIGGER, SignalPlacement::Durable) => "store",
                (Ctx::SERVER_VIEW, SignalPlacement::DurablePerVisitor) => "visitor-store",
                (Ctx::SERVER_TRIGGER, SignalPlacement::DurablePerVisitor) => "E0303",
                _ => unreachable!("Ctx::ALL contains only inhabitable contexts"),
            };

            let actual = match crossing {
                Crossing::Direct => "direct",
                Crossing::Inline => "inline",
                Crossing::Remote { endpoint } if endpoint == CLIENT => "remote",
                Crossing::Lift { .. } => "lift",
                Crossing::Store {
                    per_visitor: false, ..
                } => "store",
                Crossing::Store {
                    per_visitor: true, ..
                } => "visitor-store",
                Crossing::Rejected { code } => code,
                // Written out rather than wildcarded: the guarded `Remote`
                // arm above takes only the client endpoint, so this is the
                // one shape left, and naming it means a new `Crossing`
                // variant is a compile error here rather than a panic in a
                // test nobody reads until it fires.
                other @ Crossing::Remote { .. } => {
                    panic!("unexpected classifier placeholder: {other:?}")
                }
            };
            assert_eq!(actual, expected, "{context:?} reading {placement:?}");
        }
    }
}

#[test]
fn write_classifier_is_total_over_every_public_context_and_placement() {
    // The product is what makes this total, so both sides are sized:
    // a classifier asserted only inside these loops is satisfied by an
    // empty context list or an empty placement list.
    assert_eq!(Ctx::ALL.len(), 5);
    assert_eq!(PLACEMENTS.len(), 6);
    for context in Ctx::ALL {
        for placement in PLACEMENTS {
            let crossing = classify_write(context, placement);
            let expected = match (context, placement) {
                (_, SignalPlacement::Static) => "E0310",
                (Ctx::CLIENT_VIEW | Ctx::CLIENT_TRIGGER, SignalPlacement::Client) => "local",
                (Ctx::CLIENT_VIEW | Ctx::CLIENT_TRIGGER, SignalPlacement::Server) => "E0311",
                // Above the wildcard, because the wildcard says `command`
                // and a write to the browser's own store asks nobody: no
                // endpoint is created and nothing leaves the tab.
                (Ctx::CLIENT_VIEW | Ctx::CLIENT_TRIGGER, SignalPlacement::Remembered) => "local",
                (Ctx::CLIENT_VIEW | Ctx::CLIENT_TRIGGER, _) => "command",
                (Ctx::SERVER_VIEW | Ctx::SERVER_TRIGGER, SignalPlacement::Client) => "E0312",
                // A server invocation does not reach into a browser, and
                // reaching into its disk is further still.
                (Ctx::SERVER_VIEW | Ctx::SERVER_TRIGGER, SignalPlacement::Remembered) => "E0312",
                (Ctx::SERVER_VIEW | Ctx::SERVER_TRIGGER, SignalPlacement::Server) => "local",
                (Ctx::SERVER_VIEW | Ctx::SERVER_TRIGGER, SignalPlacement::Durable) => "store",
                (Ctx::SERVER_VIEW, SignalPlacement::DurablePerVisitor) => "visitor-store",
                (Ctx::SERVER_TRIGGER, SignalPlacement::DurablePerVisitor) => "E0303",
                (Ctx::STATIC_BUILD, _) => "E0312",
                _ => unreachable!("Ctx::ALL contains only inhabitable contexts"),
            };

            let actual = match crossing {
                MutCrossing::Local => "local",
                MutCrossing::Command { root } if root == CLIENT => "command",
                MutCrossing::StoreWrite {
                    per_visitor: false, ..
                } => "store",
                MutCrossing::StoreWrite {
                    per_visitor: true, ..
                } => "visitor-store",
                MutCrossing::Rejected { code } => code,
                // The same, for the same reason: the guarded `Command` arm
                // above takes only the client root.
                other @ MutCrossing::Command { .. } => {
                    panic!("unexpected classifier placeholder: {other:?}")
                }
            };
            assert_eq!(actual, expected, "{context:?} writing {placement:?}");
        }
    }
}

#[test]
fn output_path_refusals_have_stable_reasons() {
    for (path, reason) in [
        ("", "is empty"),
        ("/asset.js", "is an absolute path"),
        ("\\asset.js", "is an absolute path"),
        ("C:\\asset.js", "names a drive or a scheme"),
        ("https:asset.js", "names a drive or a scheme"),
        ("../asset.js", "climbs out of the bundle"),
        ("dir/./asset.js", "climbs out of the bundle"),
        ("dir\\..\\asset.js", "climbs out of the bundle"),
        ("assets/", "names a directory rather than a file"),
        ("assets\\", "names a directory rather than a file"),
    ] {
        assert_eq!(unusable_path(path), Some(reason), "path `{path}`");
    }
}

#[test]
fn safe_relative_output_paths_are_accepted() {
    for path in [
        "asset.js",
        "assets/app.js",
        "assets\\app.js",
        "a..b/file.min.js",
        ".well-known/security.txt",
    ] {
        assert_eq!(unusable_path(path), None, "path `{path}`");
    }
}

#[test]
fn command_names_encode_every_mutation_operator_and_path_segment() {
    let signal = DefId::from_index(7);
    for (op, word) in [
        (MutOp::Set, "set"),
        (MutOp::Incr, "incr"),
        (MutOp::Decr, "decr"),
        (MutOp::Append, "append"),
        (MutOp::Remove, "remove"),
    ] {
        let key = CommandKey {
            signal,
            op,
            path: vec![PathKeySeg::Index, PathKeySeg::Field("done".into())],
        };
        assert_eq!(key.render("todos"), format!("todos.{word}.at.done"));
    }
}

#[test]
fn public_singleton_root_ids_remain_distinct() {
    assert_eq!(CLIENT.0, 0);
    assert_eq!(BUILD.0, 1);
    assert_ne!(CLIENT, BUILD);
}

#[test]
fn authority_flows_compose_and_fail_closed() {
    let summary = Flow::param(1).join(&Flow::param(0));
    assert_eq!(summary.depends_on().collect::<Vec<_>>(), [0, 1]);
    assert_eq!(
        summary.apply(&[Authority::Trusted, Authority::Trusted]),
        Authority::Trusted
    );
    assert_eq!(
        summary.apply(&[Authority::Trusted, Authority::Untrusted]),
        Authority::Untrusted
    );
    assert_eq!(summary.apply(&[Authority::Trusted]), Authority::Untrusted);
    assert_eq!(Flow::param(0).authority(), Authority::Untrusted);
}
