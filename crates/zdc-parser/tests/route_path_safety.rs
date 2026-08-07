use zdc_ast::Decl;

fn route(path: &str) -> String {
    format!("route Site\n    Page is \"{path}\"\n")
}

#[test]
fn canonical_route_paths_are_accepted_verbatim() {
    for path in [
        "/",
        "/blog",
        "/blog/posts",
        "/.well-known",
        "/release-1.2",
        "/users/profile_2",
    ] {
        let program = zdc_parser::parse(&route(path)).unwrap_or_else(|error| {
            panic!(
                "`{path}` was rejected at {:?}: {}",
                error.span, error.message
            )
        });
        let Decl::Route(route) = &program.decls[0] else {
            panic!("expected a route declaration")
        };
        assert_eq!(route.variants[0].path, path);
    }
}

#[test]
fn route_paths_cannot_climb_out_of_the_build_directory() {
    for path in [
        "/../outside",
        "/./same",
        "/nested/../../outside",
        "/nested/..",
        "/%2e%2e/outside",
        // `\\` in the source is the one backslash the path holds: a
        // literal escapes it since #16, so writing it raw would be a lex
        // error and this rule would never be reached.
        "/back\\\\slash",
    ] {
        let error = zdc_parser::parse(&route(path)).expect_err("path must be refused");
        assert!(
            error.message.contains("canonical absolute path"),
            "{path}: {error:?}"
        );
        assert!(error.message.contains("`..`"), "{path}: {error:?}");
    }
}

#[test]
fn route_paths_have_one_canonical_filesystem_shape() {
    for path in [
        "//double",
        "/double//middle",
        "/trailing/",
        "/has space",
        "/query?key=value",
        "/fragment#heading",
        "/café",
    ] {
        let error = zdc_parser::parse(&route(path)).expect_err("path must be refused");
        assert!(
            error.message.contains("canonical absolute path"),
            "{path}: {error:?}"
        );
    }
}

#[test]
fn every_edit_prefix_is_refused_or_parsed_without_panicking() {
    let source = route("/nested/path");

    for boundary in source
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(source.len()))
    {
        let _ = zdc_parser::parse(&source[..boundary]);
    }
}
