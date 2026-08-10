use std::collections::{BTreeMap, BTreeSet};

use zdc_codegen::{
    document_path, file_name, runtime_files, tag_of, Evaluated, EvaluationError, FunctionKind,
    Mode, Options, BUILT_INS, HEADING_TAGS,
};

#[test]
fn codegen_options_start_empty_and_builders_replace_only_their_field() {
    let statics = BTreeMap::from([("count".into(), "3".into())]);
    let stylesheets = vec!["assets/base.css".into(), "assets/theme.css".into()];
    let options = Options::new("src/app.zd", "App")
        .with_statics(statics.clone())
        .with_stylesheets(stylesheets.clone());

    assert_eq!(options.source_path, "src/app.zd");
    assert_eq!(options.name, "App");
    assert_eq!(options.statics, statics);
    assert_eq!(options.stylesheets, stylesheets);

    let replaced = options.with_statics(BTreeMap::from([("other".into(), "true".into())]));
    assert_eq!(replaced.statics.len(), 1);
    assert_eq!(
        replaced.statics.get("other").map(String::as_str),
        Some("true")
    );
    assert_eq!(replaced.stylesheets.len(), 2);
}

#[test]
fn new_codegen_options_carry_no_host_results_or_asset_links() {
    let options = Options::new("main.zd", "main");

    assert!(options.statics.is_empty());
    assert!(options.stylesheets.is_empty());
}

#[test]
fn every_builtin_has_a_tag_and_builtin_names_are_unique() {
    let unique: BTreeSet<_> = BUILT_INS.iter().copied().collect();

    assert_eq!(unique.len(), BUILT_INS.len());
    assert!(!BUILT_INS.is_empty());
    for builtin in BUILT_INS {
        assert!(tag_of(builtin).is_some(), "missing tag for `{builtin}`");
    }
    assert_eq!(tag_of("NotAnElement"), None);
}

#[test]
fn heading_tags_cover_the_accessibility_levels_in_order() {
    assert_eq!(HEADING_TAGS, ["h1", "h2", "h3", "h4", "h5", "h6"]);
    assert_eq!(tag_of("Heading"), Some("h1"));
}

#[test]
fn canonical_urls_map_to_static_host_document_paths() {
    for (url, path) in [
        ("/", "index.html"),
        ("/blog", "blog/index.html"),
        ("/blog/post-1", "blog/post-1/index.html"),
        ("blog/post-1", "blog/post-1/index.html"),
    ] {
        assert_eq!(document_path(url), path);
        assert!(!document_path(url).starts_with('/'));
    }
}

#[test]
fn function_kinds_use_the_same_two_manifest_words_as_the_host() {
    assert_eq!(FunctionKind::Value.word(), "value");
    assert_eq!(FunctionKind::Command.word(), "command");
}

#[test]
fn endpoint_file_names_preserve_the_operation_suffix() {
    assert_eq!(file_name("visits.incr"), "functions/visits.incr.js");
    assert_eq!(file_name("profile.read"), "functions/profile.read.js");
}

#[test]
fn runtime_files_are_exactly_the_requested_known_modules_in_sorted_order() {
    let requested = BTreeSet::from([
        "runtime/store.js",
        "runtime/signal.js",
        "runtime/dom.js",
        "runtime/rpc.js",
        "runtime/wire.js",
    ]);
    let files = runtime_files(&requested, Mode::Release);
    let paths: Vec<_> = files.iter().map(|(path, _)| *path).collect();

    assert_eq!(paths, requested.into_iter().collect::<Vec<_>>());
    assert!(files.iter().all(|(_, source)| !source.trim().is_empty()));
    assert_eq!(runtime_files(&BTreeSet::new(), Mode::Release), Vec::new());
}

#[test]
fn runtime_file_sources_are_the_embedded_runtime_sources() {
    let requested = BTreeSet::from(["runtime/signal.js", "runtime/dom.js"]);
    let files = runtime_files(&requested, Mode::Development);

    assert_eq!(
        files[0],
        ("runtime/dom.js", zdc_runtime::DOM_JS.to_string())
    );
    assert_eq!(
        files[1],
        ("runtime/signal.js", zdc_runtime::SIGNAL_JS.to_string())
    );
}

/// A release build ships strictly less than a development one, and the
/// difference is exactly the assertions (#140).
#[test]
fn a_release_build_ships_less_runtime_than_a_development_build() {
    let requested = BTreeSet::from(["runtime/signal.js", "runtime/dom.js", "runtime/wire.js"]);
    let development = runtime_files(&requested, Mode::Development);
    let release = runtime_files(&requested, Mode::Release);

    let bytes = |files: &Vec<(&str, String)>| files.iter().map(|(_, s)| s.len()).sum::<usize>();
    assert!(
        bytes(&release) < bytes(&development),
        "release {} bytes, development {} bytes — the assertions are not being stripped",
        bytes(&release),
        bytes(&development)
    );
    for (name, source) in &release {
        assert!(
            !source.contains("$dev"),
            "{name} still carries a dev marker in a release build"
        );
    }
    assert!(
        development
            .iter()
            .any(|(name, source)| *name == "runtime/wire.js" && source.contains("assertEncoded")),
        "a development build must carry the assertion a release build drops"
    );
    assert!(
        release
            .iter()
            .all(|(_, source)| !source.contains("assertEncoded")),
        "a release build must not carry it"
    );
}

#[test]
fn evaluation_errors_render_code_message_and_help_as_one_report() {
    let error = EvaluationError {
        code: "E11",
        message: "computing `page` was refused".into(),
        help: "keep reads inside the project".into(),
    };

    assert_eq!(
        error.report(),
        "[E11] computing `page` was refused\n  help: keep reads inside the project"
    );
}

#[test]
fn an_unevaluated_build_result_contains_no_values_or_files() {
    let evaluated = Evaluated::default();

    assert!(evaluated.values.is_empty());
    assert!(evaluated.files.is_empty());
}
