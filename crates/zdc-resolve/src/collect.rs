use std::collections::HashMap;
use zdc_ast::Decl;
use zdc_lexer::Span;

/// A name that could not be resolved, or a declaration that conflicts
/// with another.
///
/// Resolution collects these rather than returning at the first one: a
/// programmer with three undefined names should see three diagnostics
/// from one run, not one per run.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolveError {
    pub message: String,
    pub span: Span,
    /// What the caret says, for the errors that arrive with one.
    ///
    /// Module loading parses, so a parse error reaches a reader through
    /// this type. Before these three fields existed, that trip discarded
    /// the caret label, the repair and the code, and `zdc check` — the
    /// command a reader actually runs — printed the poorer diagnostic
    /// while `zdc parse` printed the better one.
    pub label: Option<String>,
    /// A repair the parser could name exactly.
    pub suggestion: Option<zdc_parser::Suggestion>,
    /// The rule, for the errors that carry one.
    pub code: Option<&'static str>,
}

impl ResolveError {
    /// A parse error, with everything it carries.
    pub fn from_parse(error: zdc_parser::ParseError) -> ResolveError {
        ResolveError {
            message: error.message,
            span: error.span,
            label: error.label,
            suggestion: error.suggestion,
            code: Some(error.code),
        }
    }
}

/// Every top-level name, with the index of the declaration that
/// introduced it.
///
/// Built in a pass of its own because top-level declarations are
/// order-independent: a signal may read one declared further down the
/// file, since the signal graph is a graph rather than a sequence.
#[derive(Debug, Default)]
pub struct GlobalTable {
    names: HashMap<String, usize>,
    /// What each module can see, by module index: its own declarations
    /// plus the ones it named in a `use`.
    ///
    /// A module is a unit of naming (§14D.2), so being linked into the
    /// same program is not the same as being visible. Without this, `use`
    /// would be a comment: every name would already be reachable.
    visible: Vec<HashMap<String, usize>>,
    /// Every variant name a `choice` declared, mapped to the declaration
    /// that owns it and its position in that declaration.
    ///
    /// Variant names live in a namespace of their own: `Archived` is a
    /// value and a pattern, never a signal or a function, and §14G.1.2
    /// makes a pattern name mean one variant of one choice. Two choices
    /// declaring the same variant would make `when` ambiguous, so that is
    /// a conflict rather than a shadowing rule.
    variants: HashMap<String, (usize, u32)>,
    /// The index of the `view` declaration, if the program has one.
    pub view: Option<usize>,
}

/// The variant names the language provides, which a `choice` may not
/// redeclare: `when` matches by name, so a program-declared `Ready` would
/// make a `Remote` arm mean two things (§14G.1.2).
///
/// Read off [`zdc_hir::BuiltinVariant`] rather than written out, so the
/// resolver cannot know a different set from the one the HIR can carry.
/// `Code`'s three arms arrived this way without this line being edited.
pub fn builtin_variants() -> Vec<&'static str> {
    zdc_hir::BuiltinVariant::ALL
        .iter()
        .map(|variant| variant.name())
        .collect()
}

/// Whether a name is one of those, without building the list.
pub fn is_builtin_variant(name: &str) -> bool {
    zdc_hir::BuiltinVariant::from_name(name).is_some()
}

impl GlobalTable {
    /// The index into `Program::decls` that declared this name, whichever
    /// module it came from.
    pub fn lookup(&self, name: &str) -> Option<usize> {
        self.names.get(name).copied()
    }

    /// The same lookup, restricted to what `module` may see.
    pub fn lookup_in(&self, module: usize, name: &str) -> Option<usize> {
        self.visible.get(module)?.get(name).copied()
    }

    /// Whether a name exists somewhere in the link but is not visible
    /// here, so the diagnostic can say "import it" rather than "it does
    /// not exist".
    pub fn is_declared_elsewhere(&self, module: usize, name: &str) -> bool {
        self.names.contains_key(name) && self.lookup_in(module, name).is_none()
    }

    /// The choice declaration and variant position this variant name
    /// belongs to.
    pub fn variant(&self, name: &str) -> Option<(usize, u32)> {
        self.variants.get(name).copied()
    }

    /// Whether any declaration in the program named this variant.
    pub fn declares_variant(&self, name: &str) -> bool {
        self.variants.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Register every top-level declaration of a single-module program.
pub fn collect(decls: &[&Decl], prelude: usize) -> Result<GlobalTable, Vec<ResolveError>> {
    collect_linked(decls, prelude, &vec![0; decls.len()], 1, &[Vec::new()])
}

/// Register every top-level declaration of a linked program, reporting
/// every conflict.
///
/// `decl_module` says which module each declaration came from and
/// `imports` what each module borrowed, so the table can answer "visible
/// from here" and not only "declared somewhere".
///
/// `prelude` says how many of the leading declarations came from the
/// prelude rather than from a file the programmer wrote (§17.4.1). It
/// changes two things. A collision with one of them is §14G.7.7 rule 1 and
/// has to be worded as shadowing a library name rather than pointing at a
/// line the programmer cannot see. And a prelude name is visible in
/// *every* module rather than in the one it was collected into: the
/// library is ambient, so a file reached through a `use` may call `length
/// of items` without importing it, exactly as the entry file may.
pub fn collect_linked(
    decls: &[&Decl],
    prelude: usize,
    decl_module: &[usize],
    module_count: usize,
    imports: &[Vec<crate::modules::Import>],
) -> Result<GlobalTable, Vec<ResolveError>> {
    let mut table = GlobalTable {
        visible: vec![HashMap::new(); module_count.max(1)],
        ..GlobalTable::default()
    };
    let mut errors = Vec::new();
    let mut first_seen: HashMap<String, usize> = HashMap::new();

    for (index, decl) in decls.iter().enumerate() {
        let module = decl_module.get(index).copied().unwrap_or(0);
        let (name, span) = match decl {
            Decl::State(state) => (state.name.text.clone(), state.name.span),
            Decl::Function(function) => (function.name.text.clone(), function.name.span),
            Decl::Foreign(foreign) => (foreign.name.text.clone(), foreign.name.span),
            Decl::Record(record) => (record.name.text.clone(), record.name.span),
            Decl::Component(component) => (component.name.text.clone(), component.name.span),
            // A release shares the one namespace signals and functions
            // share, because it is called exactly as a function is (§19.1).
            Decl::Release(release) => (release.name.text.clone(), release.name.span),
            // A request declares a signal, so it shares the namespace a
            // `state` does: `request feed` and `state feed` in one file
            // are two declarations of one name.
            Decl::Request(request) => (request.name.text.clone(), request.name.span),
            Decl::Choice(choice) => {
                collect_variants(choice, index, &mut table, &mut errors);
                (choice.name.text.clone(), choice.name.span)
            }
            // A route is a choice (§14G.2), so its variants share the one
            // variant namespace: `when page` names them exactly as it
            // names any other choice's, and two declarations claiming one
            // name is the same conflict either way.
            Decl::Route(route) => {
                collect_route_variants(route, index, &mut table, &mut errors);
                (route.name.text.clone(), route.name.span)
            }
            // Linking already turned every `use` into the declarations it
            // named; nothing is left to register here.
            Decl::Use(_) => continue,
            // A test is named by a sentence and referred to by nothing, so
            // it takes no name from the one namespace signals, functions
            // and types share (issue #169). Two tests may therefore make
            // the same claim — the report prints it twice, which is a
            // duplicated sentence and not a program that means two things
            // by one name.
            Decl::Test(_) => continue,
            Decl::View(view) => {
                if table.view.is_some() {
                    errors.push(ResolveError {
                        message: "A program has one `view`, and this is the second one. Move \
                                  these nodes into the first `view`."
                            .to_string(),
                        span: view.span,
                        label: None,
                        suggestion: None,
                        code: None,
                    });
                } else {
                    table.view = Some(index);
                }
                continue;
            }
        };

        // Signals, functions, records and choices share one namespace, so
        // `state a` and `function a` collide with each other and not only
        // with their own kind: a call site writes the same name either
        // way, and `Todo with …` is spelled exactly like a call.
        if let Some(earlier) = first_seen.get(&name).copied() {
            let message = if earlier < prelude {
                format!(
                    "`{name}` is the name of a standard-library operation, so this declaration \
                     would give one name two meanings. Rename this one."
                )
            } else {
                format!(
                    "`{name}` is already declared. Every top-level name in a program and the \
                     files it imports must be unique, because v1 has no aliasing to tell two \
                     of them apart (spec §14D.2). Rename one of them."
                )
            };
            errors.push(ResolveError {
                message,
                span,
                label: None,
                suggestion: None,
                code: None,
            });
            continue;
        }

        first_seen.insert(name.clone(), index);
        table.names.insert(name.clone(), index);
        if index < prelude {
            for visible in table.visible.iter_mut() {
                visible.insert(name.clone(), index);
            }
        } else {
            table.visible[module].insert(name, index);
        }
    }

    // A `use` is what makes a name from another file visible here. It runs
    // after every declaration is registered, so importing a name written
    // further down the imported file is as ordinary as reading a signal
    // declared below the one that reads it.
    for (module, borrowed) in imports.iter().enumerate() {
        for import in borrowed {
            let Some(index) = table.names.get(&import.name).copied() else {
                errors.push(ResolveError {
                    message: format!(
                        "`{}` is not declared in the file this imports from. An import names \
                         what it brings in, so check the spelling or add the declaration there.",
                        import.name
                    ),
                    span: import.span,
                    label: None,
                    suggestion: None,
                    code: None,
                });
                continue;
            };
            if decl_module.get(index).copied() != Some(import.from) {
                errors.push(ResolveError {
                    message: format!(
                        "`{}` is not declared in the file this imports from. It is declared \
                         somewhere else in this program, and v1 has no re-export, so import it \
                         from where it is written.",
                        import.name
                    ),
                    span: import.span,
                    label: None,
                    suggestion: None,
                    code: None,
                });
                continue;
            }
            table.visible[module].insert(import.name.clone(), index);
        }
    }

    if errors.is_empty() {
        Ok(table)
    } else {
        Err(errors)
    }
}

/// Register every variant of one `route`, which is a `choice` whose
/// variants happen to have URLs (§14G.2).
fn collect_route_variants(
    route: &zdc_ast::RouteDecl,
    index: usize,
    table: &mut GlobalTable,
    errors: &mut Vec<ResolveError>,
) {
    for (at, variant) in route.variants.iter().enumerate() {
        let name = variant.name.text.clone();
        if is_builtin_variant(&name) {
            errors.push(ResolveError {
                message: format!(
                    "`{name}` is one of the variants the language provides for `Option`, \
                     `Remote` and `Code`, so a `route` cannot declare it: a `when` arm named \
                     `{name}` would mean two things. Rename this route."
                ),
                span: variant.name.span,
                label: None,
                suggestion: None,
                code: None,
            });
            continue;
        }
        if table.variants.contains_key(&name) {
            errors.push(ResolveError {
                message: format!(
                    "`{name}` is already a variant of another `choice` or `route`. A `when` arm \
                     names one variant of one of them, so rename one."
                ),
                span: variant.name.span,
                label: None,
                suggestion: None,
                code: None,
            });
            continue;
        }
        let at = u32::try_from(at).expect("a route declares fewer than 2^32 variants");
        table.variants.insert(name, (index, at));
    }
}

/// Register every variant of one `choice`, reporting a name that already
/// means something else.
fn collect_variants(
    choice: &zdc_ast::ChoiceDecl,
    index: usize,
    table: &mut GlobalTable,
    errors: &mut Vec<ResolveError>,
) {
    for (at, variant) in choice.variants.iter().enumerate() {
        let name = variant.name.text.clone();
        if is_builtin_variant(&name) {
            errors.push(ResolveError {
                message: format!(
                    "`{name}` is one of the variants the language provides for `Option`, \
                     `Remote` and `Code`, so a `choice` cannot declare it: a `when` arm named \
                     `{name}` would mean two things. Rename this variant."
                ),
                span: variant.name.span,
                label: None,
                suggestion: None,
                code: None,
            });
            continue;
        }
        if table.variants.contains_key(&name) {
            errors.push(ResolveError {
                message: format!(
                    "`{name}` is already a variant of another `choice`. A `when` arm names one \
                     variant of one choice, so rename one of them."
                ),
                span: variant.name.span,
                label: None,
                suggestion: None,
                code: None,
            });
            continue;
        }
        let at = u32::try_from(at).expect("a choice declares fewer than 2^32 variants");
        table.variants.insert(name, (index, at));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(src: &str) -> zdc_ast::Program {
        zdc_parser::parse(src).expect("parses")
    }

    fn collect(program: &zdc_ast::Program) -> Result<GlobalTable, Vec<ResolveError>> {
        let decls: Vec<&Decl> = program.decls.iter().collect();
        super::collect(&decls, 0)
    }

    #[test]
    fn every_top_level_declaration_is_registered() {
        let p = program(
            "state a is client Text starting \"\"\nfunction f with x\n    give x\nview\n    Column\n",
        );
        let table = collect(&p).expect("collects");

        assert!(table.lookup("a").is_some());
        assert!(table.lookup("f").is_some());
        assert!(table.view.is_some());
    }

    /// The whole reason collection is a pass of its own: `a` is declared
    /// after `b` but read by it. Both must be in the table before any
    /// body is looked at.
    #[test]
    fn forward_references_are_fine_because_order_does_not_matter() {
        let p = program("state a is client Whole from b\nstate b is client Whole starting 1\n");
        let table = collect(&p).expect("collects");

        assert!(table.lookup("a").is_some());
        assert!(table.lookup("b").is_some());
    }

    #[test]
    fn duplicate_state_names_are_reported_against_the_second_declaration() {
        let src = "state a is client Text starting \"\"\nstate a is client Text starting \"\"\n";
        let p = program(src);
        let errors = collect(&p).unwrap_err();

        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("already declared"),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains('a'),
            "the message should name the duplicate: {}",
            errors[0].message
        );
        let second = src.rfind("a is").expect("two declarations") as u32;
        assert_eq!(
            errors[0].span,
            Span::new(second, second + 1),
            "the error belongs to the second declaration, not the first"
        );
    }

    #[test]
    fn a_function_may_not_share_a_name_with_a_signal() {
        let p = program("state a is client Text starting \"\"\nfunction a with x\n    give x\n");
        let errors = collect(&p).unwrap_err();

        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn two_views_are_rejected() {
        let p = program("view\n    Column\nview\n    Row\n");
        let errors = collect(&p).unwrap_err();

        assert!(
            errors[0].message.contains("one `view`"),
            "got: {}",
            errors[0].message
        );
    }

    /// Collection reports every conflict it finds, not just the first.
    #[test]
    fn every_conflict_is_reported_not_just_the_first() {
        let p = program(
            "state a is client Text starting \"\"\n\
             state a is client Text starting \"\"\n\
             state b is client Text starting \"\"\n\
             state b is client Text starting \"\"\n",
        );
        let errors = collect(&p).unwrap_err();

        assert_eq!(errors.len(), 2);
    }
}
