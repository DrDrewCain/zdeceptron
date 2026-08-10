use crate::collect::{
    builtin_variants, collect, collect_linked, is_builtin_variant, GlobalTable, ResolveError,
};
use crate::instantiate::instantiate;
use crate::modules::Linked;
use crate::packages::{Mapping, Packages};
use crate::scope::Scopes;
use std::collections::{HashMap, HashSet};
use zdc_ast as ast;
use zdc_hir::{
    destination_as_href, Builtin, BuiltinElement, BuiltinVariant, Choice, Component, Def, DefId,
    DefKind, ExprId, Field, Foreign, Function, Hir, HirArg, HirArm, HirArmBody, HirBind,
    HirBinding, HirBlock, HirEach, HirEachNode, HirElement, HirExpr, HirExprKind, HirHandler,
    HirIf, HirIfNode, HirMutation, HirNode, HirNodeArm, HirNodeArmBody, HirPathSeg, HirPipeline,
    HirPlace, HirStmt, HirWhen, HirWhenNode, Local, LocalId, LocalSignal, ModuleTarget,
    OperatorName, Record, Res, RouteParam, RouteTable, RouteVariantInfo, Signal, Variant, View,
    BUILTIN_OF_OPERATORS, DESTINATION_ARGUMENT, DESTINATION_ELEMENT,
};

/// The view elements the language provides.
///
/// Per §14G.7.7 rule 1 these live in the ordinary module namespace, so a
/// user `component Paragraph` is a redeclaration error naming the built-in
/// — which is what makes `Row` and `VoteCard` indistinguishable at the call
/// site (§14D.1) rather than there being a privileged set.
///
/// **One name per element, and no synonyms.** §4.1 forbids two phrasings
/// for one construct, so there is exactly one way to write a paragraph and
/// no escape hatch naming a raw tag alongside it. The names are chosen for
/// what the element means rather than for the tag it becomes: the mapping
/// lives in `zdc-codegen`'s shape table, which is the only place a tag name
/// is written, so §16.1's template cloning keeps a compile-time-constant
/// tag for every element in the language.
///
/// Public so an editor offers exactly the names this pass accepts. A
/// completion list that is its own copy of this is a second table that
/// drifts, which is the defect `scripts/check-grammar-drift.py` exists to
/// catch on the TextMate side.
///
/// Two members carry a rule of their own, recorded here because the list
/// itself no longer has room for a note beside a name. `Link` is routing's
/// only element (§14G.2 revision 1): it renders a real anchor, which is
/// what makes every navigation crawlable and what leaves `set` out of
/// navigation entirely. `Prose` is the only element whose argument is
/// parsed as HTML, and it accepts `Markup` and nothing else (§16.3.5).
///
/// The names come from [`BuiltinElement::NAMES`] rather than being written
/// again here: one table, so a name this pass accepts and the HIR has no
/// variant for cannot exist.
pub const BUILTIN_ELEMENTS: &[&str] = BuiltinElement::NAMES;

/// The variant names every program can match, whatever it declares: the
/// ones `Option`, `Remote` and `Code` provide. A `choice` adds its own on
/// top and may not redeclare one of these (spec §14G.1.2).
pub fn builtin_patterns() -> Vec<&'static str> {
    builtin_variants()
}

/// The base type names the language provides.
///
/// `List`, `Map`, `Option` and `Remote` are absent on purpose: they are
/// constructors the lexer knows by name and the parser turns into
/// [`ast::TypeExpr`] variants, so they never reach a `Named` position.
///
/// Written out here rather than read off `zdc_types::Type`, which is the
/// one that decides what these names mean. Resolution runs before the
/// checker and does not link it, and inverting that edge to share a
/// seven-element list would be the larger change. Drift is a test failure
/// instead: `tests/builtin_contract.rs` asserts this is exactly
/// `Type::builtin_names()`, against the dev-dependency the crate already
/// has.
pub const BUILTIN_TYPES: &[&str] = &[
    "Text",
    "Markup",
    "Whole",
    "Decimal",
    "Truth",
    "Error",
    "Code",
    ast::HANDLE_TYPE_NAME,
];

/// Type names from other languages, and the ZDeceptron type each one is
/// almost always reaching for.
///
/// Levenshtein cannot find these — `Int` is four edits from `Whole` — and
/// they are the names everyone arriving from somewhere else types first.
/// A diagnostic that only said `Int` does not exist would leave the reader
/// to search for what does (§7.3).
///
/// Matched case-insensitively, so `int` and `string` are covered too.
const FOREIGN_TYPE_NAMES: &[(&str, &str)] = &[
    ("int", "`Whole`"),
    ("integer", "`Whole`"),
    ("long", "`Whole`"),
    // The one genuinely ambiguous name: JavaScript's `Number` is both.
    ("number", "`Whole` or `Decimal`"),
    ("float", "`Decimal`"),
    ("double", "`Decimal`"),
    ("string", "`Text`"),
    ("str", "`Text`"),
    ("bool", "`Truth`"),
    ("boolean", "`Truth`"),
];

/// Lowers a parsed program into HIR, resolving every identifier.
///
/// Resolution never stops at the first error. Each walk visits all of a
/// node's children before deciding whether the node itself resolved, so
/// a program with three undefined names yields three diagnostics from
/// one run. A node whose children did not all resolve is dropped from
/// the tree, which is harmless: the HIR is returned only when no error
/// was recorded at all.
pub struct Resolver<'a> {
    /// Every declaration to resolve, prelude first (§17.4.1).
    decls: Vec<&'a ast::Decl>,
    /// How many of `decls` came from the prelude.
    prelude: usize,
    hir: Hir,
    scopes: Scopes,
    globals: GlobalTable,
    errors: Vec<ResolveError>,
    /// The definition each declaration became, indexed by its position
    /// in `decls`.
    defs: Vec<DefId>,
    /// The module each declaration came from, and the one whose visible
    /// names the walk is currently using. Indexed by position in `decls`,
    /// so the prelude's own declarations sit in front of the program's.
    decl_module: Vec<usize>,
    module_count: usize,
    module: usize,
    imports: Vec<Vec<crate::modules::Import>>,
    /// The component being walked, so `children` can say whether it was
    /// declared and a `state` line can be attached to the right instance.
    component: Option<ComponentFrame>,
    /// How each callable is called and how many parameters it declares,
    /// read off the syntax before any body is walked.
    ///
    /// A body may call a function declared below it, so the answer cannot
    /// come from `DefKind`, which is still a placeholder at that point.
    /// This is the same reason `collect` is a pass of its own.
    signatures: HashMap<DefId, (ast::CallForm, usize)>,
    /// Every local a `with` statement introduced, with the span to report
    /// it at. A parameter or a loop name may go unread — the shape of the
    /// thing it binds is not the programmer's choice — but a binding is
    /// written for one purpose, so one that is never read is checked.
    bindings: Vec<(LocalId, String, zdc_lexer::Span)>,
    /// Every local that was read somewhere. Filled by `value_name`, which
    /// is the one place a local becomes a value.
    read: HashSet<LocalId>,
    /// What the project's `zd.toml` maps a bare module specifier to
    /// (#238). Empty for a resolver built from a source string, which has
    /// no project directory to read one from.
    packages: Packages,
}

/// What a component's body is allowed to name beyond the ordinary scopes.
struct ComponentFrame {
    name: String,
    children: Option<LocalId>,
    states: Vec<LocalSignal>,
    /// Whether the body has already placed `children`.
    ///
    /// Instantiation splices the call site's nodes in wherever `children`
    /// stands, and it splices the *same* nodes — the same binders, the same
    /// component state. A second `children` therefore emitted a second
    /// `const [open, setOpen] = signal(…)` for one instance's state, in one
    /// scope, which is not a bad rendering but a module that will not load.
    placed_children: bool,
}

impl<'a> Resolver<'a> {
    /// Resolve a program on its own, with no library beneath it.
    ///
    /// Used by tests that are about one pass rather than about a whole
    /// compilation; `with_prelude` is what the compiler calls.
    pub fn new(program: &'a ast::Program) -> Self {
        Resolver {
            decls: program.decls.iter().collect(),
            prelude: 0,
            hir: Hir::new(),
            scopes: Scopes::new(),
            globals: GlobalTable::default(),
            errors: Vec::new(),
            defs: Vec::new(),
            decl_module: Vec::new(),
            module_count: 1,
            module: 0,
            imports: vec![Vec::new()],
            component: None,
            signatures: HashMap::new(),
            bindings: Vec::new(),
            read: HashSet::new(),
            packages: Packages::none(std::path::Path::new("")),
        }
    }

    /// Resolve a program against the prelude, into one set of arenas.
    ///
    /// §17.4.1's phase 0. The library's declarations are allocated first,
    /// so a user reference to one is an ordinary `Res::Def` and every pass
    /// after this needs no rule at all for the fact that some definitions
    /// were not written by the programmer.
    pub fn with_prelude(prelude: &'a ast::Program, program: &'a ast::Program) -> Self {
        let mut decls: Vec<&'a ast::Decl> = prelude.decls.iter().collect();
        let count = decls.len();
        decls.extend(program.decls.iter());
        Resolver {
            decls,
            prelude: count,
            hir: Hir::new(),
            scopes: Scopes::new(),
            globals: GlobalTable::default(),
            errors: Vec::new(),
            defs: Vec::new(),
            decl_module: Vec::new(),
            module_count: 1,
            module: 0,
            imports: vec![Vec::new()],
            component: None,
            signatures: HashMap::new(),
            bindings: Vec::new(),
            read: HashSet::new(),
            packages: Packages::none(std::path::Path::new("")),
        }
    }

    /// Resolve a program linked from several files (spec §14D.2).
    ///
    /// Cross-module resolution has to happen here rather than later,
    /// because a `durable` signal may be declared in one file and read in
    /// another and the placement pass needs both ends (§14D.3).
    pub fn linked(linked: &'a Linked) -> Self {
        let mut resolver = Resolver::new(&linked.program);
        resolver.adopt_modules(linked);
        resolver
    }

    /// The two together: a linked program resolved against the prelude,
    /// which is what every entry point actually compiles.
    pub fn linked_with_prelude(prelude: &'a ast::Program, linked: &'a Linked) -> Self {
        let mut resolver = Resolver::with_prelude(prelude, &linked.program);
        resolver.adopt_modules(linked);
        resolver
    }

    /// Record which module each declaration came from.
    ///
    /// `linked.decl_module` is indexed by position in the linked program,
    /// and the prelude sits in front of that in `decls`, so every index
    /// shifts by the prelude's length. The prelude's own declarations are
    /// ambient rather than owned by any one module — `collect` makes them
    /// visible from all of them — so what they are numbered here does not
    /// decide what can see them.
    fn adopt_modules(&mut self, linked: &'a Linked) {
        let mut decl_module = vec![0usize; self.prelude];
        decl_module.extend(linked.decl_module.iter().copied());
        self.decl_module = decl_module;
        self.module_count = linked.modules.len();
        self.imports = linked.imports.clone();
        self.packages = linked.packages.clone();
    }

    /// The project's package mapping, for a caller that has one without
    /// having gone through [`crate::load`] (#238).
    ///
    /// The language server is that caller: it links a document only when
    /// the document has a `use`, so a single-file program arrives here as a
    /// parsed buffer with no mapping attached. Without this it would
    /// underline a `from "three"` that `zdc build` accepts — and the
    /// editor and the command line disagreeing about whether a program
    /// compiles is the failure this whole pipeline is arranged to prevent.
    pub fn with_packages(mut self, packages: Packages) -> Self {
        self.packages = packages;
        self
    }

    pub fn resolve(mut self) -> Result<Hir, Vec<ResolveError>> {
        // Cloned out so the walks below do not borrow `self`, which they
        // also need mutably.
        let decls = self.decls.clone();
        self.globals = if self.decl_module.is_empty() {
            collect(&decls, self.prelude)?
        } else {
            collect_linked(
                &decls,
                self.prelude,
                &self.decl_module,
                self.module_count,
                &self.imports,
            )?
        };

        // Every declaration gets its definition before any body is
        // looked at. This is what makes top-level declarations
        // order-independent: a signal may read one declared further down
        // the file, because the signal graph is a graph, not a sequence.
        for decl in &decls {
            let (name, span) = match decl {
                ast::Decl::State(state) => (state.name.text.clone(), state.name.span),
                ast::Decl::Function(function) => (function.name.text.clone(), function.name.span),
                ast::Decl::Foreign(foreign) => (foreign.name.text.clone(), foreign.name.span),
                ast::Decl::Record(record) => (record.name.text.clone(), record.name.span),
                ast::Decl::Choice(choice) => (choice.name.text.clone(), choice.name.span),
                ast::Decl::Component(component) => {
                    (component.name.text.clone(), component.name.span)
                }
                ast::Decl::Route(route) => (route.name.text.clone(), route.name.span),
                ast::Decl::Release(release) => (release.name.text.clone(), release.name.span),
                ast::Decl::Use(import) => ("use".to_string(), import.span),
                ast::Decl::View(view) => ("view".to_string(), view.span),
                // The definition's name is the claim, verbatim. A test is
                // registered in no scope (see `collect`), so nothing can
                // refer to it and nothing has to be able to spell it —
                // which frees the name to be the sentence a report prints
                // and a diagnostic quotes (issue #169).
                ast::Decl::Test(test) => (test.claim.clone(), test.claim_span),
            };
            let id = self.hir.defs.alloc(Def {
                name,
                span,
                kind: pending(),
            });
            match decl {
                ast::Decl::Function(function) => {
                    self.signatures
                        .insert(id, (function.form, function.params.len()));
                }
                ast::Decl::Foreign(foreign) => {
                    self.signatures
                        .insert(id, (foreign.form, foreign.params.len()));
                }
                // A release is called `f with a, b` and never `f of a`, so
                // its signature is a `With` one of its declared arity.
                ast::Decl::Release(release) => {
                    self.signatures
                        .insert(id, (ast::CallForm::With, release.params.len()));
                }
                _ => {}
            }
            self.defs.push(id);
        }
        self.hir.prelude_defs = self.prelude;

        for (index, decl) in decls.iter().enumerate() {
            if index == self.prelude {
                self.hir.prelude_exprs = self.hir.exprs.len();
                self.hir.prelude_locals = self.hir.locals.len();
            }
            self.module = self.decl_module.get(index).copied().unwrap_or(0);
            let kind = match decl {
                ast::Decl::State(state) => self.signal(state).map(DefKind::Signal),
                ast::Decl::Function(function) => self.function(function).map(DefKind::Function),
                ast::Decl::Foreign(foreign) => Some(DefKind::Foreign(self.foreign(foreign))),
                ast::Decl::Record(record) => Some(DefKind::Record(Record {
                    fields: self.fields(&record.name.text, &record.fields),
                })),
                ast::Decl::Choice(choice) => Some(DefKind::Choice(Choice {
                    variants: choice
                        .variants
                        .iter()
                        .map(|variant| Variant {
                            name: variant.name.text.clone(),
                            fields: self.fields(&variant.name.text, &variant.fields),
                            span: variant.span,
                        })
                        .collect(),
                })),
                ast::Decl::Component(component) => {
                    Some(DefKind::Component(self.component(component)))
                }
                ast::Decl::Route(route) => Some(DefKind::Choice(self.route(index, route))),
                ast::Decl::Release(release) => Some(DefKind::Release(self.release(release))),
                // Linking consumed the import; nothing is left to lower.
                ast::Decl::Use(_) => None,
                ast::Decl::View(view) => Some(DefKind::View(self.view(view))),
                ast::Decl::Test(test) => self.test(test).map(DefKind::Signal),
            };
            if let Some(kind) = kind {
                self.hir.defs[self.defs[index]].kind = kind;
            }
        }
        if self.prelude == decls.len() {
            self.hir.prelude_exprs = self.hir.exprs.len();
            self.hir.prelude_locals = self.hir.locals.len();
        }

        self.hir.view = self.globals.view.map(|index| self.defs[index]);
        self.check_bindings_are_read();

        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        // Every component instance is expanded here, before any later pass
        // runs. §14D.3: the signal graph must span component boundaries, so
        // tier splitting and information flow operate over the inlined
        // graph rather than per declaration.
        instantiate(&mut self.hir)?;
        Ok(self.hir)
    }

    // --- declarations ---

    /// Lower `test "…" expect e` to the `static Truth from e` it is —
    /// issue #169.
    ///
    /// # Why this is a lowering and not a new kind of definition
    ///
    /// A `DefKind::Test` would have to be ruled on at roughly sixty match
    /// sites across the placement pass, the type checker, the flow pass,
    /// the emitters and the language server — every one of which would be
    /// answering, for the first time, a question the `static` signal beside
    /// it already answers. Sixty new answers is sixty chances to give a
    /// different one, and the failure mode is not a compile error: it is a
    /// test that quietly stops being checked because some pass decided a
    /// `Test` was not worth walking.
    ///
    /// Lowering to a signal makes the whole existing pipeline load-bearing
    /// instead. In exchange for one field on [`Signal`], a test:
    ///
    /// * is resolved, so a claim about a deleted function does not compile;
    /// * is **typechecked against `Truth`**, by the same rule that checks
    ///   any other declared type against its initialiser — which is why
    ///   the type is synthesised here rather than written by the
    ///   programmer. `test "…" is Truth expect …` would be a word the
    ///   reader has to write and can only write one way;
    /// * is **placed**, at `static`, so what a claim may read is decided by
    ///   the pass that decides it for everything else. This is where the
    ///   scope limit comes from and it is not arbitrary: a `client` signal
    ///   is not readable from build time, so a claim about one is refused
    ///   with the existing placement diagnostic rather than by a rule
    ///   invented for tests;
    /// * pulls every function it calls into the build root, by the same
    ///   fixpoint that does it for `static` state — so the runner has a
    ///   module with the code in it and no separate reachability walk.
    ///
    /// The span of the whole `expect` clause rides along on the signal, and
    /// that single field is what the split and the runner read.
    fn test(&mut self, test: &ast::TestDecl) -> Option<Signal> {
        let init = self.expr(&test.expectation)?;
        Some(Signal {
            // A claim is a statement about the program, not a secret in it,
            // and it is nobody's authority: both lattices are at their
            // bottom, exactly as they are for a `state` written with
            // neither word.
            secret: false,
            trusted: false,
            placement: ast::Placement::Static,
            // Synthesised, and pointed at the expectation rather than at
            // the `test` line: if the expression is not a `Truth`, the
            // caret belongs under the expression that is the wrong type.
            ty: ast::TypeExpr::Named(ast::Ident {
                text: "Truth".to_string(),
                span: test.expectation_span,
            }),
            // Derived, never a source. `set` on a test would be a program
            // deciding its own claim, and the phrasing does not exist.
            is_source: false,
            init,
            emits: None,
            expectation: Some(test.expectation_span),
        })
    }

    fn signal(&mut self, state: &ast::StateDecl) -> Option<Signal> {
        // `starting` declares a source the program sets directly;
        // `from` declares a value the compiler recomputes (spec §4.5).
        let (is_source, expr) = match &state.init {
            ast::Init::Starting(expr) => (true, expr),
            ast::Init::From(expr) => (false, expr),
            // §14G.8 item 14 lands parser-first. Resolving this to nothing
            // would emit a program silently missing the effect it declares,
            // so the gap is named instead of hidden.
            ast::Init::Effect { .. } => {
                self.error(
                    format!(
                        "`{}` declares an effect with `takes`, and that construct is not \
                         implemented past the parser yet (§14G.8 item 14).",
                        state.name.text
                    ),
                    state.name.span,
                );
                return None;
            }
        };
        let init = self.expr(expr)?;
        self.type_visibility(&state.ty);
        Some(Signal {
            secret: state.secret,
            trusted: state.trusted,
            placement: state.placement,
            ty: state.ty.clone(),
            is_source,
            init,
            emits: state.emits.clone(),
            expectation: None,
        })
    }

    /// Whether the program declares a `route` at all, so `address` can say
    /// "add one" rather than "this is not a name".
    fn program_declares_a_route(&self) -> bool {
        self.decls
            .iter()
            .any(|decl| matches!(decl, ast::Decl::Route(_)))
    }

    /// A `route` declaration: an ordinary `choice` whose variants carry
    /// their parameters as named fields, plus the URL table.
    ///
    /// §14G.1.2 called this exactly right — route parameters *are* variant
    /// fields — so nothing downstream needs a second notion of a variant.
    /// `when page` binds `slug` because `BlogPost` declares a field named
    /// `slug`, and that is the whole mechanism.
    fn route(&mut self, index: usize, route: &ast::RouteDecl) -> Choice {
        let mut variants = Vec::with_capacity(route.variants.len());
        let mut infos = Vec::with_capacity(route.variants.len());

        for variant in &route.variants {
            let as_fields: Vec<ast::FieldDecl> = variant
                .params
                .iter()
                .map(|param| ast::FieldDecl {
                    name: param.name.clone(),
                    // A route parameter is never an identity key. It is a
                    // variant field, and `unique` exists so a list of rows
                    // reconciles by identity — a URL is not a row.
                    unique: false,
                    ty: param.ty.clone(),
                    span: param.span,
                })
                .collect();
            variants.push(Variant {
                name: variant.name.text.clone(),
                fields: self.fields(&variant.name.text, &as_fields),
                span: variant.span,
            });

            let mut params = Vec::with_capacity(variant.params.len());
            for param in &variant.params {
                let enumerated_in = match &param.enumerated_in {
                    Some(name) => match self.value_name(name) {
                        Some(Res::Def(def)) => Some(def),
                        Some(_) => {
                            self.error(
                                format!(
                                    "`{}` does not name a `state` declaration. The `in` of a \
                                     route parameter names a `static` signal holding every value \
                                     the parameter ranges over.",
                                    name.text
                                ),
                                name.span,
                            );
                            None
                        }
                        None => None,
                    },
                    None => None,
                };
                params.push(RouteParam {
                    name: param.name.text.clone(),
                    enumerated_in,
                    span: param.span,
                });
            }

            infos.push(RouteVariantInfo {
                path: variant.path.clone(),
                path_span: variant.path_span,
                params,
                span: variant.span,
            });
        }

        // One `route` per program, for the same reason there is one
        // `view`: `address` names the URL this document was served at, and
        // two route types would make that value's type ambiguous.
        if self.hir.routes.is_some() {
            self.error(
                "A program has one `route`, and this is the second one. Move these URLs into the \
                 first `route`."
                    .to_string(),
                route.span,
            );
        } else {
            self.hir.routes = Some((self.defs[index], RouteTable { variants: infos }));
        }

        Choice { variants }
    }

    /// The fields of a record or of a variant's payload, in declaration
    /// order.
    ///
    /// A repeated name is reported rather than kept: construction is by
    /// name (§14G.1.2), so a second `title` would give one field two values
    /// and elimination would still bind positionally to the first.
    fn fields(&mut self, owner: &str, fields: &[ast::FieldDecl]) -> Vec<Field> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out = Vec::with_capacity(fields.len());
        let mut identity: Option<&ast::Ident> = None;
        for field in fields {
            // `unique` parses ahead of the emitter (#2). Accepting it and
            // reconciling positionally anyway would be the worst of the
            // three options: the program would read as identity-keyed, run
            // as positional, and differ only in a cost nobody is watching.
            if field.unique {
                if let Some(first) = identity {
                    self.error(
                        format!(
                            "`{owner}` declares two identities, `{}` and `{}`. A row has one.",
                            first.text, field.name.text
                        ),
                        field.name.span,
                    );
                } else {
                    identity = Some(&field.name);
                    self.error(
                        format!(
                            "`{owner}` declares `{}` as its identity, and `unique` is not \
                             implemented past the parser yet (#2). Removing the word compiles, \
                             and reconciles by position.",
                            field.name.text
                        ),
                        field.name.span,
                    );
                }
            }
            if !seen.insert(field.name.text.as_str()) {
                self.error(
                    format!(
                        "`{owner}` declares `{}` twice. Each field of a record or a variant is \
                         named once, because a value is built by naming its fields.",
                        field.name.text
                    ),
                    field.name.span,
                );
            }
            self.type_visibility(&field.ty);
            out.push(Field {
                name: field.name.text.clone(),
                ty: field.ty.clone(),
                span: field.span,
            });
        }
        out
    }

    fn function(&mut self, function: &ast::FunctionDecl) -> Option<Function> {
        self.reject_operator_name(&function.name, function.form);
        self.scopes.push();
        let params = self.bind_all(&function.params);
        let body = self.block(&function.body);
        self.scopes.pop();
        Some(Function {
            form: function.form,
            params,
            body,
        })
    }

    /// Lower a `release` declaration.
    ///
    /// The body is an ordinary block in the parameters' scope, so every
    /// later pass walks it with the code it already has. What a release
    /// *adds* is three clauses, and each is resolved against the parameter
    /// list here so that the rules downstream read booleans rather than
    /// re-matching names.
    fn release(&mut self, release: &ast::ReleaseDecl) -> zdc_hir::Release {
        self.scopes.push();
        let params = self.bind_all(&release.params);
        let body = self.block(&release.body);
        self.scopes.pop();

        // E-REL-09: a `trusted` clause naming something that is not a
        // parameter of this release. Reported here rather than in the graph
        // because it is a name that resolves to nothing, which is exactly
        // what this pass is for.
        let mut endorsed = vec![false; release.params.len()];
        for clause in &release.endorsed {
            match release
                .params
                .iter()
                .position(|param| param.text == clause.text)
            {
                Some(index) => endorsed[index] = true,
                None => self.error(
                    format!(
                        "`trusted {}` names no parameter of `{}`. An endorsement grants a \
                         parameter of this release, and `{}` is not one (E-REL-09).",
                        clause.text, release.name.text, clause.text
                    ),
                    clause.span,
                ),
            }
        }

        self.type_visibility(&release.gives);
        zdc_hir::Release {
            params,
            gives: release.gives.clone(),
            endorsed,
            limit: release.limit.as_ref().map(|limit| zdc_hir::ReleaseBudget {
                count: limit.count,
                span: limit.span,
            }),
            body,
        }
    }

    fn foreign(&mut self, foreign: &ast::ForeignDecl) -> Foreign {
        self.reject_operator_name(&foreign.name, foreign.form);
        let target = self.foreign_module_target(foreign);
        self.check_foreign_view_site(foreign);
        self.check_foreign_handle_site(foreign);
        self.check_foreign_receiver(foreign);
        // A `foreign` has no body, so its parameter names exist only to be
        // written at a call site. They are still bound, because a call
        // matches `name is value` against them exactly as it does for an
        // ordinary function.
        self.scopes.push();
        let params = foreign
            .params
            .iter()
            .map(|param| self.bind(&param.name))
            .collect();
        self.scopes.pop();
        Foreign {
            site: foreign.site,
            source: foreign.source.clone(),
            target,
            export: foreign.export.clone(),
            form: foreign.form,
            params,
            param_types: foreign.params.iter().map(|p| p.ty.clone()).collect(),
            trusted_params: foreign.params.iter().map(|p| p.trusted).collect(),
            result_grant: foreign.result_grant,
            result: foreign.result.clone(),
        }
    }

    /// Where a `foreign`'s module specifier resolves, refusing the ones
    /// that resolve nowhere (#238).
    ///
    /// The export beside it is refused at parse time because it reaches
    /// the generated `import` as syntax. The module reaches it as a string
    /// literal, so escaping makes it well-formed — and well-formed is not
    /// the same as safe. That was the argument for refusing a URL outright,
    /// and it has been overturned deliberately, because the rule did not
    /// buy what it claimed: the alternative to a refused URL is a two-line
    /// `.js` file importing the same URL. The remote code arrives either
    /// way; refusing only moves it somewhere the compiler cannot see it,
    /// cannot report it in the manifest, and could never pin it. Written in
    /// the declaration it is visible to all three.
    ///
    /// So the specifiers that resolve are: a path, an `http:`/`https:` URL,
    /// the `zd:` layer, and a bare name the project mapped. A bare name it
    /// did not map is the one this whole pass exists for — it used to
    /// compile and then fail in the browser on the first import, and it is
    /// now a refusal that names the file and the line to add.
    ///
    /// `None` when there is no specifier at all: a method comes with its
    /// receiver — `scene.add(mesh)` names no module — so there is nothing
    /// here to resolve and nothing to refuse.
    fn foreign_module_target(&mut self, foreign: &ast::ForeignDecl) -> Option<ModuleTarget> {
        let ast::ForeignSource::Import {
            module,
            module_span,
        } = &foreign.source
        else {
            return None;
        };
        let (module, module_span) = (module.clone(), *module_span);
        Some(self.import_target(foreign, &module, module_span))
    }

    /// The specifier of a `foreign` that does import one, resolved.
    ///
    /// Split from [`Self::foreign_module_target`] so that every refusal
    /// below reads the specifier as a value rather than a field: it is one
    /// arm of [`ast::ForeignSource`] now, and threading it through is what
    /// keeps a method from silently acquiring an empty-string module.
    fn import_target(
        &mut self,
        foreign: &ast::ForeignDecl,
        module: &str,
        module_span: zdc_lexer::Span,
    ) -> ModuleTarget {
        if module.is_empty() {
            self.error(
                format!(
                    "`{}` names an empty module. Write the module a bundler can resolve, as in \
                     `from \"./sparkline.js\" as \"mount\"`.",
                    foreign.name.text
                ),
                module_span,
            );
            return ModuleTarget::AsWritten;
        }
        if let Some(reason) = module_specifier_refusal(module) {
            self.error(
                format!(
                    "`{}` imports from `{}`, and {reason} Write a path relative to this file, as \
                     in `from \"./sparkline.js\"`, a URL, as in `from \
                     \"https://esm.sh/marked@15.0.7\"`, or a package name `{}` maps, as in `from \
                     \"marked\"`.",
                    foreign.name.text,
                    module,
                    crate::packages::MANIFEST
                ),
                module_span,
            );
            return ModuleTarget::AsWritten;
        }
        if let Some(refusal) = self.escaping_path(module) {
            self.error(
                format!(
                    "`{}` imports from `{}`, which names a file that {refusal}. A relative \
                     specifier is copied out of the project directory into the bundle, so it is \
                     bounded by the rule `use` is: a build reads the project it is building and \
                     nothing else. Move the file under the project directory, or name it with a \
                     URL, as in `from \"https://esm.sh/marked@15.0.7\"`.",
                    foreign.name.text, module
                ),
                module_span,
            );
            return ModuleTarget::AsWritten;
        }
        if !is_bare_specifier(module) {
            return ModuleTarget::AsWritten;
        }
        self.mapped_target(foreign, module, module_span)
    }

    /// Whether a specifier names a file outside the project, if it names a
    /// file at all (#238).
    ///
    /// Only a relative specifier does. That is not a shortcut, it is
    /// [`crate::linked_module`]'s rule restated: `./` and `../` are exactly
    /// the forms the build copies out of the project directory, and every
    /// other form — a URL, a bare name, `zd:`, a root-absolute `/vendor/x`
    /// the browser resolves against the deployed site — names something no
    /// build-time path lookup is performed for.
    ///
    /// The rule itself is not restated: it is `zdc_hir::sandbox`, the same
    /// entry point `use` and the build-time capabilities go through, so
    /// this cannot drift from them and a symlink out of the project is
    /// caught here for the same reason it is there. `None` when this build
    /// has no project directory — a source string in memory opens no files,
    /// so there is nothing to bound.
    fn escaping_path(&self, specifier: &str) -> Option<&'static str> {
        if !specifier.starts_with("./") && !specifier.starts_with("../") {
            return None;
        }
        let root = self.packages.root()?;
        // Resolved the way `zdc build` resolves it before copying, so the
        // two cannot disagree about which file the specifier names.
        let target = root.join(specifier.trim_start_matches("./"));
        zdc_hir::sandbox::refuse(root, specifier, &target).map(|refusal| refusal.reason())
    }

    /// A bare specifier, looked up in the project's `[packages]` table.
    ///
    /// Split out because all three answers report against the same span
    /// and none of them may fall through: `Missing` and `Conflicting` are
    /// refusals, and the third is the only way a bare specifier resolves.
    fn mapped_target(
        &mut self,
        foreign: &ast::ForeignDecl,
        module: &str,
        module_span: zdc_lexer::Span,
    ) -> ModuleTarget {
        let manifest = self.packages.file().display().to_string();
        match self.packages.mapping(module) {
            Mapping::Mapped(target) => {
                let target = target.to_string();
                self.check_mapping_target(foreign, module, module_span, &target, &manifest);
                ModuleTarget::Mapped(target)
            }
            Mapping::Missing => {
                self.error(
                    format!(
                        "`{}` imports from `{}`, and a bare specifier names a package rather than \
                         a file, so nothing in this build resolves it: the browser would fail on \
                         that import before any of this program ran. Map it in `{manifest}`, \
                         under `[packages]`, as in `{} = \"https://esm.sh/{}@1.0.0\"`.",
                        foreign.name.text, module, module, module
                    ),
                    module_span,
                );
                ModuleTarget::AsWritten
            }
            Mapping::Conflicting { first, second } => {
                let (first, second) = (first.to_string(), second.to_string());
                self.error(
                    format!(
                        "`{}` imports from `{}`, and `{manifest}` maps `{}` twice — to `{first}` \
                         and to `{second}`. One specifier is one module, so the second line does \
                         not win: which of the two a page loads would depend on the order they \
                         were written in. Delete one, leaving the single line `{} = \
                         \"{first}\"`.",
                        foreign.name.text, module, module, module
                    ),
                    module_span,
                );
                ModuleTarget::AsWritten
            }
        }
    }

    /// A mapping's target has to resolve, for the same reasons a specifier
    /// does — and it is checked here, at the declaration that needed it,
    /// rather than when the file is read.
    ///
    /// A `zd.toml` may map packages a given program never imports, and a
    /// mistake in a line nothing uses is not this program's mistake. It
    /// also gives the diagnostic a span in a `.zd` file to point a caret
    /// at, which a line in a `.toml` has no way to supply.
    ///
    /// The containment rule is applied here as well as to a written
    /// specifier, and that is the point rather than a repetition: `zd.toml`
    /// is a second place a path can be written, so a mapping that was not
    /// bounded would be a way around the bound — `marked =
    /// "../../../../.ssh/id_rsa"` reaching a file `from "…"` could not.
    fn check_mapping_target(
        &mut self,
        foreign: &ast::ForeignDecl,
        module: &str,
        module_span: zdc_lexer::Span,
        target: &str,
        manifest: &str,
    ) {
        let refusal = match module_specifier_refusal(target) {
            Some(reason) => reason.to_string(),
            None if is_bare_specifier(target) => "a bare specifier names a package rather than a \
                                                  file, so mapping one to another leaves the \
                                                  same question unanswered."
                .to_string(),
            None => match self.escaping_path(target) {
                Some(refusal) => format!(
                    "it names a file that {refusal}, and a build reads the project it is building \
                     and nothing else."
                ),
                None => return,
            },
        };
        self.error(
            format!(
                "`{}` imports from `{}`, which `{manifest}` maps to `{target}`, and {refusal} \
                 Write a URL, as in `{} = \"https://esm.sh/{}@1.0.0\"`, or a path to a copy this \
                 build ships, as in `{} = \"./assets/{}.js\"`.",
                foreign.name.text, module, module, module, module, module
            ),
            module_span,
        );
    }

    /// A `gives view` foreign owns a DOM node, so it is `client` or it is
    /// nothing.
    ///
    /// A server function has no `document` to own a node in, and neither
    /// has the build host — §14E.2's overturn made `foreign` runtime-only
    /// precisely because the compiler is the host there. `is anywhere`
    /// fails for the same reason as `is server`: "anywhere" includes the
    /// places with no DOM, so it is not a weaker claim than `is client`
    /// but a stronger and false one.
    fn check_foreign_view_site(&mut self, foreign: &ast::ForeignDecl) {
        if !foreign.owns_view() || foreign.site == ast::ForeignSite::Client {
            return;
        }
        self.error(
            format!(
                "`{}` is `{}` and gives a view. A foreign that gives a view owns a DOM node, so \
                 it can only be linked into the client bundle: a server function has no \
                 `document` to own a node in, and neither does the build host (spec §14E.2). \
                 Write `foreign {} is client`.",
                foreign.name.text,
                foreign.site.describe(),
                foreign.name.text
            ),
            foreign.site_span,
        );
    }

    /// A `foreign` that touches a `Handle` is `client` or it is nothing.
    ///
    /// **This is the load-bearing half of the handle's information-flow
    /// argument, and it is why it lives here rather than in a later pass.**
    /// §14E.3 row 1 lets a `secret` cross into a foreign only where the
    /// call sits in server context, and `zdc-graph`'s `E-IFC-13` implements
    /// that by obliging every argument of a `foreign … is client` to be
    /// Public. Pinning every handle-touching foreign to `is client`
    /// therefore means **no secret can ever reach a host object** — so
    /// nothing secret is in one to be read back out by a later call, which
    /// is the laundering hole an opaque value would otherwise open through
    /// the whole lattice.
    ///
    /// It is also simply true of the objects this exists for. A three.js
    /// `Scene`, a `WebGLRenderer` and a canvas context are browser things;
    /// a server root has no `document` and the build host has none either,
    /// which is the same reason `check_foreign_view_site` gives.
    fn check_foreign_handle_site(&mut self, foreign: &ast::ForeignDecl) {
        let result_ty = match &foreign.result {
            ast::ForeignResult::Value(ty) | ast::ForeignResult::New(ty) => Some(ty),
            // Neither writes a result type, so neither has one to mention
            // a handle in. A `gives nothing` method still reaches this
            // rule through its `takes` line, which is where its receiver
            // is written.
            ast::ForeignResult::View | ast::ForeignResult::Nothing => None,
        };
        let touches = foreign.params.iter().any(|p| p.ty.mentions_handle())
            || result_ty.is_some_and(ast::TypeExpr::mentions_handle);
        if !touches || foreign.site == ast::ForeignSite::Client {
            return;
        }
        self.error(
            format!(
                "`{}` is `{}` and mentions `{}`. A handle is a live object in the browser's \
                 memory, so a foreign that takes or gives one can only be `client` (spec \
                 §14E.2, §14E.3). Write `foreign {} is client`.",
                foreign.name.text,
                foreign.site.describe(),
                ast::HANDLE_TYPE_NAME,
                foreign.name.text
            ),
            foreign.site_span,
        );
    }

    /// A receiver is the first parameter, and a handle is the only thing
    /// that has one.
    ///
    /// `on Handle as "add"` says the symbol is looked up on an object at
    /// the call rather than imported, and `of Handle as "domElement"` says
    /// the same about a member that is read rather than called. The rules
    /// are one set because the question is one question, and each is
    /// refused separately, naming the one that failed.
    ///
    /// * **There is a receiver.** `takes` comes after the source line and
    ///   its first parameter is what the symbol is looked up on, so a
    ///   declaration with no parameters names nothing to look it up on.
    /// * **The receiver is a `Handle`.** Nothing else in the language has
    ///   members: `Text`, `Whole` and the rest are values the compiler
    ///   knows the whole of, and their operations are the prelude's.
    /// * **A property takes nothing else.** `x.p` has no argument list, so
    ///   a second parameter describes an emission that does not exist.
    /// * **It hands back a value.** `gives view` is called by the runtime
    ///   with a DOM node, which is an import's contract and not a
    ///   receiver's; `gives new` constructs, and neither a method nor a
    ///   property does either.
    fn check_foreign_receiver(&mut self, foreign: &ast::ForeignDecl) {
        let (leader, span) = match foreign.source {
            ast::ForeignSource::Receiver { span } => ("on", span),
            ast::ForeignSource::Property { span } => ("of", span),
            ast::ForeignSource::Import { .. } => return,
        };
        // The same three facts, said in the words of whichever form was
        // written. One rule, two vocabularies: a method is *called on* an
        // object and a property is *read off* one, and a diagnostic that
        // used the other form's verb would describe a construct the author
        // did not write.
        let (applied, member, has) = match foreign.source {
            ast::ForeignSource::Receiver { .. } => ("looked up on", "it is called on", "methods"),
            ast::ForeignSource::Property { .. } | ast::ForeignSource::Import { .. } => {
                ("read off", "it is read off", "properties")
            }
        };
        match foreign.params.first() {
            None => self.error(
                format!(
                    "`{}` is declared `{leader} {}` and takes nothing. The symbol is {applied} \
                     the first argument, so `takes` has to name at least the object {member} \
                     (spec §14E.1).",
                    foreign.name.text,
                    ast::HANDLE_TYPE_NAME
                ),
                span,
            ),
            Some(receiver) if !receiver.ty.is_bare_handle() => self.error(
                format!(
                    "`{}` is declared `{leader} {}`, so `{}` is what {member} — and only a \
                     handle has {has} a program can name. Write `takes {} is {}` first (spec \
                     §14E.1).",
                    foreign.name.text,
                    ast::HANDLE_TYPE_NAME,
                    receiver.name.text,
                    receiver.name.text,
                    ast::HANDLE_TYPE_NAME
                ),
                receiver.span,
            ),
            Some(_) => {}
        }
        // **A property read has nowhere to put a second argument.** `x.p`
        // is a member expression and not a call, so a declaration naming
        // two parameters is describing something the emission cannot
        // express — and emitting `x.p` while silently dropping the second
        // argument is exactly the silent acceptance §4.1 refuses.
        if foreign.is_property() {
            if let Some(extra) = foreign.params.get(1) {
                self.error(
                    format!(
                        "`{}` is declared `of {}` and takes {} arguments. A property is read, not \
                         called: `x.{}` has no argument list, so only the object it is read from \
                         can be named (spec §14E.1).",
                        foreign.name.text,
                        ast::HANDLE_TYPE_NAME,
                        foreign.params.len(),
                        foreign.export
                    ),
                    extra.span,
                );
            }
        }
        // `gives nothing` is **not** refused here, and that is the whole
        // of blocker 2: `scene.add(mesh)` is a method that hands back no
        // value, and refusing the combination would leave the commonest
        // shape in any host library unwritable.
        let refused = match &foreign.result {
            ast::ForeignResult::View => Some("view"),
            ast::ForeignResult::New(_) => Some("new"),
            ast::ForeignResult::Value(_) | ast::ForeignResult::Nothing => None,
        };
        if let Some(word) = refused {
            let done = match foreign.source {
                ast::ForeignSource::Receiver { .. } => "A method is called on",
                ast::ForeignSource::Property { .. } | ast::ForeignSource::Import { .. } => {
                    "A property is read off"
                }
            };
            self.error(
                format!(
                    "`{}` is declared `{leader} {}` and `gives {word}`. {done} an object that \
                     already exists, and neither form is (spec §14E.1).",
                    foreign.name.text,
                    ast::HANDLE_TYPE_NAME
                ),
                foreign.result_span,
            );
        }
    }

    /// `length` and `text` mean one thing wherever `of` follows them, so
    /// no declaration may take either name in the `of` form.
    fn reject_operator_name(&mut self, name: &ast::Ident, form: ast::CallForm) {
        if form != ast::CallForm::Of || !BUILTIN_OF_OPERATORS.contains(&name.text.as_str()) {
            return;
        }
        self.error(
            format!(
                "`{} of` is one of the operations the language provides, so it cannot be \
                 declared again: a program reading `{} of x` must always mean the same thing. \
                 Rename this one.",
                name.text, name.text
            ),
            name.span,
        );
    }

    fn view(&mut self, view: &ast::ViewDecl) -> View {
        let metadata = self.metadata(view);
        self.scopes.push();
        let nodes = self.nodes(&view.nodes);
        self.scopes.pop();
        View { metadata, nodes }
    }

    /// The document's metadata, reduced to the literals it has to be.
    ///
    /// `<title>` is written into `index.html` when the bundle is built, so
    /// there is no run time at which a computed one could be evaluated —
    /// and a title that silently never updated would be worse than one the
    /// compiler refuses.
    fn metadata(&mut self, view: &ast::ViewDecl) -> zdc_hir::Metadata {
        let mut metadata = zdc_hir::Metadata::default();
        for arg in &view.args {
            let ast::Arg::Named { name, value } = arg else {
                self.error(
                    "A `view` takes only named metadata: `view title is \"…\"`.".to_string(),
                    view.span,
                );
                continue;
            };
            let slot = match name.text.as_str() {
                "title" => &mut metadata.title,
                "description" => &mut metadata.description,
                "language" => &mut metadata.language,
                _ => {
                    self.error(
                        format!(
                            "A `view` has no `{}`. Its metadata is {}.",
                            name.text,
                            english_list(zdc_hir::VIEW_METADATA)
                        ),
                        name.span,
                    );
                    continue;
                }
            };
            let ast::Expr::Text { value, .. } = value else {
                self.error(
                    format!(
                        "`{}` is written into the document when the bundle is built, so it has \
                         to be text written here rather than a value computed later.",
                        name.text
                    ),
                    value.span(),
                );
                continue;
            };
            if slot.is_some() {
                self.error(format!("`{}` is given twice.", name.text), name.span);
                continue;
            }
            *slot = Some(value.clone());
        }
        metadata
    }

    /// A `component` declaration (spec §14D.1).
    ///
    /// Parameters and `children` bind exactly as a function's do, and the
    /// component's own `state` binds as a local rather than a definition:
    /// it belongs to one instance, and a definition belongs to the program.
    fn component(&mut self, component: &ast::ComponentDecl) -> Component {
        self.scopes.push();
        let params = self.bind_all(&component.params);
        let children = component.children.map(|span| {
            let id = self.hir.locals.alloc(Local {
                name: "children".to_string(),
                span,
            });
            self.scopes.declare("children", id);
            id
        });

        let outer = self.component.replace(ComponentFrame {
            name: component.name.text.clone(),
            children,
            states: Vec::new(),
            placed_children: false,
        });

        // The state lines bind before any node is walked, so a node may
        // read state written below it, exactly as a top-level signal may.
        let mut body_nodes: Vec<&ast::Node> = Vec::new();
        for item in &component.body {
            match item {
                ast::ComponentItem::State(state) => self.component_state(component, state),
                ast::ComponentItem::Node(node) => body_nodes.push(node),
            }
        }
        let body: Vec<HirNode> = body_nodes
            .into_iter()
            .filter_map(|node| self.node(node))
            .collect();

        let frame = self
            .component
            .take()
            .expect("the frame this walk pushed is still here");
        self.component = outer;
        self.scopes.pop();

        Component {
            params,
            children,
            states: frame.states,
            body,
        }
    }

    /// One `state` line inside a component.
    ///
    /// **Component-local state must be `client`-placed.** A component
    /// instance is a browser-side thing; `server` state is per invocation
    /// and `durable` state is shared, so neither has a per-instance
    /// meaning (§14D.1). Both are refused here by name.
    fn component_state(&mut self, owner: &ast::ComponentDecl, state: &ast::StateDecl) {
        let (is_source, expr) = match &state.init {
            ast::Init::Starting(expr) => (true, expr),
            ast::Init::From(expr) => (false, expr),
            // Doubly out of reach: the construct is unimplemented, and an
            // effect is server-placed while component-local state must be
            // `client` (§14D.1). The first refusal is the honest one.
            ast::Init::Effect { .. } => {
                self.error(
                    format!(
                        "`{}` declares an effect with `takes`, and that construct is not \
                         implemented past the parser yet (§14G.8 item 14).",
                        state.name.text
                    ),
                    state.name.span,
                );
                return;
            }
        };
        let init = self.expr(expr);

        if state.placement != ast::Placement::Client {
            let placement = state.placement.word();
            let why = match state.placement {
                ast::Placement::Server => {
                    "`server` state lives in one serverless invocation, so it is per request \
                     rather than per instance"
                }
                ast::Placement::Static => {
                    "`static` state is computed once at build time and inlined, so every instance \
                     would share the one value"
                }
                ast::Placement::Durable | ast::Placement::Client => {
                    "`durable` state is one value shared by every visitor, so it is not per \
                     instance either"
                }
                ast::Placement::Remembered => {
                    "a `remembered` value is one entry in the browser's store, keyed by the \
                     signal's name, so every instance would share the one entry"
                }
            };
            self.error(
                format!(
                    "`{}` is declared `{placement}` inside the component `{}`, and state inside a \
                     component belongs to one instance of it. {why}. Write `client` here, or \
                     declare the state at the top level and pass it in.",
                    state.name.text, owner.name.text
                ),
                state.span,
            );
        }
        if state.secret {
            self.error(
                format!(
                    "`{}` is declared `secret` inside the component `{}`. Only `server` and \
                     `durable` state may be secret, and state inside a component is `client`.",
                    state.name.text, owner.name.text
                ),
                state.span,
            );
        }
        if state.trusted {
            self.error(
                format!(
                    "`{}` is declared `trusted` inside the component `{}`. State inside a \
                     component is `client`, and a browser owns its own memory — there is no such \
                     thing as protecting a browser from itself (spec §18.1, E-INT-01).",
                    state.name.text, owner.name.text
                ),
                state.span,
            );
        }

        self.type_visibility(&state.ty);
        let local = self.hir.locals.alloc(Local {
            name: state.name.text.clone(),
            span: state.name.span,
        });
        self.scopes.declare(&state.name.text, local);

        let Some(init) = init else { return };
        if let Some(frame) = self.component.as_mut() {
            frame.states.push(LocalSignal {
                local,
                placement: state.placement,
                ty: state.ty.clone(),
                is_source,
                init,
                span: state.span,
            });
        }
    }

    // --- statements ---

    fn block(&mut self, block: &ast::Block) -> zdc_hir::BlockId {
        self.scopes.push();
        let stmts: Vec<HirStmt> = block
            .stmts
            .iter()
            .map(|stmt| self.stmt(stmt))
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect();
        self.scopes.pop();
        self.hir.blocks.alloc(HirBlock {
            stmts,
            span: block.span,
        })
    }

    fn stmt(&mut self, stmt: &ast::Stmt) -> Option<HirStmt> {
        Some(match stmt {
            ast::Stmt::Pipeline(clause) => HirStmt::Pipeline(self.pipeline(clause)?),
            ast::Stmt::Mutation(mutation) => HirStmt::Mutation(self.mutation(mutation)?),
            ast::Stmt::Give(expr) => HirStmt::Give(self.expr(expr)?),
            ast::Stmt::Bind(bind) => HirStmt::Bind(self.bind_stmt(bind)?),
            ast::Stmt::Do(effect) => HirStmt::Do(zdc_hir::HirDo {
                call: self.expr(&effect.call)?,
                span: effect.span,
            }),
            ast::Stmt::When(when) => {
                let scrutinee = self.expr(&when.scrutinee);
                let arms = all_or_none(when.arms.iter().map(|arm| self.arm(arm)).collect());
                HirStmt::When(HirWhen {
                    scrutinee: scrutinee?,
                    arms: arms?,
                    span: when.span,
                })
            }
            ast::Stmt::Each(each) => {
                // The sequence is resolved before the loop name is
                // bound, so `each item in item` iterates the outer
                // `item` rather than itself.
                let iter = self.expr(&each.iter);
                self.scopes.push();
                let var = self.bind(&each.var);
                let body = self.block(&each.body);
                self.scopes.pop();
                HirStmt::Each(HirEach {
                    var,
                    iter: iter?,
                    body,
                    span: each.span,
                })
            }
            ast::Stmt::If(conditional) => {
                let cond = self.expr(&conditional.cond);
                let then = self.block(&conditional.then);
                let otherwise = conditional
                    .otherwise
                    .as_ref()
                    .map(|block| self.block(block));
                HirStmt::If(HirIf {
                    cond: cond?,
                    then,
                    otherwise,
                    span: conditional.span,
                })
            }
        })
    }

    /// `with total is 0` — spec §17.4.10.
    ///
    /// Each value is resolved *before* its own name is declared, so `with
    /// total is total` names something else or nothing, never itself.
    /// Nothing is pushed: a binding is in scope from here to the end of
    /// the block it was written in, which is the block's own scope.
    fn bind_stmt(&mut self, bind: &ast::BindStmt) -> Option<HirBind> {
        let bindings = all_or_none(
            bind.bindings
                .iter()
                .map(|binding| {
                    let value = self.expr(&binding.value);
                    self.reject_shadow(&binding.name);
                    let local = self.bind(&binding.name);
                    self.bindings
                        .push((local, binding.name.text.clone(), binding.name.span));
                    Some(HirBinding {
                        local,
                        value: value?,
                        span: binding.span,
                    })
                })
                .collect(),
        );
        Some(HirBind {
            bindings: bindings?,
            span: bind.span,
        })
    }

    /// A binding may not take a name that already means something here.
    ///
    /// Shadowing is refused rather than allowed because ZDeceptron has no
    /// way to say which one you meant: §4.2 forbids sigils, so there is no
    /// qualified form, and the prelude is resolved into the same namespace
    /// (§17.4.1), so `with first is …` would quietly take a library name
    /// out of value position for the rest of the block. The programmer can
    /// always choose another name; nothing can recover the hidden one.
    fn reject_shadow(&mut self, ident: &ast::Ident) {
        let hides = if self.scopes.lookup(&ident.text).is_some() {
            "a name already bound here"
        } else if self.globals.lookup(&ident.text).is_some()
            || self.globals.variant(&ident.text).is_some()
            || BuiltinVariant::from_name(&ident.text).is_some()
        {
            "a top-level declaration"
        } else {
            return;
        };
        self.error(
            format!(
                "`{}` already names {hides}, and a binding may not hide it: there is no way to \
                 write the one it would cover. Choose a different name.",
                ident.text
            ),
            ident.span,
        );
    }

    /// A binding that is never read is reported once resolution has seen
    /// the whole program, because the statement that reads one may come
    /// after it.
    ///
    /// An error rather than a warning, and not only because this compiler
    /// has no warning channel: every ZDeceptron expression is pure, so an
    /// unread binding cannot have been written for an effect. It is dead
    /// in every case, which makes it a mistake in every case — the name
    /// was misspelled at the use, or the use was never written.
    fn check_bindings_are_read(&mut self) {
        let unread: Vec<(String, zdc_lexer::Span)> = self
            .bindings
            .iter()
            .filter(|(local, _, _)| {
                !self.read.contains(local) && !self.hir.is_prelude_local(*local)
            })
            .map(|(_, name, span)| (name.clone(), *span))
            .collect();
        for (name, span) in unread {
            self.error(
                format!(
                    "`{name}` is bound here and never read. A binding names a value for the \
                     statements after it; one nothing reads computes nothing. Remove it, or use \
                     it."
                ),
                span,
            );
        }
    }

    /// A pipeline clause's loop name is in scope for that clause's
    /// expression and nowhere else.
    fn pipeline(&mut self, clause: &ast::PipelineClause) -> Option<HirPipeline> {
        Some(match clause {
            ast::PipelineClause::From(expr) => HirPipeline::From(self.expr(expr)?),
            ast::PipelineClause::TakeFirst(expr) => HirPipeline::TakeFirst(self.expr(expr)?),
            ast::PipelineClause::Keep { var, cond } => {
                let (var, cond) = self.clause_binder(var, cond);
                HirPipeline::Keep { var, cond: cond? }
            }
            ast::PipelineClause::Sort { var, key } => {
                let (var, key) = self.clause_binder(var, key);
                HirPipeline::Sort { var, key: key? }
            }
            ast::PipelineClause::MapEach { var, to } => {
                let (var, to) = self.clause_binder(var, to);
                HirPipeline::MapEach { var, to: to? }
            }
        })
    }

    fn clause_binder(
        &mut self,
        var: &ast::Ident,
        expr: &ast::Expr,
    ) -> (LocalId, Option<zdc_hir::ExprId>) {
        self.scopes.push();
        let var = self.bind(var);
        let expr = self.expr(expr);
        self.scopes.pop();
        (var, expr)
    }

    fn mutation(&mut self, mutation: &ast::Mutation) -> Option<HirMutation> {
        Some(match mutation {
            ast::Mutation::Set { place, value } => {
                let place = self.place(place);
                let value = self.expr(value);
                HirMutation::Set {
                    place: place?,
                    value: value?,
                }
            }
            ast::Mutation::Add { value, place } => {
                let value = self.expr(value);
                let place = self.place(place);
                HirMutation::Add {
                    value: value?,
                    place: place?,
                }
            }
            ast::Mutation::Subtract { value, place } => {
                let value = self.expr(value);
                let place = self.place(place);
                HirMutation::Subtract {
                    value: value?,
                    place: place?,
                }
            }
            ast::Mutation::Append { value, place } => {
                let value = self.expr(value);
                let place = self.place(place);
                HirMutation::Append {
                    value: value?,
                    place: place?,
                }
            }
            ast::Mutation::Remove { value, place } => {
                let value = self.expr(value);
                let place = self.place(place);
                HirMutation::Remove {
                    value: value?,
                    place: place?,
                }
            }
        })
    }

    fn place(&mut self, place: &ast::Place) -> Option<HirPlace> {
        let base = self.value_name(&place.base);
        let path = all_or_none(
            place
                .path
                .iter()
                .map(|segment| match segment {
                    ast::PathSeg::Field(name) => Some(HirPathSeg::Field(name.text.clone())),
                    ast::PathSeg::Index(expr) => self.expr(expr).map(HirPathSeg::Index),
                })
                .collect(),
        );
        Some(HirPlace {
            id: self.hir.new_place(),
            base: base?,
            path: path?,
            span: place.span,
        })
    }

    fn arm(&mut self, arm: &ast::Arm) -> Option<HirArm> {
        let pattern_name = self.pattern_name(&arm.pattern.name);
        self.scopes.push();
        let bindings = self.bind_all(&arm.pattern.bindings);
        let body = match &arm.body {
            ast::ArmBody::Show(expr) => self.expr(expr).map(HirArmBody::Show),
            ast::ArmBody::Block(block) => Some(HirArmBody::Block(self.block(block))),
        };
        self.scopes.pop();
        Some(HirArm {
            pattern_name: pattern_name?,
            bindings,
            body: body?,
            span: arm.span,
        })
    }

    // --- view nodes ---

    fn nodes(&mut self, nodes: &[ast::Node]) -> Vec<HirNode> {
        nodes
            .iter()
            .map(|node| self.node(node))
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect()
    }

    fn node(&mut self, node: &ast::Node) -> Option<HirNode> {
        Some(match node {
            ast::Node::Element(element) => HirNode::Element(self.element(element)?),
            ast::Node::Handler(handler) => {
                // The payload binder scopes over the body and nothing else,
                // so it is pushed and popped exactly as `each`'s loop
                // variable is.
                self.scopes.push();
                let payload = handler.payload.as_ref().map(|name| self.bind(name));
                let body = self.block(&handler.body);
                self.scopes.pop();
                HirNode::Handler(HirHandler {
                    event: handler.event.text.clone(),
                    payload,
                    event_span: handler.event.span,
                    body,
                    span: handler.span,
                })
            }
            ast::Node::Each(each) => {
                let iter = self.expr(&each.iter);
                self.scopes.push();
                let var = self.bind(&each.var);
                let body = self.nodes(&each.body);
                self.scopes.pop();
                HirNode::Each(HirEachNode {
                    var,
                    iter: iter?,
                    body,
                    span: each.span,
                })
            }
            ast::Node::When(when) => {
                let scrutinee = self.expr(&when.scrutinee);
                let arms = all_or_none(when.arms.iter().map(|arm| self.node_arm(arm)).collect());
                HirNode::When(HirWhenNode {
                    scrutinee: scrutinee?,
                    arms: arms?,
                    span: when.span,
                })
            }
            ast::Node::If(conditional) => {
                let cond = self.expr(&conditional.cond);
                self.scopes.push();
                let then = self.nodes(&conditional.then);
                self.scopes.pop();
                let otherwise = conditional.otherwise.as_ref().map(|nodes| {
                    self.scopes.push();
                    let nodes = self.nodes(nodes);
                    self.scopes.pop();
                    nodes
                });
                HirNode::If(HirIfNode {
                    cond: cond?,
                    then,
                    otherwise,
                    span: conditional.span,
                })
            }
            ast::Node::Children(span) => {
                let Some(frame) = self.component.as_ref() else {
                    self.error(
                        "`children` names the nodes nested under a component at its call site, \
                         so it can only be written inside a `component`."
                            .to_string(),
                        *span,
                    );
                    return None;
                };
                if frame.children.is_none() {
                    let name = frame.name.clone();
                    self.error(
                        format!(
                            "`{name}` does not take `children`, so there are none to place here. \
                             Write `component {name} with children` to receive the nodes nested \
                             under it."
                        ),
                        *span,
                    );
                    return None;
                }
                if frame.placed_children {
                    let name = frame.name.clone();
                    self.error(
                        format!(
                            "`{name}` places `children` twice. The nodes nested at a call site are \
                             one run of nodes and are written once: placing them again would put a \
                             second copy of the same state and the same binders in the same scope."
                        ),
                        *span,
                    );
                    return None;
                }
                if let Some(frame) = self.component.as_mut() {
                    frame.placed_children = true;
                }
                HirNode::Children(*span)
            }
        })
    }

    fn element(&mut self, element: &ast::Element) -> Option<HirElement> {
        let res = self.element_name(&element.name);
        self.refuse_written_destination(element);
        let args = all_or_none(element.args.iter().map(|arg| self.arg(arg)).collect());
        let children = self.nodes(&element.children);
        Some(HirElement {
            name: element.name.text.clone(),
            res: res?,
            args: destination_as_href(&element.name.text, args?),
            children,
            span: element.span,
        })
    }

    /// `Link href is …` is refused, so the destination has one phrasing.
    ///
    /// The destination is written first — `Link Home`, `Link
    /// "https://example.com"` — and [`destination_as_href`] is what puts
    /// it under the name `href` in the HIR. Were the name also writable in
    /// the source there would be two phrasings for one construct, which
    /// §4.1 forbids by name.
    fn refuse_written_destination(&mut self, element: &ast::Element) {
        if element.name.text != DESTINATION_ELEMENT {
            return;
        }
        for arg in &element.args {
            let ast::Arg::Named { name, .. } = arg else {
                continue;
            };
            if name.text == DESTINATION_ARGUMENT {
                self.error(
                    format!(
                        "`{DESTINATION_ELEMENT}` takes where it goes as its first argument, not \
                         as `{DESTINATION_ARGUMENT} is …`. Write \
                         `{DESTINATION_ELEMENT} \"https://example.com\"`, or \
                         `{DESTINATION_ELEMENT} Home` for one of this program's own routes."
                    ),
                    name.span,
                );
            }
        }
    }

    fn node_arm(&mut self, arm: &ast::NodeArm) -> Option<HirNodeArm> {
        let pattern_name = self.pattern_name(&arm.pattern.name);
        self.scopes.push();
        let bindings = self.bind_all(&arm.pattern.bindings);
        let body = match &arm.body {
            ast::NodeArmBody::Show(element) => self
                .element(element)
                .map(|element| HirNodeArmBody::Show(Box::new(element))),
            ast::NodeArmBody::Nodes(nodes) => Some(HirNodeArmBody::Nodes(self.nodes(nodes))),
        };
        self.scopes.pop();
        Some(HirNodeArm {
            pattern_name: pattern_name?,
            bindings,
            body: body?,
            span: arm.span,
        })
    }

    // --- expressions ---

    fn expr(&mut self, expr: &ast::Expr) -> Option<ExprId> {
        let span = expr.span();
        let kind = match expr {
            ast::Expr::Number { value, .. } => HirExprKind::Number(*value),
            ast::Expr::Text { value, .. } => HirExprKind::Text(value.clone()),
            ast::Expr::Truth { value, .. } => HirExprKind::Truth(*value),
            ast::Expr::Empty { .. } => HirExprKind::Empty,
            ast::Expr::List { items, .. } => HirExprKind::List(all_or_none(
                items.iter().map(|item| self.expr(item)).collect(),
            )?),
            ast::Expr::Map { entries, .. } => HirExprKind::Map(all_or_none(
                entries
                    .iter()
                    .map(|(key, value)| {
                        // Both halves are visited before either is judged,
                        // so two undefined names in one entry are two
                        // diagnostics.
                        let key = self.expr(key);
                        let value = self.expr(value);
                        Some((key?, value?))
                    })
                    .collect(),
            )?),
            ast::Expr::Environment { key, .. } => HirExprKind::Environment(key.clone()),
            ast::Expr::Address { .. } => {
                if self.hir.routes.is_none() && !self.program_declares_a_route() {
                    self.error(
                        "`address` is the URL this document was served at, and it has a value \
                         only once a `route` says which URLs exist. Add a `route` declaration."
                            .to_string(),
                        span,
                    );
                    return None;
                }
                HirExprKind::Address
            }
            ast::Expr::Media { query, .. } => HirExprKind::Media(query.clone()),
            // §4.4 already specifies that a callable declaring no
            // parameters is written as a bare name, and nothing
            // implemented it. That is what makes `clock` work with no new
            // syntax (§17.4.2).
            ast::Expr::Var { name, .. } => match self.value_name(name)? {
                res @ Res::Def(def) if self.takes_no_arguments(def) => HirExprKind::Call {
                    callee: res,
                    args: Vec::new(),
                },
                res => HirExprKind::Ref(res),
            },
            ast::Expr::Build {
                capability,
                argument,
                ..
            } => {
                // The argument is visited whether or not the capability
                // name resolves, so a misspelt capability and an undefined
                // name inside it are two diagnostics rather than one.
                let argument = self.expr(argument);
                let found = zdc_hir::BuildCapability::from_name(&capability.text);
                if found.is_none() {
                    let known: Vec<&str> = zdc_hir::BuildCapability::ALL
                        .iter()
                        .map(|capability| capability.name())
                        .collect();
                    self.error(
                        format!(
                            "`build {}` is not a capability the compiler provides. A build has \
                             no host to import from — the compiler is the host — so the set is \
                             closed, and it is `{}`.",
                            capability.text,
                            known.join("`, `")
                        ),
                        capability.span,
                    );
                }
                HirExprKind::Build {
                    capability: found?,
                    argument: argument?,
                }
            }
            ast::Expr::Call { name, args, .. } => {
                let callee = self.callee_name(name);
                let args = all_or_none(args.iter().map(|arg| self.arg(arg)).collect());
                let callee = callee?;
                self.check_call_form(name, callee, ast::CallForm::With);
                HirExprKind::Call {
                    callee,
                    args: args?,
                }
            }
            ast::Expr::Of { name, operand, .. } => {
                let operand = self.expr(operand)?;
                match OperatorName::from_name(&name.text) {
                    Some(op) => HirExprKind::Operator { op, operand },
                    None => {
                        let callee = self.of_name(name)?;
                        self.check_call_form(name, callee, ast::CallForm::Of);
                        HirExprKind::OfCall { callee, operand }
                    }
                }
            }
            ast::Expr::Unary { op, operand, .. } => HirExprKind::Unary {
                op: *op,
                operand: self.expr(operand)?,
            },
            ast::Expr::Binary { op, lhs, rhs, .. } => {
                let lhs = self.expr(lhs);
                let rhs = self.expr(rhs);
                HirExprKind::Binary {
                    op: *op,
                    lhs: lhs?,
                    rhs: rhs?,
                }
            }
            ast::Expr::Field { base, name, .. } => HirExprKind::Field {
                base: self.expr(base)?,
                name: name.text.clone(),
            },
            ast::Expr::Index { base, index, .. } => {
                let base = self.expr(base);
                let index = self.expr(index);
                HirExprKind::Index {
                    base: base?,
                    index: index?,
                }
            }
            // Both halves are resolved before either `?` short-circuits,
            // so an unknown name in each is reported once rather than the
            // first hiding the second.
            ast::Expr::Append { item, list, .. } => {
                let item = self.expr(item);
                let list = self.expr(list);
                HirExprKind::Append {
                    item: item?,
                    list: list?,
                }
            }
            // All three operands, for the reason `append`'s two are: an
            // unknown name in each is reported once rather than the first
            // hiding the others.
            ast::Expr::Insert {
                key, value, table, ..
            } => {
                let key = self.expr(key);
                let value = self.expr(value);
                let table = self.expr(table);
                HirExprKind::Insert {
                    key: key?,
                    value: value?,
                    table: table?,
                }
            }
        };
        Some(self.hir.exprs.alloc(HirExpr { kind, span }))
    }

    fn arg(&mut self, arg: &ast::Arg) -> Option<HirArg> {
        Some(match arg {
            ast::Arg::Positional(expr) => HirArg::Positional(self.expr(expr)?),
            ast::Arg::Named { name, value } => HirArg::Named {
                name: name.text.clone(),
                value: self.expr(value)?,
            },
        })
    }

    // --- names ---

    /// A name used as a value: a local first, so an inner binding
    /// shadows a top-level one, then a top-level declaration.
    fn value_name(&mut self, ident: &ast::Ident) -> Option<Res> {
        if let Some(local) = self.scopes.lookup(&ident.text) {
            self.read.insert(local);
            return Some(Res::Local(local));
        }
        // A component is a run of view nodes, so it has no value form.
        // Asked of what this module can see, because being linked into the
        // same program is not the same as being visible (§14D.2).
        if let Some(index) = self.globals.lookup_in(self.module, &ident.text) {
            if self.is_component(index) {
                self.error(
                    format!(
                        "`{}` is a component, which is a run of view nodes rather than a value. \
                         Write it as an element instead, on a line of its own.",
                        ident.text
                    ),
                    ident.span,
                );
                return None;
            }
        }
        if let Some(res) = self.global_name(&ident.text) {
            return Some(res);
        }
        self.undefined(ident);
        None
    }

    /// A name used as a callee, written `name with …`.
    ///
    /// Locals are skipped: ZDeceptron has no first-class functions, so a
    /// local can never *be* the thing being called, and letting one hide a
    /// top-level name would make a library function stop working inside a
    /// loop that happened to bind its name. This is `element_name`'s
    /// existing rule applied to the other callable position (§17.4.1).
    fn callee_name(&mut self, ident: &ast::Ident) -> Option<Res> {
        if let Some(res) = self.global_name(&ident.text) {
            return Some(res);
        }
        if self.not_callable(ident, "with") {
            return None;
        }
        self.undefined(ident);
        None
    }

    /// Report a name that *is* in scope but cannot be the thing called.
    ///
    /// The skip above is deliberate and the message must not pretend
    /// otherwise. Without this, `apply of f` inside `function apply of f`
    /// is told that `f` is undefined — "Declare it with `function f of …`,
    /// or check the spelling" — and offered the nearest *unrelated* global
    /// as a suggestion. Every word of that is wrong: `f` is declared, it
    /// is spelled correctly, and the nearest global is not what was meant.
    ///
    /// The reader who hits this is trying to pass a function as an
    /// argument, which is the one thing the message needs to address.
    /// `infer.rs` already extends exactly this courtesy to the
    /// value-position case (`double` used as a value rather than called);
    /// this is the callee position getting the same answer.
    ///
    /// Returns whether it reported, so the caller can skip `undefined`.
    fn not_callable(&mut self, ident: &ast::Ident, form: &str) -> bool {
        if self.scopes.lookup(&ident.text).is_none() {
            return false;
        }
        self.error(
            format!(
                "`{}` is in scope here, but it names a value, and ZDeceptron has no \
                 first-class functions, so it cannot be the operation in `{} {form} …`. \
                 Only a top-level `function` can be called.",
                ident.text, ident.text
            ),
            ident.span,
        );
        true
    }

    /// A name used as a unary accessor, written `name of value`.
    ///
    /// Locals are skipped for the same reason they are in callee position,
    /// and it is what makes `text of n` safe in a scope that binds a local
    /// called `text` — which `guestbook.zd`'s `Ready with text` does.
    fn of_name(&mut self, ident: &ast::Ident) -> Option<Res> {
        if let Some(res) = self.global_name(&ident.text) {
            return Some(res);
        }
        if self.declared_elsewhere(ident) {
            return None;
        }
        if self.not_callable(ident, "of") {
            return None;
        }
        // `of` names an operation, and an operation is a `function`, so the
        // value-producing declarations are exactly the right set to search
        // (#150). This site had no suggestion on any path.
        let suggestion = match self.nearest_value(&ident.text) {
            Some(nearest) => format!(" Did you mean `{nearest}`?"),
            None => String::new(),
        };
        self.error(
            format!(
                "`{} of` is not an operation this program can perform.{suggestion} Declare it \
                 with `function {} of …`, or check the spelling.",
                ident.text, ident.text
            ),
            ident.span,
        );
        None
    }

    /// The top-level meaning of a name: a declaration this module can see,
    /// a declared variant, or one the language provides.
    fn global_name(&mut self, name: &str) -> Option<Res> {
        if let Some(index) = self.globals.lookup_in(self.module, name) {
            return Some(Res::Def(self.defs[index]));
        }
        // A variant name is a value (`All`) and a constructor (`Archived
        // with reason is …`) alike, so it is looked up here as well as in
        // pattern position.
        if let Some((index, at)) = self.globals.variant(name) {
            return Some(Res::Variant {
                choice: self.defs[index],
                index: at,
            });
        }
        // The pair's constructor. It is a name the language provides, so
        // it is looked up after everything the program declares: a program
        // with its own `record Pair` keeps it, exactly as one with its own
        // `component Input` keeps that.
        if name == "Pair" {
            return Some(Res::Builtin(Builtin::Pair));
        }
        // §17.4.2: the built-in variants were recognised in pattern
        // position only, so nothing could ever *return* an `Option`. A
        // library whose whole job is producing one needs to build it.
        BuiltinVariant::from_name(name).map(Res::BuiltinVariant)
    }

    /// Report a name the link contains but this module never imported.
    ///
    /// Separate from `undefined` because "you did not import it" and "it
    /// does not exist" are different mistakes with different fixes, and
    /// every name position wants to draw the distinction.
    fn declared_elsewhere(&mut self, ident: &ast::Ident) -> bool {
        if !self.globals.is_declared_elsewhere(self.module, &ident.text) {
            return false;
        }
        self.error(
            format!(
                "`{}` is declared in another file but this one does not import it. Add it to \
                 a `use` line: `use \"./that-file\" for {}`.",
                ident.text, ident.text
            ),
            ident.span,
        );
        true
    }

    fn undefined(&mut self, ident: &ast::Ident) {
        if self.declared_elsewhere(ident) {
            return;
        }
        // The program's own vocabulary first: a misspelling of a name this
        // file declared is likelier to be what the writer reached for than
        // a built-in variant one edit away, and until #150 it was the one
        // case that suggested nothing at all.
        //
        // A name one edit from a built-in variant is almost always that
        // variant: `error.code is Timout` is the mistake `code` became a
        // choice in order to catch, and naming `Timeout` here is what
        // turns catching it into fixing it (§7.3).
        let suggestion = match self.nearest_value(&ident.text) {
            Some(nearest) => format!(" Did you mean `{nearest}`?"),
            None => match nearest_variant(&ident.text) {
                Some(nearest) => format!(" Did you mean the variant `{nearest}`?"),
                None => String::new(),
            },
        };
        self.error(
            format!(
                "`{}` is not defined.{suggestion} Declare it with `state`, `function`, `record`, \
                 or `choice`, import it with `use`, or check the spelling.",
                ident.text
            ),
            ident.span,
        );
    }

    /// Whether a definition is a callable that declares no parameters, and
    /// so is written as a bare name (§4.4).
    fn takes_no_arguments(&self, def: DefId) -> bool {
        matches!(self.signatures.get(&def), Some((_, 0)))
    }

    /// §17.4.2: a callable answers to exactly one spelling, and the
    /// declaration chooses it. Saying which one is valid is the whole
    /// point — a message that only said "wrong" would leave the programmer
    /// guessing between two forms that both look reasonable.
    fn check_call_form(&mut self, ident: &ast::Ident, callee: Res, written: ast::CallForm) {
        let Res::Def(def) = callee else {
            return;
        };
        // A record is built by naming its fields, so `with` is the only
        // form it has.
        let declared = self
            .signatures
            .get(&def)
            .map(|(form, _)| *form)
            .unwrap_or(ast::CallForm::With);
        if declared == written {
            return;
        }
        let (valid, wrong) = match declared {
            ast::CallForm::Of => (
                format!("`{} of …`", ident.text),
                format!("`{} with …`", ident.text),
            ),
            ast::CallForm::With => (
                format!("`{} with …`", ident.text),
                format!("`{} of …`", ident.text),
            ),
        };
        self.error(
            format!("`{}` is written {valid}, not {wrong}.", ident.text),
            ident.span,
        );
    }

    /// A name used as a view element. Element position is not value
    /// position, so a local named `Row` does not hide the element `Row`.
    ///
    /// A component is looked up here and nowhere else, which is what makes
    /// `Row` and `VoteCard` indistinguishable at the call site (§14D.1):
    /// there is no privileged set of built-ins, only two tables consulted
    /// in one place.
    fn element_name(&mut self, ident: &ast::Ident) -> Option<Res> {
        if let Some(element) = BuiltinElement::from_name(&ident.text) {
            return Some(Res::Builtin(Builtin::Element(element)));
        }
        if let Some(index) = self.globals.lookup_in(self.module, &ident.text) {
            if self.is_component(index) || self.is_view_foreign(index) {
                return Some(Res::Def(self.defs[index]));
            }
            // A `foreign` that gives a value is named here to say so
            // precisely. It is a plausible mistake — the two declaration
            // forms differ in one clause — and "not a component" would
            // point at the wrong repair.
            if self.is_foreign(index) {
                self.error(
                    format!(
                        "`{}` is a `foreign` that gives a value, so it is called for a result \
                         rather than written as a view element. Only `gives view` owns a DOM \
                         node (spec §14E.1).",
                        ident.text
                    ),
                    ident.span,
                );
                return None;
            }
            self.error(
                format!(
                    "`{}` is declared, but not as a component, so it cannot be written as a view \
                     element. Declare it with `component`, or use a built-in element.",
                    ident.text
                ),
                ident.span,
            );
            return None;
        }
        if self.globals.is_declared_elsewhere(self.module, &ident.text) {
            self.error(
                format!(
                    "`{}` is declared in another file but this one does not import it. Add it to \
                     a `use` line: `use \"./that-file\" for {}`.",
                    ident.text, ident.text
                ),
                ident.span,
            );
            return None;
        }
        // Sixty-six built-ins is too many to list in a diagnostic, and a
        // list that long is read as noise rather than as help (§7.3). The
        // nearest name is what the writer almost always meant. The count in
        // the message is read off the table rather than written out, so
        // this sentence is the only part that can go stale — and it did,
        // silently, when the vocabulary went from thirty-six to sixty-six.
        let suggestion = match nearest_element(&ident.text) {
            Some(nearest) => format!(" Did you mean `{nearest}`?"),
            None => String::new(),
        };
        self.error(
            format!(
                "`{}` is not a view element.{suggestion} A view element is one of the {} built-ins \
                 or a `component` this file declares or imports.",
                ident.text,
                BUILTIN_ELEMENTS.len()
            ),
            ident.span,
        );
        None
    }

    /// Check that every name written in a type is a type, and one this
    /// module can see.
    ///
    /// A type name is a name, so this is the pass that owns it, for the
    /// same reason it owns `wholeOr` in value position. It used to check
    /// visibility alone and admit anything else as an opaque type, which
    /// meant a typo in a type name produced a *successful build*: `Map of
    /// Id to Int` reached the checker as two types it had never heard of
    /// and neither did it (#28). Visibility is still checked first,
    /// because "you did not import it" and "it does not exist" are
    /// different mistakes with different fixes (§14D.2).
    fn type_name(&mut self, name: &ast::Ident) {
        if self.globals.is_declared_elsewhere(self.module, &name.text) {
            self.error(
                format!(
                    "`{}` is declared in another file but this one does not import it. Add it \
                     to a `use` line: `use \"./that-file\" for {}`.",
                    name.text, name.text
                ),
                name.span,
            );
            return;
        }
        if BUILTIN_TYPES.contains(&name.text.as_str()) {
            return;
        }
        match self.globals.lookup_in(self.module, &name.text) {
            // A `record`, a `choice` and a `route` are what declares a
            // type. A `route` is a `choice` with URLs (§14G.2), so
            // `Option of Site` is an ordinary type and `examples/site.zd`
            // writes one.
            Some(index)
                if matches!(
                    self.decls.get(index),
                    Some(ast::Decl::Record(_) | ast::Decl::Choice(_) | ast::Decl::Route(_))
                ) => {}
            // Declared, but by something that declares no type. The fix is
            // a different one, so the sentence is.
            Some(_) => self.error(
                format!(
                    "`{}` is not a type. It is declared here, but only a `record`, a `choice` \
                     or a `route` declares a type.",
                    name.text
                ),
                name.span,
            ),
            None => {
                let suggestion = match self.nearest_type(&name.text) {
                    Some(nearest) => format!(" Did you mean {nearest}?"),
                    None => String::new(),
                };
                self.error(
                    format!(
                        "`{}` is not a type.{suggestion} A type is `{}`, or a `record` or \
                         `choice` this file declares or imports.",
                        name.text,
                        BUILTIN_TYPES.join("`, `")
                    ),
                    name.span,
                );
            }
        }
    }

    /// The type a written name is almost certainly reaching for.
    ///
    /// The table of names from other languages is consulted first: it is
    /// the only thing that can connect `Int` to `Whole`, and a programmer
    /// who wrote `Int` did not misspell anything.
    /// The nearest name the program declared that could hold a *value*.
    ///
    /// The sibling of [`Self::nearest_type`], over the declarations that
    /// produce values rather than types: `state`, `function` and
    /// `foreign`. A record or a choice is a type, and suggesting one where
    /// a value was written would answer a question nobody asked.
    ///
    /// Visibility is checked the same way, so a name declared in a module
    /// this one did not import is not offered — a suggestion the reader
    /// cannot act on is worse than none (#150).
    fn nearest_value(&self, written: &str) -> Option<String> {
        let declared: Vec<String> = self
            .decls
            .iter()
            .enumerate()
            .filter_map(|(index, decl)| match decl {
                ast::Decl::State(state) => Some((index, &state.name.text)),
                ast::Decl::Function(function) => Some((index, &function.name.text)),
                ast::Decl::Foreign(foreign) => Some((index, &foreign.name.text)),
                ast::Decl::Record(_)
                | ast::Decl::Choice(_)
                | ast::Decl::Route(_)
                | ast::Decl::Component(_)
                | ast::Decl::Release(_)
                | ast::Decl::Use(_)
                | ast::Decl::View(_)
                // A test declares no value: it is registered in no scope,
                // so it is never what a misspelt name meant.
                | ast::Decl::Test(_) => None,
            })
            .filter(|(index, name)| self.globals.lookup_in(self.module, name) == Some(*index))
            .map(|(_, name)| name.clone())
            .collect();
        nearest_of(written, &declared)
    }

    fn nearest_type(&self, written: &str) -> Option<String> {
        let folded = written.to_lowercase();
        if let Some((_, suggestion)) = FOREIGN_TYPE_NAMES
            .iter()
            .find(|(foreign, _)| *foreign == folded)
        {
            return Some((*suggestion).to_string());
        }
        if let Some(builtin) = nearest(written, BUILTIN_TYPES) {
            return Some(format!("`{builtin}`"));
        }
        let declared: Vec<String> = self
            .decls
            .iter()
            .enumerate()
            .filter_map(|(index, decl)| match decl {
                ast::Decl::Record(record) => Some((index, &record.name.text)),
                ast::Decl::Choice(choice) => Some((index, &choice.name.text)),
                ast::Decl::Route(route) => Some((index, &route.name.text)),
                ast::Decl::State(_)
                | ast::Decl::Function(_)
                | ast::Decl::Foreign(_)
                | ast::Decl::Component(_)
                | ast::Decl::Release(_)
                | ast::Decl::Use(_)
                | ast::Decl::View(_)
                | ast::Decl::Test(_) => None,
            })
            .filter(|(index, name)| self.globals.lookup_in(self.module, name) == Some(*index))
            .map(|(_, name)| name.clone())
            .collect();
        nearest_of(written, &declared).map(|name| format!("`{name}`"))
    }

    /// Every name a type expression writes, in the order they are written.
    fn type_visibility(&mut self, ty: &ast::TypeExpr) {
        match ty {
            ast::TypeExpr::Named(name) => self.type_name(name),
            ast::TypeExpr::List(inner)
            | ast::TypeExpr::Option(inner)
            | ast::TypeExpr::Remote(inner) => self.type_visibility(inner),
            ast::TypeExpr::Map(key, value) | ast::TypeExpr::Pair(key, value) => {
                self.type_visibility(key);
                self.type_visibility(value);
            }
        }
    }

    /// Whether a declaration is a `component`.
    ///
    /// Read off the syntax tree rather than off the definition: every
    /// declaration is allocated a definition before any body is walked, so
    /// a component declared below the one that uses it still holds the
    /// placeholder kind at the moment its name is looked up.
    fn is_component(&self, index: usize) -> bool {
        matches!(self.decls.get(index), Some(ast::Decl::Component(_)))
    }

    fn is_foreign(&self, index: usize) -> bool {
        matches!(self.decls.get(index), Some(ast::Decl::Foreign(_)))
    }

    /// Whether this declaration is a `foreign … gives view`.
    ///
    /// Such a foreign owns a DOM node and hands back no ZDeceptron value,
    /// so element position is the *only* position it can be written in —
    /// which is the mirror of `zdc-codegen` refusing it in expression
    /// position. One construct, one place to write it (§4.1).
    fn is_view_foreign(&self, index: usize) -> bool {
        matches!(
            self.decls.get(index),
            Some(ast::Decl::Foreign(foreign)) if foreign.owns_view()
        )
    }

    /// The variant a `when` arm matches. Which choice it belongs to is a
    /// question for the type checker, so only the name is checked here.
    fn pattern_name(&mut self, ident: &ast::Ident) -> Option<String> {
        if is_builtin_variant(&ident.text) || self.globals.declares_variant(&ident.text) {
            return Some(ident.text.clone());
        }
        // §4.1 puts the whole weight of a rigid grammar on the
        // diagnostic naming the form that was meant. `Timout` is one
        // edit from `Timeout`, and an arm list is short enough that the
        // nearest name is almost always the one intended.
        let suggestion = match nearest_variant(&ident.text) {
            Some(nearest) => format!(" Did you mean `{nearest}`?"),
            None => String::new(),
        };
        self.error(
            format!(
                "`{}` is not a variant name.{suggestion} A `when` arm matches {}, or a variant a \
                 `choice` in this file declares.",
                ident.text,
                english_list(&builtin_patterns())
            ),
            ident.span,
        );
        None
    }

    /// Bind a run of names in the current scope: a function's
    /// parameters, or a pattern's binders.
    ///
    /// A pattern binds one fresh name per named field of the variant it
    /// matches (spec §14G.1.2), so this is a list in both cases. Two
    /// fields bound to the same name would make one of them
    /// unreachable, so that is reported rather than silently shadowed.
    fn bind_all(&mut self, idents: &[ast::Ident]) -> Vec<LocalId> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut ids = Vec::with_capacity(idents.len());
        for ident in idents {
            if !seen.insert(ident.text.as_str()) {
                self.error(
                    format!(
                        "`{}` is bound twice here. Each name after `with` must be different, or \
                         one of them can never be read.",
                        ident.text
                    ),
                    ident.span,
                );
            }
            ids.push(self.bind(ident));
        }
        ids
    }

    fn bind(&mut self, ident: &ast::Ident) -> LocalId {
        let id = self.hir.locals.alloc(Local {
            name: ident.text.clone(),
            span: ident.span,
        });
        self.scopes.declare(&ident.text, id);
        id
    }

    fn error(&mut self, message: String, span: zdc_lexer::Span) {
        self.errors.push(ResolveError {
            message,
            span,
            label: None,
            suggestion: None,
            code: None,
        });
    }
}

/// The kind a definition holds between being allocated and being
/// resolved.
///
/// Definitions are allocated for the whole program before any body is
/// walked, so that a body can refer to one declared later. An empty view
/// is the placeholder because it owns no arena index: if a bug ever left
/// one un-overwritten, the result is an empty view rather than a
/// definition pointing at an arena slot that means something else.
fn pending() -> DefKind {
    DefKind::View(View {
        metadata: zdc_hir::Metadata::default(),
        nodes: Vec::new(),
    })
}

/// The `zd:` prefix names the language's own primitive layer (§17.4.10).
///
/// It is not a scheme in the URL sense at all: nothing resolves it over a
/// network, `zdc-codegen`'s intrinsic table answers it in process, and a
/// program cannot add to that table. It is exempted by name rather than by
/// pattern so that widening the exemption is a visible edit.
const PRIMITIVE_MODULE_PREFIX: &str = "zd:";

/// The two schemes that name a module a browser will fetch (#238).
///
/// Written out rather than matched loosely, so that adding a third is a
/// visible edit against a list, exactly as `zd:` is. `http:` is here
/// beside `https:` because a page served over `http:` — a dev server on
/// localhost, which is what `zdc dev` is — can only load modules over it,
/// and refusing it would make the compiler's rule depend on a deployment
/// the compiler cannot see. A page served over `https:` has the browser's
/// own mixed-content rule on its side, which is a better enforcement than
/// this one would be.
const FETCHABLE_SCHEMES: [&str; 2] = ["http", "https"];

/// Whether this specifier names a package rather than a module — the form
/// that resolves only through the project's `[packages]` mapping (#238).
///
/// The browser's own definition: not a URL, and not beginning `/`, `./` or
/// `../`. `zd:` is excluded because the language answers it in process, so
/// it is not something a project maps.
fn is_bare_specifier(module: &str) -> bool {
    !module.starts_with('/')
        && !module.starts_with("./")
        && !module.starts_with("../")
        && !module.starts_with(PRIMITIVE_MODULE_PREFIX)
        && url_scheme(module).is_none()
}

/// The `scheme:` this specifier carries, per RFC 3986 — which is what a
/// browser's module resolver treats as an absolute URL.
fn url_scheme(module: &str) -> Option<&str> {
    module
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .filter(|scheme| {
            let mut chars = scheme.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphabetic())
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        })
}

/// Why this module specifier may not be written, if it may not be.
///
/// Four forms resolve: a path, an `http:`/`https:` URL, the `zd:` layer,
/// and a bare package name the project maps. Every other scheme is refused
/// by name, because none of them names a place a module is fetched from —
/// `data:` is the code itself rather than a location, `file:` is a path on
/// whichever machine ran the build, and `npm:` is a registry no browser
/// resolves. A specifier whose target has no origin cannot be reported in
/// the manifest, which is the guarantee the URL form was allowed to keep.
fn module_specifier_refusal(module: &str) -> Option<&'static str> {
    if module
        .chars()
        .any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}')
    {
        return Some(
            "a module specifier may not contain a control character: it is written into a \
             generated `import` and read back by tools that treat those as commands.",
        );
    }
    if module.starts_with("//") {
        return Some(
            "a specifier beginning `//` names another host, so the browser would load and run \
             code this build never saw.",
        );
    }
    if module.starts_with(PRIMITIVE_MODULE_PREFIX) {
        return None;
    }
    let scheme = url_scheme(module)?;
    if FETCHABLE_SCHEMES.contains(&scheme) {
        // A remote origin, allowed deliberately (#238). It runs with this
        // page's origin and that is the maintainer's call to accept: the
        // rule that refused it did not stop the code arriving, it stopped
        // the compiler seeing it arrive.
        return None;
    }
    Some(
        "a specifier carrying that URL scheme names no place a module is fetched from — a `data:` \
         document is the code itself, `file:` is a path on whichever machine ran the build, and a \
         registry scheme is one no browser resolves — so nothing can report where the page loads \
         it from.",
    )
}

/// Combine per-item results only after every item has been visited.
///
/// Collecting straight into `Option<Vec<_>>` would stop at the first
/// `None` and hide the errors in everything after it, which is exactly
/// the behaviour resolution must not have.
fn all_or_none<T>(resolved: Vec<Option<T>>) -> Option<Vec<T>> {
    resolved.into_iter().collect()
}

/// `a`, `b`, and `c` — for listing the valid names in a diagnostic.
/// The built-in whose name is closest to `written`, if one is close enough
/// to be worth naming.
///
/// Closeness is Levenshtein distance, case-folded, with the threshold at a
/// third of the written name's length. `Paragrph` suggests `Paragraph`;
/// `Widget` suggests nothing, because suggesting a name at random is worse
/// than suggesting none.
fn nearest_element(written: &str) -> Option<&'static str> {
    nearest(written, BUILTIN_ELEMENTS)
}

/// The built-in variant whose name is closest to `written`.
///
/// This is what makes a misspelled arm or a misspelled `Code` value a
/// diagnostic that names the intended one: `Timout` suggests `Timeout`.
/// Before `code` became a choice there was nothing to suggest, because
/// `error.code is "Timout"` was a well-typed comparison of two `Text`s
/// that answered `no` for ever.
fn nearest_variant(written: &str) -> Option<&'static str> {
    nearest(written, &builtin_patterns())
}

/// The same, over names the program declared rather than names the
/// language provides, so a misspelled `record` suggests the record.
fn nearest_of(written: &str, candidates: &[String]) -> Option<String> {
    let borrowed: Vec<&str> = candidates.iter().map(String::as_str).collect();
    nearest(written, &borrowed).map(str::to_string)
}

/// The candidate closest to `written`, if one is close enough to be worth
/// naming.
fn nearest<'c>(written: &str, candidates: &[&'c str]) -> Option<&'c str> {
    let budget = (written.chars().count() / 3).max(1);
    let mut best: Option<(usize, &'c str)> = None;
    for candidate in candidates {
        let distance = edit_distance(&written.to_lowercase(), &candidate.to_lowercase());
        if distance > budget {
            continue;
        }
        if best.is_none_or(|(shortest, _)| distance < shortest) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, name)| name)
}

/// Levenshtein distance, two rows at a time.
fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];

    for (row, left_char) in left.chars().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let substitution = usize::from(left_char != *right_char);
            current[column + 1] = (previous[column] + substitution)
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn english_list(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{}, and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hir_of(src: &str) -> Result<Hir, Vec<ResolveError>> {
        let program = zdc_parser::parse(src).expect("parses");
        Resolver::new(&program).resolve()
    }

    fn errors_of(src: &str) -> Vec<String> {
        hir_of(src)
            .expect_err("expected resolution to fail")
            .into_iter()
            .map(|error| error.message)
            .collect()
    }

    /// `unique` parses ahead of the emitter (#2), and is refused rather
    /// than ignored.
    ///
    /// Accepting it and reconciling positionally anyway is the one option
    /// that cannot be defended: the program reads as identity-keyed, runs
    /// as positional, and differs only in a cost nobody is watching.
    #[test]
    fn a_unique_field_is_refused_until_the_emitter_can_key_on_it() {
        let errors = errors_of("record Todo\n    unique id is Whole\n    title is Text\n");
        assert_eq!(errors.len(), 1, "one refusal, not a cascade: {errors:?}");
        assert!(
            errors[0].contains("not implemented"),
            "the message says the word is unbuilt, not that the record is wrong: {}",
            errors[0]
        );
    }

    /// §14G.8 item 14 (#211) parses ahead of the rest of the pipeline.
    ///
    /// It is refused here rather than resolved, because a construct that
    /// parsed and then silently resolved to nothing would emit a program
    /// missing the effect it declared. Refusing names the gap; the
    /// alternative hides it in the output.
    #[test]
    fn an_effect_declaration_is_refused_until_the_rest_of_it_lands() {
        let errors = errors_of(
            "state signUp is server Remote of Outcome takes form is Draft\n    give Accepted\n",
        );
        assert_eq!(errors.len(), 1, "one refusal, not a cascade: {errors:?}");
        assert!(
            errors[0].contains("not implemented"),
            "the message has to say the construct is unbuilt, not that the program is wrong: {}",
            errors[0]
        );
    }

    /// A row has one identity. Caught before the unimplemented-word
    /// refusal would fire twice and read as a cascade.
    #[test]
    fn a_record_cannot_declare_two_identities() {
        let errors = errors_of("record Todo\n    unique id is Whole\n    unique slug is Text\n");
        assert!(
            errors.iter().any(|e| e.contains("two identities")),
            "the second `unique` is named as the defect it is: {errors:?}"
        );
    }

    /// #150. A misspelling of a name the *program* declared suggested
    /// nothing: `undefined` searched only `builtin_patterns()`, so
    /// `Timout` found `Timeout` and a typo'd `state` found silence.
    #[test]
    fn an_unknown_value_name_suggests_the_nearest_declared_one() {
        let errors =
            errors_of("state wholeOrr is client Whole starting 0\n\nview\n    Text wholeOrr2\n");
        assert!(
            errors.iter().any(|e| e.contains("`wholeOrr`")),
            "the declared name is one edit away and in scope: {errors:?}"
        );
    }

    /// The same for a function, which is the other value-producing
    /// declaration a reader is likely to misspell.
    #[test]
    fn an_unknown_name_suggests_a_declared_function() {
        let errors = errors_of(
            "function politeGreeting with who\n    give who\n\n\
             state out is client Text from politeGreting with who is \"a\"\n\nview\n    Text out\n",
        );
        assert!(
            errors.iter().any(|e| e.contains("`politeGreeting`")),
            "got: {errors:?}"
        );
    }

    /// `name of value` had no suggestion on any path.
    #[test]
    fn an_unknown_of_accessor_suggests_the_nearest_declared_one() {
        let errors = errors_of(
            "function loudly of body\n    give body\n\n\
             state out is client Text from loudy of \"a\"\n\nview\n    Text out\n",
        );
        assert!(
            errors.iter().any(|e| e.contains("`loudly`")),
            "got: {errors:?}"
        );
    }

    /// Suggesting a name at random is worse than suggesting none, which is
    /// the threshold `nearest` already holds for types and elements.
    #[test]
    fn a_far_miss_suggests_nothing() {
        let errors = errors_of(
            "state wholeOrr is client Whole starting 0\n\nview\n    Text totallyUnrelated\n",
        );
        assert!(
            errors.iter().any(|e| e.contains("not defined")),
            "it is still refused: {errors:?}"
        );
        assert!(
            !errors.iter().any(|e| e.contains("Did you mean")),
            "nothing is close enough to name: {errors:?}"
        );
    }

    /// §17.4.10's binding, in scope for the statements after it.
    #[test]
    fn a_binding_is_in_scope_for_the_rest_of_its_block() {
        let hir = hir_of("function f\n    with total is 1\n    give total\n").expect("resolves");
        let DefKind::Function(function) =
            &hir.defs[hir.defs.iter().next().expect("a definition").0].kind
        else {
            panic!("expected a function")
        };
        let HirStmt::Bind(bind) = &hir.blocks[function.body].stmts[0] else {
            panic!("expected a binding")
        };
        let HirStmt::Give(give) = &hir.blocks[function.body].stmts[1] else {
            panic!("expected a give")
        };
        assert_eq!(
            hir.exprs[*give].kind,
            HirExprKind::Ref(Res::Local(bind.bindings[0].local))
        );
    }

    /// Each value is resolved before its own name exists, so a binding
    /// cannot name itself — which would otherwise be a value defined in
    /// terms of nothing.
    #[test]
    fn a_binding_may_not_name_itself() {
        let errors = errors_of("function f\n    with total is total\n    give total\n");
        assert!(
            errors.iter().any(|e| e.contains("`total` is not defined")),
            "{errors:?}"
        );
    }

    /// Shadowing is refused rather than allowed: §4.2 leaves no qualified
    /// form, so the hidden name could never be written again.
    #[test]
    fn a_binding_may_not_shadow_a_parameter() {
        let errors = errors_of("function f with total\n    with total is 1\n    give total\n");
        assert!(
            errors.iter().any(|e| e.contains("already names a name")),
            "{errors:?}"
        );
    }

    #[test]
    fn a_binding_may_not_shadow_a_top_level_declaration() {
        let errors = errors_of(
            "state count is client Whole starting 0\n\
             function f\n    with count is 1\n    give count\n",
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("already names a top-level declaration")),
            "{errors:?}"
        );
    }

    #[test]
    fn two_bindings_on_one_line_may_not_share_a_name() {
        let errors = errors_of("function f\n    with a is 1, a is 2\n    give a\n");
        assert!(
            errors.iter().any(|e| e.contains("already names a name")),
            "{errors:?}"
        );
    }

    /// Every ZDeceptron expression is pure, so a binding nothing reads
    /// computes nothing. There is no case where one is wanted.
    #[test]
    fn a_binding_that_is_never_read_is_an_error() {
        let errors = errors_of("function f\n    with total is 1\n    give 0\n");
        assert!(
            errors.iter().any(|e| e.contains("never read")),
            "{errors:?}"
        );
    }

    /// A read from a later statement counts, which is why the check waits
    /// until the whole program has been walked.
    #[test]
    fn a_binding_read_only_from_a_nested_block_is_read() {
        hir_of("function f with flag\n    with total is 1\n    if flag\n        give total\n    give 0\n")
            .expect("resolves");
    }

    /// A later binding on the same line sees the earlier ones.
    #[test]
    fn one_with_may_bind_a_name_from_a_name_it_just_bound() {
        hir_of("function f\n    with a is 1, b is a + 1\n    give b\n").expect("resolves");
    }

    #[test]
    fn a_signal_reading_another_signal_resolves_to_a_def() {
        let hir = hir_of("state a is client Whole starting 1\nstate b is client Whole from a\n")
            .expect("resolves");
        assert_eq!(hir.defs.len(), 2);
    }

    /// The behaviour the collection pass exists for: a declaration may
    /// read one written below it.
    #[test]
    fn forward_reference_resolves() {
        let hir = hir_of("state b is client Whole from a\nstate a is client Whole starting 1\n")
            .expect("forward references are legal");
        assert_eq!(hir.defs.len(), 2);
    }

    #[test]
    fn a_reference_points_at_the_definition_it_names() {
        let hir = hir_of("state b is client Whole from a\nstate a is client Whole starting 1\n")
            .expect("resolves");
        let DefKind::Signal(b) = &hir.defs[hir.defs.iter().next().expect("a definition").0].kind
        else {
            panic!("expected a signal")
        };
        let HirExprKind::Ref(Res::Def(target)) = hir.exprs[b.init].kind else {
            panic!("expected a resolved reference")
        };
        assert_eq!(hir.defs[target].name, "a");
    }

    #[test]
    fn an_undefined_name_is_reported_with_its_span() {
        let errors = hir_of("state a is client Whole from missing\n").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("missing"),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("not"),
            "the message should say the name was not found: {}",
            errors[0].message
        );
        let start = "state a is client Whole from ".len() as u32;
        assert_eq!(errors[0].span, zdc_lexer::Span::new(start, start + 7));
    }

    #[test]
    fn a_function_parameter_is_in_scope_in_its_body() {
        hir_of("function double with n\n    give n + n\n").expect("resolves");
    }

    #[test]
    fn a_parameter_is_not_in_scope_outside_its_function() {
        let errors =
            hir_of("function f with n\n    give n\nstate a is client Whole from n\n").unwrap_err();
        assert_eq!(errors.len(), 1, "n must not leak out of f");
    }

    #[test]
    fn a_loop_variable_is_scoped_to_its_clause() {
        hir_of("function f with xs\n    from xs\n    keep each item where item.live\n")
            .expect("resolves");
    }

    #[test]
    fn a_loop_variable_does_not_escape_its_clause() {
        let errors = hir_of(
            "function f with xs\n    from xs\n    keep each item where item.live\n    give item\n",
        )
        .unwrap_err();
        assert_eq!(errors.len(), 1, "item must not outlive its clause");
    }

    #[test]
    fn every_pipeline_clause_binds_its_own_loop_name() {
        hir_of(
            "function f with xs\n\
             \x20   from xs\n\
             \x20   keep each a where a.live\n\
             \x20   sort each b by b.rank\n\
             \x20   map each c to c.name\n\
             \x20   take first 5\n",
        )
        .expect("resolves");
    }

    #[test]
    fn an_inner_binding_shadows_an_outer_one() {
        hir_of("function f with item\n    each item in item\n        give item\n")
            .expect("shadowing is legal");
    }

    #[test]
    fn a_loop_sequence_is_resolved_before_the_loop_name_is_bound() {
        // Without that ordering `each item in item` would iterate
        // itself, and this program would resolve with `f` having no
        // parameter at all.
        let errors = hir_of("function f\n    each item in item\n        give item\n").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("item"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn every_error_is_reported_not_just_the_first() {
        let errors =
            hir_of("state a is client Whole from nope\nstate b is client Whole from alsonope\n")
                .unwrap_err();
        assert_eq!(
            errors.len(),
            2,
            "resolution must not stop at the first error"
        );
    }

    /// Both sides of a binary expression are visited before the
    /// expression is judged, so two mistakes in one line are two
    /// diagnostics.
    #[test]
    fn both_halves_of_one_expression_are_reported() {
        let errors = hir_of("state a is client Whole from nope + alsonope\n").unwrap_err();
        assert_eq!(errors.len(), 2);
    }

    /// The same rule across a list: every argument is visited before the
    /// call is judged.
    #[test]
    fn every_bad_argument_in_one_call_is_reported() {
        let errors = errors_of("view\n    Text nope, other is alsonope\n");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn a_when_arm_binds_one_name_per_named_field() {
        let hir = hir_of(
            "state entry is client Whole starting 1\n\
             function f\n\
             \x20   when entry\n\
             \x20       Failed with why, moment\n\
             \x20           give why\n",
        )
        .expect("resolves");

        let DefKind::Function(f) = &hir.defs[hir.defs.iter().nth(1).expect("a function").0].kind
        else {
            panic!("expected a function")
        };
        let HirStmt::When(when) = &hir.blocks[f.body].stmts[0] else {
            panic!("expected a when statement")
        };
        assert_eq!(when.arms[0].bindings.len(), 2);
        assert_eq!(hir.locals[when.arms[0].bindings[1]].name, "moment");
    }

    #[test]
    fn an_arm_binding_does_not_escape_its_arm() {
        let errors = hir_of(
            "state entry is client Whole starting 1\n\
             function f\n\
             \x20   when entry\n\
             \x20       Ready with value\n\
             \x20           give value\n\
             \x20   give value\n",
        )
        .unwrap_err();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn a_pattern_may_not_bind_one_name_twice() {
        let errors = errors_of(
            "state entry is client Whole starting 1\n\
             function f\n\
             \x20   when entry\n\
             \x20       Failed with why, why\n\
             \x20           give why\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bound twice"), "got: {}", errors[0]);
    }

    #[test]
    fn a_function_may_not_bind_one_parameter_twice() {
        let errors = errors_of("function f with a, a\n    give a\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bound twice"), "got: {}", errors[0]);
    }

    #[test]
    fn an_unknown_variant_name_names_the_ones_that_exist() {
        let errors = errors_of(
            "state entry is client Whole starting 1\n\
             view\n\
             \x20   when entry\n\
             \x20       Loadng show Spinner\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`Loading`"), "got: {}", errors[0]);
    }

    #[test]
    fn an_unknown_element_name_names_the_ones_that_exist() {
        let errors = errors_of("view\n    Colunm\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`Column`"), "got: {}", errors[0]);
    }

    /// A view element name is not a value name, so an element is looked
    /// up among the elements even where a local of the same name exists.
    #[test]
    fn a_local_does_not_hide_an_element_of_the_same_name() {
        hir_of(
            "state names is client Whole starting 1\n\
             view\n\
             \x20   each Text in names\n\
             \x20       Text Text\n",
        )
        .expect("resolves");
    }

    #[test]
    fn a_loop_name_in_a_view_reaches_the_handlers_inside_it() {
        hir_of(
            "state votes is durable Whole starting 0\n\
             state items is client Whole starting 0\n\
             view\n\
             \x20   each item in items\n\
             \x20       Row item.name\n\
             \x20           on click\n\
             \x20               add 1 to votes at item.id\n",
        )
        .expect("resolves");
    }

    #[test]
    fn a_view_loop_name_does_not_escape_the_loop() {
        let errors = errors_of(
            "state items is client Whole starting 0\n\
             view\n\
             \x20   each item in items\n\
             \x20       Text item\n\
             \x20   Text item\n",
        );
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn the_view_is_recorded_on_the_program() {
        let hir = hir_of("view\n    Column\n").expect("resolves");
        let view = hir.view.expect("a view");
        assert!(matches!(hir.defs[view].kind, DefKind::View(_)));
    }

    #[test]
    fn a_program_without_a_view_has_none() {
        let hir = hir_of("state a is client Whole starting 1\n").expect("resolves");
        assert_eq!(hir.view, None);
    }

    #[test]
    fn a_signal_records_whether_it_is_a_source() {
        let hir = hir_of("state a is client Whole starting 1\nstate b is client Whole from a\n")
            .expect("resolves");
        let kinds: Vec<bool> = hir
            .defs
            .iter()
            .map(|(_, def)| match &def.kind {
                DefKind::Signal(signal) => signal.is_source,
                other @ (DefKind::Function(_)
                | DefKind::View(_)
                | DefKind::Record(_)
                | DefKind::Choice(_)
                | DefKind::Component(_)
                | DefKind::Foreign(_)
                | DefKind::Release(_)) => panic!("expected a signal, got {other:?}"),
            })
            .collect();
        assert_eq!(kinds, [true, false]);
    }

    #[test]
    fn a_collection_error_is_returned_before_any_body_is_walked() {
        let errors = errors_of(
            "state a is client Whole starting 1\n\
             state a is client Whole starting 2\n\
             state b is client Whole from nope\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("already declared"), "got: {}", errors[0]);
    }

    // --- record and choice declarations (spec §14B.1, §14G.1.2) ---

    #[test]
    fn a_record_becomes_a_definition_with_its_fields_in_order() {
        let hir = hir_of("record Todo\n    id is Whole\n    title is Text\n").expect("resolves");
        let (_, def) = hir.defs.iter().next().expect("a definition");
        let DefKind::Record(record) = &def.kind else {
            panic!("expected a record, got {:?}", def.kind)
        };
        assert_eq!(def.name, "Todo");
        let names: Vec<&str> = record
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        assert_eq!(names, ["id", "title"]);
    }

    /// A payload-free variant is a value, so a bare name resolves to it.
    #[test]
    fn a_variant_name_resolves_to_its_choice_and_position() {
        let hir = hir_of(
            "choice Status\n\
             \x20   Active\n\
             \x20   Archived with reason is Text\n\
             state s is client Status starting Archived with reason is \"old\"\n",
        )
        .expect("resolves");
        let (_, def) = hir
            .defs
            .iter()
            .find(|(_, def)| def.name == "s")
            .expect("the signal");
        let DefKind::Signal(signal) = &def.kind else {
            panic!("expected a signal")
        };
        let HirExprKind::Call {
            callee: Res::Variant { index, .. },
            ..
        } = hir.exprs[signal.init].kind
        else {
            panic!(
                "expected a variant construction, got {:?}",
                hir.exprs[signal.init].kind
            )
        };
        assert_eq!(index, 1, "`Archived` is the second variant");
    }

    /// A `when` arm may name a variant a `choice` in this file declared.
    #[test]
    fn a_when_arm_may_match_a_declared_variant() {
        hir_of(
            "choice Status\n\
             \x20   Active\n\
             \x20   Archived with reason is Text\n\
             state s is client Status starting Active\n\
             function f\n\
             \x20   when s\n\
             \x20       Active show 1\n\
             \x20       Archived with why show 2\n",
        )
        .expect("resolves");
    }

    #[test]
    fn two_choices_may_not_declare_the_same_variant() {
        let errors = errors_of("choice A\n    Same\nchoice B\n    Same\n");
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("already a variant"),
            "got: {}",
            errors[0]
        );
    }

    /// `when` matches by name, so a program-declared `Ready` would make a
    /// `Remote` arm mean two things (§14G.1.2).
    #[test]
    fn a_choice_may_not_redeclare_a_builtin_variant() {
        let errors = errors_of("choice Fetch\n    Ready\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`Option`"), "got: {}", errors[0]);
    }

    /// Construction is by name, so a field named twice would give one field
    /// two values.
    #[test]
    fn a_record_may_not_declare_one_field_twice() {
        let errors = errors_of("record Todo\n    id is Whole\n    id is Text\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("twice"), "got: {}", errors[0]);
    }

    /// A record shares the top-level namespace with signals and functions,
    /// because `Todo with …` is spelled exactly like a call.
    #[test]
    fn a_record_may_not_share_a_name_with_a_signal() {
        let errors =
            errors_of("state Todo is client Whole starting 1\nrecord Todo\n    id is Whole\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("already declared"), "got: {}", errors[0]);
    }

    #[test]
    fn a_collection_literal_resolves_every_element() {
        let errors = errors_of("state xs is client List of Whole starting [nope, alsonope]\n");
        assert_eq!(errors.len(), 2, "every element is visited: {errors:?}");
    }

    #[test]
    fn a_map_literal_resolves_both_halves_of_every_entry() {
        let errors =
            errors_of("state m is client Map of Whole to Whole starting [nope to alsonope]\n");
        assert_eq!(errors.len(), 2, "{errors:?}");
    }

    #[test]
    fn no_message_names_a_rust_type() {
        let sources = [
            "state a is client Whole from nope\n",
            "view\n    Colunm\n",
            "state a is client Whole starting 1\nview\n    when a\n        Nope show Spinner\n",
            "function f with a, a\n    give a\n",
            "choice A\n    Same\nchoice B\n    Same\n",
            "record Todo\n    id is Whole\n    id is Text\n",
            "choice Fetch\n    Ready\n",
        ];
        let forbidden = ["Ident", "TokenKind", "Expr", "DefId", "LocalId", "HirExpr"];

        // Every source has to *be* rejected, or the loop below reads no
        // messages and the test says nothing about any of them. A source
        // the resolver starts accepting is a change to make deliberately,
        // not one to discover from a coverage report.
        let mut inspected = 0;
        for src in sources {
            let messages = errors_of(src);
            assert!(!messages.is_empty(), "{src:?} is no longer rejected");
            for message in messages {
                inspected += 1;
                for needle in forbidden {
                    assert!(
                        !message.contains(needle),
                        "message for {src:?} leaked `{needle}`: {message}"
                    );
                }
            }
        }
        assert!(inspected >= sources.len(), "read {inspected} messages");
    }

    // --- components (spec §14D.1) ---

    /// The symmetry §14D.1 exists to state: a component call site is an
    /// element, resolved by the same lookup a built-in goes through.
    #[test]
    fn a_component_is_used_exactly_where_a_built_in_element_is() {
        hir_of(
            "component Card with title\n\
             \x20   Row\n\
             \x20       Text title\n\
             view\n\
             \x20   Column\n\
             \x20       Card \"hello\"\n",
        )
        .expect("resolves");
    }

    /// Instantiation puts the body where the call site was, so the view
    /// holds the component's nodes and nothing that names the component.
    #[test]
    fn a_call_site_becomes_the_components_body() {
        let hir = hir_of(
            "component Card with title\n\
             \x20   Row\n\
             \x20       Text title\n\
             view\n\
             \x20   Column\n\
             \x20       Card \"hello\"\n",
        )
        .expect("resolves");
        let view = hir.view.expect("a view");
        let DefKind::View(view) = &hir.defs[view].kind else {
            panic!("expected a view")
        };
        let HirNode::Element(column) = &view.nodes[0] else {
            panic!("expected the column")
        };
        let HirNode::Element(row) = &column.children[0] else {
            panic!(
                "expected the component's own Row, got {:?}",
                column.children
            )
        };
        assert_eq!(row.name, "Row");
    }

    /// Two instances of one component have two of everything: §14D.1's
    /// per-instance state depends on it, and so does emitting two
    /// distinct names for them.
    #[test]
    fn two_instances_bind_separate_locals() {
        let hir = hir_of(
            "component Box with label\n\
             \x20   state open is client Truth starting no\n\
             \x20   Text label\n\
             view\n\
             \x20   Column\n\
             \x20       Box \"a\"\n\
             \x20       Box \"b\"\n",
        )
        .expect("resolves");
        let view = hir.view.expect("a view");
        let DefKind::View(view) = &hir.defs[view].kind else {
            panic!("expected a view")
        };
        let HirNode::Element(column) = &view.nodes[0] else {
            panic!("expected the column")
        };
        let scopes: Vec<LocalId> = column
            .children
            .iter()
            .filter_map(|node| match node {
                HirNode::Scope(scope) => Some(scope.locals[0].local),
                // Written out rather than wildcarded: a new node kind that
                // can also own component state has to be considered here,
                // not silently skipped into a passing count.
                HirNode::Element(_)
                | HirNode::Each(_)
                | HirNode::When(_)
                | HirNode::If(_)
                | HirNode::Handler(_)
                | HirNode::Children(_) => None,
            })
            .collect();
        assert_eq!(scopes.len(), 2, "one scope per instance");
        assert_ne!(scopes[0], scopes[1], "each instance owns its own state");
    }

    /// §14D.1: component state must be `client`. `server` state is per
    /// invocation and `durable` state is shared, so neither has a
    /// per-instance meaning.
    #[test]
    fn component_state_may_not_be_server_or_durable() {
        for placement in ["server", "durable"] {
            let errors = errors_of(&format!(
                "component Box with label\n\
                 \x20   state seen is {placement} Whole starting 0\n\
                 \x20   Text label\n\
                 view\n\
                 \x20   Box \"a\"\n"
            ));
            assert_eq!(errors.len(), 1, "{placement}: {errors:?}");
            assert!(errors[0].contains(placement), "got: {}", errors[0]);
            assert!(errors[0].contains("instance"), "got: {}", errors[0]);
            assert!(errors[0].contains("client"), "got: {}", errors[0]);
        }
    }

    #[test]
    fn component_state_may_not_be_secret() {
        let errors = errors_of(
            "component Box with label\n\
             \x20   secret state key is client Text starting \"\"\n\
             \x20   Text label\n\
             view\n\
             \x20   Box \"a\"\n",
        );
        assert!(
            errors.iter().any(|message| message.contains("secret")),
            "got: {errors:?}"
        );
    }

    #[test]
    fn component_state_may_not_be_trusted() {
        let errors = errors_of(
            "component Box with label\n\
             \x20   trusted state role is client Text starting \"\"\n\
             \x20   Text label\n\
             view\n\
             \x20   Box \"a\"\n",
        );
        assert!(
            errors.iter().any(|message| message.contains("E-INT-01")),
            "got: {errors:?}"
        );
    }

    #[test]
    fn children_outside_a_component_is_reported() {
        let errors = errors_of("view\n    Column\n        children\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`component`"), "got: {}", errors[0]);
    }

    #[test]
    fn a_component_that_did_not_declare_children_cannot_place_them() {
        let errors = errors_of(
            "component Box with label\n\
             \x20   Column\n\
             \x20       children\n\
             view\n\
             \x20   Box \"a\"\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("with children"), "got: {}", errors[0]);
    }

    /// Instantiation splices the call site's nodes in wherever `children`
    /// stands, and it splices the same nodes — the same binders and the
    /// same component state, not a copy of them. A body that placed
    /// `children` twice therefore emitted one instance's
    /// `const [n, setN] = signal(0)` twice into one scope, which is not a
    /// bad rendering but a module the engine refuses to load.
    #[test]
    fn a_component_may_place_its_children_only_once() {
        let errors = errors_of(
            "component Box with children\n\
             \x20   Column\n\
             \x20       children\n\
             \x20       children\n\
             component Inner\n\
             \x20   state n is client Whole starting 0\n\
             \x20   Text n\n\
             view\n\
             \x20   Box\n\
             \x20       Inner\n",
        );
        assert!(
            errors.iter().any(|message| message.contains("twice")),
            "got: {errors:?}"
        );
    }

    #[test]
    fn placing_children_once_is_still_fine() {
        hir_of(
            "component Box with children\n\
             \x20   Column\n\
             \x20       children\n\
             view\n\
             \x20   Box\n\
             \x20       Text \"a\"\n",
        )
        .expect("one `children` is what a component is for");
    }

    #[test]
    fn nesting_nodes_under_a_component_that_takes_none_is_reported() {
        let errors = errors_of(
            "component Box with label\n\
             \x20   Text label\n\
             view\n\
             \x20   Box \"a\"\n\
             \x20       Text \"orphan\"\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("children"), "got: {}", errors[0]);
    }

    /// A component is written where it is used, so one that contains
    /// itself describes a view with no end. The diagnostic names the whole
    /// path rather than only the component it noticed.
    #[test]
    fn a_component_cycle_names_every_component_on_the_path() {
        let errors = errors_of(
            "component A with x\n\
             \x20   B x\n\
             component B with x\n\
             \x20   A x\n\
             view\n\
             \x20   A 1\n",
        );
        assert_eq!(errors.len(), 1);
        // The whole path, in order, and not merely "some `A` appears".
        // This asserted `contains("`A`") || contains('A')`, whose second
        // arm is implied by the first and holds for any message mentioning
        // the component at all — so the disjunction could not fail while
        // the path was what the test was named for.
        assert!(
            errors[0].contains("A → B → A"),
            "the message must name the whole cycle, got: {}",
            errors[0]
        );
    }

    #[test]
    fn a_component_is_not_a_value() {
        let errors = errors_of(
            "component Box with label\n\
             \x20   Text label\n\
             state a is client Whole from Box\n\
             view\n\
             \x20   Column\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("component"), "got: {}", errors[0]);
    }

    #[test]
    fn a_missing_argument_names_the_parameter() {
        let errors = errors_of(
            "component Box with label, tone\n\
             \x20   Text label\n\
             view\n\
             \x20   Box \"a\"\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`tone`"), "got: {}", errors[0]);
    }

    #[test]
    fn an_unknown_named_argument_names_the_parameters_that_exist() {
        let errors = errors_of(
            "component Box with label\n\
             \x20   Text label\n\
             view\n\
             \x20   Box tone is \"loud\"\n",
        );
        assert!(
            errors.iter().any(|message| message.contains("`label`")),
            "got: {errors:?}"
        );
    }

    /// §14D.1's `VoteCard` writes through a parameter, so the argument has
    /// to be something a write can reach.
    #[test]
    fn writing_through_a_parameter_needs_a_name_at_the_call_site() {
        hir_of(
            "component Tick with total\n\
             \x20   Button \"more\"\n\
             \x20       on click\n\
             \x20           add 1 to total\n\
             state count is client Whole starting 0\n\
             view\n\
             \x20   Tick count\n",
        )
        .expect("a `state` name is writable");

        let errors = errors_of(
            "component Tick with total\n\
             \x20   Button \"more\"\n\
             \x20       on click\n\
             \x20           add 1 to total\n\
             state count is client Whole starting 0\n\
             view\n\
             \x20   Tick count + 1\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("written to"), "got: {}", errors[0]);
    }

    /// Placement is a property of state, never of code (§5.1). A component
    /// handed a `durable` signal does not become server-placed; the read
    /// simply appears where the call site is, and the type checker decides
    /// what it yields there.
    #[test]
    fn a_component_handed_durable_state_stays_colorless() {
        let hir = hir_of(
            "component Show with value\n\
             \x20   when value\n\
             \x20       Loading show Spinner\n\
             \x20       Failed with e show Spinner\n\
             \x20       Ready with v show Text v\n\
             state total is durable Whole starting 0\n\
             view\n\
             \x20   Show total\n",
        )
        .expect("resolves");
        assert!(
            hir.defs
                .iter()
                .all(|(_, def)| !matches!(def.kind, DefKind::Signal(_)) || def.name != "Show"),
            "a component never becomes a signal"
        );
    }

    #[test]
    fn no_component_message_names_a_rust_type() {
        let sources = [
            "view\n    Column\n        children\n",
            "component Box with label\n    state s is durable Whole starting 0\n    Text label\nview\n    Box \"a\"\n",
            "component A with x\n    B x\ncomponent B with x\n    A x\nview\n    A 1\n",
            "component Box with label\n    Text label\nview\n    Box\n",
        ];
        let forbidden = [
            "Ident",
            "TokenKind",
            "HirNode",
            "DefId",
            "LocalId",
            "HirExpr",
            "Placement",
        ];
        // Every source here is one this pass must refuse. Counted, because
        // the assertion below is inside the loop: were `errors_of` to
        // start returning nothing — the exact failure that would mean the
        // messages had stopped being produced at all — the loop body would
        // never run and this test would pass over zero messages.
        let mut scanned = 0;
        for src in sources {
            let messages = errors_of(src);
            assert!(!messages.is_empty(), "{src:?} must be refused");
            for message in messages {
                scanned += 1;
                for needle in forbidden {
                    assert!(
                        !message.contains(needle),
                        "message for {src:?} leaked `{needle}`: {message}"
                    );
                }
            }
        }
        assert!(
            scanned >= sources.len(),
            "every source must contribute at least one message, got {scanned}"
        );
    }

    // --- `of`, `foreign`, and the call forms (spec §17.4.2) -------------

    #[test]
    fn an_of_call_resolves_to_the_function_it_names() {
        let hir = hir_of(
            "function double of n\n\
             \x20   give n + n\n\
             state a is client Whole from double of 2\n",
        )
        .expect("resolves");
        let (_, def) = hir.defs.iter().find(|(_, d)| d.name == "a").expect("`a`");
        let DefKind::Signal(signal) = &def.kind else {
            panic!("expected a signal")
        };
        assert!(matches!(
            hir.exprs[signal.init].kind,
            HirExprKind::OfCall { .. }
        ));
    }

    /// §17.4.2: the declaration decides the spelling, so calling an `of`
    /// function with `with` is an error naming the one valid form. §4.1
    /// still holds — a caller never chooses between two spellings.
    #[test]
    fn calling_an_of_function_with_with_names_the_one_valid_form() {
        let errors = errors_of(
            "function double of n\n\
             \x20   give n + n\n\
             state a is client Whole from double with n is 2\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`double of …`"), "got: {}", errors[0]);
    }

    #[test]
    fn calling_a_with_function_with_of_names_the_one_valid_form() {
        let errors = errors_of(
            "function double with n\n\
             \x20   give n + n\n\
             state a is client Whole from double of 2\n",
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`double with …`"), "got: {}", errors[0]);
    }

    /// `length of` and `text of` mean one thing wherever they appear, so
    /// no declaration may take either name in the `of` form (§4.1).
    #[test]
    fn a_program_may_not_redeclare_a_built_in_of_operator() {
        let errors = errors_of("function length of xs\n    give 0\n");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("`length of`"), "got: {}", errors[0]);
    }

    /// The `of` namespace never consults locals, which is what makes
    /// `text of n` safe in a scope that binds a local called `text` —
    /// as `guestbook.zd`'s `Ready with text` does.
    #[test]
    fn a_local_does_not_hide_an_of_operator_of_the_same_name() {
        hir_of(
            "state entry is client Remote of Text from entry\n\
             function describe\n\
             \x20   when entry\n\
             \x20       Loading\n\
             \x20           give \"\"\n\
             \x20       Failed with error\n\
             \x20           give \"\"\n\
             \x20       Ready with text\n\
             \x20           give length of text\n",
        )
        .expect("`length of` is not looked up among the locals");
    }

    /// §4.4 writes a callable with no parameters as a bare name, and
    /// nothing implemented it. That is what makes `clock` work.
    #[test]
    fn a_bare_name_calls_a_callable_that_declares_no_parameters() {
        let hir = hir_of(
            "foreign clock is anywhere\n\
             \x20   from \"zd:time\" as \"now\"\n\
             \x20   gives Whole\n\
             state now is client Whole from clock\n",
        )
        .expect("resolves");
        let (_, def) = hir.defs.iter().find(|(_, d)| d.name == "now").expect("now");
        let DefKind::Signal(signal) = &def.kind else {
            panic!("expected a signal")
        };
        assert!(
            matches!(
                &hir.exprs[signal.init].kind,
                HirExprKind::Call { args, .. } if args.is_empty()
            ),
            "got {:?}",
            hir.exprs[signal.init].kind
        );
    }

    /// §17.4.2: `BUILTIN_PATTERNS` recognised the built-in variants in
    /// *pattern* position only, so no function could ever return an
    /// `Option`. A library whose whole job is producing one needs to build
    /// it.
    #[test]
    fn a_built_in_variant_can_be_constructed_and_not_only_matched() {
        let hir = hir_of(
            "function wrap with v\n\
             \x20   give Some with value is v\n\
             function nothing\n\
             \x20   give None\n",
        )
        .expect("resolves");
        let (_, def) = hir
            .defs
            .iter()
            .find(|(_, d)| d.name == "wrap")
            .expect("wrap");
        let DefKind::Function(function) = &def.kind else {
            panic!("expected a function")
        };
        let HirStmt::Give(expr) = &hir.blocks[function.body].stmts[0] else {
            panic!("expected a give")
        };
        assert!(matches!(
            hir.exprs[*expr].kind,
            HirExprKind::Call {
                callee: Res::BuiltinVariant(_),
                ..
            }
        ));
    }

    #[test]
    fn a_foreign_becomes_a_definition_carrying_its_module_and_symbol() {
        let hir = hir_of(
            "foreign trim is anywhere\n\
             \x20   from \"zd:text\" as \"trim\"\n\
             \x20   takes of value is Text\n\
             \x20   gives Text\n",
        )
        .expect("resolves");
        let (_, def) = hir.defs.iter().next().expect("a definition");
        let DefKind::Foreign(foreign) = &def.kind else {
            panic!("expected a foreign, got {:?}", def.kind)
        };
        assert_eq!(foreign.module(), Some("zd:text"));
        assert_eq!(foreign.export.as_str(), "trim");
        assert!(foreign.is_primitive());
        assert!(!foreign.owns_view());
        assert_eq!(foreign.params.len(), 1);
    }

    /// A specifier whose scheme names no place a module is fetched from is
    /// refused (#238).
    ///
    /// `//host/x.js` is here for a different reason from the rest: it is a
    /// protocol-relative URL, so what it resolves to depends on how the
    /// page was served, and a specifier that means two things is not one
    /// the manifest can report.
    #[test]
    fn a_foreign_from_a_scheme_that_is_not_fetchable_is_refused() {
        for module in [
            "//evil.example/x.js",
            "data:text/javascript,alert(1)",
            "file:///etc/passwd",
            "npm:left-pad",
        ] {
            let source = format!(
                "foreign parse is anywhere\n\
                 \x20   from \"{module}\" as \"parse\"\n\
                 \x20   gives Text\n"
            );
            let errors = errors_of(&source);
            assert!(
                errors.iter().any(|e| e.contains("imports from")),
                "`{module}` names no fetchable module and must be refused, got {errors:?}"
            );
        }
    }

    /// A URL is allowed, and the refusal that used to cover it is gone
    /// (#238). It did not prevent the remote code — it moved it into a
    /// hand-written `.js` file importing the same URL — so it cost the
    /// compiler its only view of what a page fetches and bought nothing.
    #[test]
    fn a_foreign_from_a_url_resolves() {
        for module in [
            "https://esm.sh/three@0.180.0",
            "http://localhost:8080/three.js",
        ] {
            let source = format!(
                "foreign parse is anywhere\n\
                 \x20   from \"{module}\" as \"parse\"\n\
                 \x20   gives Text\n"
            );
            let hir = hir_of(&source).unwrap_or_else(|errors| panic!("`{module}`: {errors:?}"));
            let (_, def) = hir.defs.iter().next().expect("a definition");
            let DefKind::Foreign(foreign) = &def.kind else {
                panic!("expected a foreign")
            };
            assert_eq!(foreign.target, Some(ModuleTarget::AsWritten));
        }
    }

    /// The failure this replaces (#238): `from "three"` compiled, shipped
    /// nothing, wrote no import map, and the page died on its first
    /// import. A resolver with no project to read a mapping from is the
    /// same case as a project that maps nothing.
    #[test]
    fn a_bare_specifier_with_no_mapping_is_refused() {
        let errors = errors_of(
            "foreign parse is anywhere\n\
             \x20   from \"marked\" as \"parse\"\n\
             \x20   gives Text\n",
        );
        assert!(
            errors.iter().any(|e| e.contains("zd.toml")),
            "the repair is a line in a file, so the message names it: {errors:?}"
        );
    }

    /// A control character in a specifier is refused for a different
    /// reason than a scheme: it is read back by tools that treat those as
    /// commands, so it never reaches a resolver as written.
    #[test]
    fn a_foreign_module_carrying_a_control_character_is_refused() {
        let errors = errors_of(
            "foreign parse is anywhere\n\
             \x20   from \"./a\u{1b}[2Jb.js\" as \"parse\"\n\
             \x20   gives Text\n",
        );
        assert!(
            errors.iter().any(|e| e.contains("control character")),
            "got {errors:?}"
        );
    }

    /// The forms that resolve without the project having to say anything
    /// are accepted — otherwise the refusal would take the prelude with
    /// it, which is where `zd:text` comes from.
    #[test]
    fn a_foreign_from_within_this_build_still_resolves() {
        for module in [
            "./sparkline.js",
            "../lib/spark.js",
            "/vendor/x.js",
            "zd:text",
        ] {
            let source = format!(
                "foreign parse is anywhere\n\
                 \x20   from \"{module}\" as \"parse\"\n\
                 \x20   gives Text\n"
            );
            assert!(
                hir_of(&source).is_ok(),
                "`{module}` resolves within this build and must be accepted"
            );
        }
    }

    /// A `gives view` foreign owns a DOM node, and neither a server
    /// function nor the build host has a `document` to own one in.
    #[test]
    fn a_view_giving_foreign_must_be_client() {
        for site in ["server", "anywhere"] {
            let source = format!(
                "foreign Sparkline is {site}\n\
                 \x20   from \"./sparkline.js\" as \"mount\"\n\
                 \x20   gives view\n"
            );
            let errors = errors_of(&source);
            assert!(
                errors.iter().any(|e| e.contains("gives a view")),
                "`is {site}` cannot own a DOM node, got {errors:?}"
            );
        }
        assert!(hir_of(
            "foreign Sparkline is client\n\
             \x20   from \"./sparkline.js\" as \"mount\"\n\
             \x20   gives view\n",
        )
        .is_ok());
    }

    #[test]
    fn a_list_of_names_reads_as_english() {
        assert_eq!(english_list(&["a"]), "`a`");
        assert_eq!(english_list(&["a", "b"]), "`a`, and `b`");
        assert_eq!(english_list(&["a", "b", "c"]), "`a`, `b`, and `c`");
    }
}
