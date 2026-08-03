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
pub const BUILTIN_VARIANTS: &[&str] = &["Loading", "Ready", "Failed", "Some", "None"];

impl GlobalTable {
    /// The index into `Program::decls` that declared this name.
    pub fn lookup(&self, name: &str) -> Option<usize> {
        self.names.get(name).copied()
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

/// Register every top-level declaration, reporting every conflict.
///
/// `prelude` says how many of the leading declarations came from the
/// prelude rather than from the file being compiled (§17.4.1). It changes
/// nothing about how names are registered and everything about how a
/// collision is worded: a user declaration that shadows a library one is
/// §14G.7.7 rule 1, and the message has to say which library name it means
/// rather than pointing at a line the programmer cannot see.
pub fn collect(decls: &[&Decl], prelude: usize) -> Result<GlobalTable, Vec<ResolveError>> {
    let mut table = GlobalTable::default();
    let mut errors = Vec::new();
    let mut first_seen: HashMap<String, usize> = HashMap::new();

    for (index, decl) in decls.iter().enumerate() {
        let (name, span) = match decl {
            Decl::State(state) => (state.name.text.clone(), state.name.span),
            Decl::Function(function) => (function.name.text.clone(), function.name.span),
            Decl::Foreign(foreign) => (foreign.name.text.clone(), foreign.name.span),
            Decl::Record(record) => (record.name.text.clone(), record.name.span),
            Decl::Choice(choice) => {
                collect_variants(choice, index, &mut table, &mut errors);
                (choice.name.text.clone(), choice.name.span)
            }
            Decl::View(view) => {
                if table.view.is_some() {
                    errors.push(ResolveError {
                        message: "A program has one `view`, and this is the second one. Move \
                                  these nodes into the first `view`."
                            .to_string(),
                        span: view.span,
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
                    "`{name}` is already declared earlier in this file. Every top-level name \
                     must be unique, so rename one of them."
                )
            };
            errors.push(ResolveError { message, span });
            continue;
        }

        first_seen.insert(name.clone(), index);
        table.names.insert(name, index);
    }

    if errors.is_empty() {
        Ok(table)
    } else {
        Err(errors)
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
        if BUILTIN_VARIANTS.contains(&name.as_str()) {
            errors.push(ResolveError {
                message: format!(
                    "`{name}` is one of the variants the language provides for `Option` and \
                     `Remote`, so a `choice` cannot declare it: a `when` arm named `{name}` \
                     would mean two things. Rename this variant."
                ),
                span: variant.name.span,
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
