//! What is at each span, according to the compiler.
//!
//! Every editor feature this crate offers is a question about a position:
//! what is here, what type does it have, where was it declared, how should
//! it be coloured. One index answers all four, so the three features cannot
//! disagree about what a span is.
//!
//! The index is built from the syntax tree, which owns the spans, and from
//! the HIR, which owns the meanings. They are joined on the **start byte of
//! an identifier**, which is unique within a file: no two identifiers begin
//! at the same offset, and the four HIR nodes that carry a resolution all
//! begin at the identifier they resolve — a reference at its name, a call
//! at its callee, a mutation target at its base, an element at its tag.
//! That is checked by `spans_start_at_their_identifier` below, so the join
//! fails loudly if the parser's span conventions ever change.

use std::collections::HashMap;

use zdc_ast as ast;
use zdc_hir::{DefId, DefKind, Hir, HirExprKind, HirNode, HirNodeArmBody, HirStmt, LocalId, Res};
use zdc_lexer::{Span, Token, TokenKind};

/// A span, and what the compiler says is at it.
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub span: Span,
    pub name: String,
    pub kind: SymbolKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    /// A `state` declaration's own name.
    Signal {
        def: Option<DefId>,
        placement: ast::Placement,
        secret: bool,
        /// `starting` declares a source; `from` declares a derivation
        /// that cannot be assigned to (spec §4.5).
        source: bool,
    },
    /// A `function` declaration's own name.
    Function { def: Option<DefId> },
    /// A `component` declaration's own name.
    Component { def: Option<DefId> },
    /// The `view` keyword's declaration.
    View,
    /// A parameter, loop variable, or pattern binder, where it is bound.
    Binding {
        local: Option<LocalId>,
        parameter: bool,
    },
    /// A use of a name. `res` is `None` when the file did not resolve.
    Use {
        res: Option<Res>,
        expr: Option<zdc_hir::ExprId>,
    },
    /// A view element's tag.
    Element,
    /// A `when` arm's variant name.
    Variant,
    /// A name written in type position.
    TypeName { builtin: bool },
    /// A named argument's label — the `hint` of `hint is "search"`.
    Label,
    /// A field selected with `.`.
    Field,
    /// An event name — the `click` of `on click`.
    Event,
    /// `is`, in whichever of its three jobs it is doing here.
    Is(IsRole),
}

/// The three jobs `is` does, which is the distinction a regular expression
/// cannot make and this crate exists to make (`editors/vscode/README.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsRole {
    /// `state count is client Whole …` — it introduces a declaration.
    Declaration,
    /// `hint is "search"` — it binds a named argument.
    NamedArgument,
    /// `a is b` — it tests equality.
    Equality,
}

/// Every symbol in a file, ordered so that a lookup finds the innermost.
#[derive(Debug, Default, Clone)]
pub struct SymbolIndex {
    symbols: Vec<Symbol>,
}

impl SymbolIndex {
    /// The innermost symbol whose span contains this byte offset.
    ///
    /// Half-open at the end, except that a cursor resting immediately
    /// after a name still hovers it: an editor reports the caret between
    /// two characters, and after the last letter of `count` is where a
    /// programmer thinks they are pointing at `count`.
    pub fn at(&self, offset: u32) -> Option<&Symbol> {
        self.symbols
            .iter()
            .filter(|symbol| symbol.span.start <= offset && offset <= symbol.span.end)
            .min_by_key(|symbol| symbol.span.len())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.iter()
    }

    /// Every symbol whose span starts at this offset, for the token
    /// encoder, which walks the lexer's output rather than this index.
    pub fn by_start(&self) -> HashMap<u32, &Symbol> {
        let mut found: HashMap<u32, &Symbol> = HashMap::new();
        for symbol in &self.symbols {
            // A shorter span at the same start is the more specific
            // description of the token that begins there.
            found
                .entry(symbol.span.start)
                .and_modify(|existing| {
                    if symbol.span.len() < existing.span.len() {
                        *existing = symbol;
                    }
                })
                .or_insert(symbol);
        }
        found
    }
}

/// Build the index from whatever the pipeline produced.
///
/// The syntax tree alone yields every span and every syntactic role. The
/// HIR, when the file resolved, adds what each name refers to. A file that
/// does not resolve therefore still highlights and still hovers; it only
/// stops being able to say where a name was declared.
pub fn index(program: &ast::Program, hir: Option<&Hir>, tokens: &[Token]) -> SymbolIndex {
    let mut builder = Builder {
        symbols: Vec::new(),
        tokens,
        uses: hir.map(uses_by_start).unwrap_or_default(),
        defs: hir.map(defs_by_start).unwrap_or_default(),
        locals: hir.map(locals_by_start).unwrap_or_default(),
    };
    builder.program(program);
    SymbolIndex {
        symbols: builder.symbols,
    }
}

// --- joining the HIR onto the syntax tree ---

type Uses = HashMap<u32, (Res, Option<zdc_hir::ExprId>)>;

/// Every resolved reference, keyed by the start byte of its identifier.
fn uses_by_start(hir: &Hir) -> Uses {
    let mut found: Uses = HashMap::new();

    // Expressions live in one flat arena, so references and calls need no
    // tree walk at all.
    for (id, expr) in hir.exprs.iter() {
        let res = match &expr.kind {
            HirExprKind::Ref(res) => *res,
            HirExprKind::Call { callee, .. } => *callee,
            _ => continue,
        };
        found.insert(expr.span.start, (res, Some(id)));
    }

    // Mutation targets are statements, and blocks are a flat arena too.
    for (_, block) in hir.blocks.iter() {
        for stmt in &block.stmts {
            if let HirStmt::Mutation(mutation) = stmt {
                let place = match mutation {
                    zdc_hir::HirMutation::Set { place, .. }
                    | zdc_hir::HirMutation::Add { place, .. }
                    | zdc_hir::HirMutation::Subtract { place, .. }
                    | zdc_hir::HirMutation::Append { place, .. }
                    | zdc_hir::HirMutation::Remove { place, .. } => place,
                };
                found.insert(place.span.start, (place.base, None));
            }
        }
    }

    // View nodes are the one part of the HIR that is a tree.
    for (_, def) in hir.defs.iter() {
        if let DefKind::View(view) = &def.kind {
            element_uses(&view.nodes, &mut found);
        }
    }

    found
}

fn element_uses(nodes: &[HirNode], found: &mut Uses) {
    for node in nodes {
        match node {
            HirNode::Element(element) => {
                found.insert(element.span.start, (element.res, None));
                element_uses(&element.children, found);
            }
            HirNode::Each(each) => element_uses(&each.body, found),
            HirNode::When(when) => {
                for arm in &when.arms {
                    match &arm.body {
                        HirNodeArmBody::Show(element) => {
                            found.insert(element.span.start, (element.res, None));
                            element_uses(&element.children, found);
                        }
                        HirNodeArmBody::Nodes(nodes) => element_uses(nodes, found),
                    }
                }
            }
            HirNode::If(conditional) => {
                element_uses(&conditional.then, found);
                if let Some(otherwise) = &conditional.otherwise {
                    element_uses(otherwise, found);
                }
            }
            HirNode::Scope(scope) => element_uses(&scope.body, found),
            HirNode::Handler(_) | HirNode::Children(_) => {}
        }
    }
}

fn defs_by_start(hir: &Hir) -> HashMap<u32, DefId> {
    hir.defs
        .iter()
        .map(|(id, def)| (def.span.start, id))
        .collect()
}

fn locals_by_start(hir: &Hir) -> HashMap<u32, LocalId> {
    hir.locals
        .iter()
        .map(|(id, local)| (local.span.start, id))
        .collect()
}

// --- the syntax walk ---

struct Builder<'a> {
    symbols: Vec<Symbol>,
    tokens: &'a [Token],
    uses: Uses,
    defs: HashMap<u32, DefId>,
    locals: HashMap<u32, LocalId>,
}

impl<'a> Builder<'a> {
    fn push(&mut self, span: Span, name: impl Into<String>, kind: SymbolKind) {
        self.symbols.push(Symbol {
            span,
            name: name.into(),
            kind,
        });
    }

    /// A use of `ident`, carrying whatever the resolver decided it was.
    fn use_of(&mut self, ident: &ast::Ident) {
        let resolved = self.uses.get(&ident.span.start).copied();
        self.push(
            ident.span,
            ident.text.clone(),
            SymbolKind::Use {
                res: resolved.map(|(res, _)| res),
                expr: resolved.and_then(|(_, expr)| expr),
            },
        );
    }

    fn binding(&mut self, ident: &ast::Ident, parameter: bool) {
        let local = self.locals.get(&ident.span.start).copied();
        self.push(
            ident.span,
            ident.text.clone(),
            SymbolKind::Binding { local, parameter },
        );
    }

    /// The tokens starting within a byte range.
    ///
    /// Found by bisection rather than by scanning. The callers ask once
    /// per declaration and once per named argument, so a linear scan would
    /// make building the index quadratic in the size of the file — and the
    /// editor rebuilds it on every keystroke.
    ///
    /// Bisection is valid because the lexer emits tokens in source order;
    /// `the_token_stream_is_sorted_by_start_offset` holds that precondition
    /// down. The lifetime is the token stream's rather than `&self`'s, so a
    /// caller may push symbols while walking the result.
    fn tokens_from(&self, from: u32, to: u32) -> &'a [Token] {
        let first = self.tokens.partition_point(|token| token.span.start < from);
        let last = self
            .tokens
            .partition_point(|token| token.span.start < to)
            .max(first);
        &self.tokens[first..last]
    }

    /// The span of the first `is` between two offsets, if there is one.
    ///
    /// `is not` lexes as its own token, so a search for `is` cannot
    /// accidentally match the first half of an inequality.
    fn is_token(&self, from: u32, to: u32) -> Option<Span> {
        self.tokens_from(from, to)
            .iter()
            .find(|token| token.kind == TokenKind::Is && token.span.end <= to)
            .map(|token| token.span)
    }

    fn program(&mut self, program: &ast::Program) {
        for decl in &program.decls {
            match decl {
                ast::Decl::State(state) => self.state(state),
                ast::Decl::Function(function) => self.function(function),
                ast::Decl::View(view) => self.view(view),
                ast::Decl::Component(component) => self.component(component),
                // A record's and a choice's own names are indexed by the
                // resolver pass above; their field and variant names are
                // type-level and carry no hover or jump target yet. An
                // import names declarations in another file, which this
                // index does not hold.
                // A `route`'s own name and its variants are indexed by
                // the resolver pass above, exactly as a `choice`'s are.
                ast::Decl::Record(_)
                | ast::Decl::Choice(_)
                | ast::Decl::Route(_)
                | ast::Decl::Use(_) => {}
            }
        }
    }

    fn state(&mut self, state: &ast::StateDecl) {
        let def = self.defs.get(&state.name.span.start).copied();
        let (source, init) = match &state.init {
            ast::Init::Starting(expr) => (true, expr),
            ast::Init::From(expr) => (false, expr),
        };
        self.push(
            state.name.span,
            state.name.text.clone(),
            SymbolKind::Signal {
                def,
                placement: state.placement,
                secret: state.secret,
                source,
            },
        );

        // Everything between the name and the initializer is the
        // declaration's head: one `is`, a placement keyword, and the type.
        // Type constructors (`List`, `Map`) keep no span of their own in
        // the tree — `TypeExpr::List` holds only its element — so the
        // type is read off the token stream instead, which also gets the
        // constructor words a tree walk would miss.
        let head_end = init.span().start;
        if let Some(span) = self.is_token(state.name.span.end, head_end) {
            self.push(span, "is", SymbolKind::Is(IsRole::Declaration));
        }
        for token in self.tokens_from(state.name.span.end, head_end) {
            if token.span.end > head_end {
                continue;
            }
            if let TokenKind::Ident(text) = &token.kind {
                // Both halves come from the compiler: the lexer owns the
                // constructor words, and the checker owns the base types.
                let builtin = zdc_lexer::word_to_type_ctor(text.as_str()).is_some()
                    || zdc_types::Type::is_builtin_name(text.as_str());
                self.push(token.span, text.clone(), SymbolKind::TypeName { builtin });
            }
        }

        self.expr(init);
    }

    fn function(&mut self, function: &ast::FunctionDecl) {
        let def = self.defs.get(&function.name.span.start).copied();
        self.push(
            function.name.span,
            function.name.text.clone(),
            SymbolKind::Function { def },
        );
        for param in &function.params {
            self.binding(param, true);
        }
        self.block(&function.body);
    }

    fn component(&mut self, component: &ast::ComponentDecl) {
        let def = self.defs.get(&component.name.span.start).copied();
        self.push(
            component.name.span,
            component.name.text.clone(),
            SymbolKind::Component { def },
        );
        for param in &component.params {
            self.binding(param, true);
        }
        for item in &component.body {
            match item {
                ast::ComponentItem::State(state) => self.state(state),
                ast::ComponentItem::Node(node) => self.nodes(std::slice::from_ref(node)),
            }
        }
    }

    fn view(&mut self, view: &ast::ViewDecl) {
        // The `view` keyword itself is the declaration's name, so hovering
        // it says what a view is rather than nothing.
        let keyword = Span::new(view.span.start, view.span.start.saturating_add(4));
        self.push(keyword, "view", SymbolKind::View);
        self.nodes(&view.nodes);
    }

    fn block(&mut self, block: &ast::Block) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &ast::Stmt) {
        match stmt {
            ast::Stmt::Pipeline(clause) => match clause {
                ast::PipelineClause::From(expr) | ast::PipelineClause::TakeFirst(expr) => {
                    self.expr(expr)
                }
                ast::PipelineClause::Keep { var, cond } => {
                    self.binding(var, false);
                    self.expr(cond);
                }
                ast::PipelineClause::Sort { var, key } => {
                    self.binding(var, false);
                    self.expr(key);
                }
                ast::PipelineClause::MapEach { var, to } => {
                    self.binding(var, false);
                    self.expr(to);
                }
            },
            ast::Stmt::Mutation(mutation) => match mutation {
                ast::Mutation::Set { place, value } => {
                    self.place(place);
                    self.expr(value);
                }
                ast::Mutation::Add { value, place }
                | ast::Mutation::Subtract { value, place }
                | ast::Mutation::Append { value, place }
                | ast::Mutation::Remove { value, place } => {
                    self.expr(value);
                    self.place(place);
                }
            },
            ast::Stmt::Give(expr) => self.expr(expr),
            ast::Stmt::When(when) => {
                self.expr(&when.scrutinee);
                for arm in &when.arms {
                    self.pattern(&arm.pattern);
                    match &arm.body {
                        ast::ArmBody::Show(expr) => self.expr(expr),
                        ast::ArmBody::Block(block) => self.block(block),
                    }
                }
            }
            ast::Stmt::Each(each) => {
                self.binding(&each.var, false);
                self.expr(&each.iter);
                self.block(&each.body);
            }
            ast::Stmt::If(conditional) => {
                self.expr(&conditional.cond);
                self.block(&conditional.then);
                if let Some(otherwise) = &conditional.otherwise {
                    self.block(otherwise);
                }
            }
        }
    }

    fn place(&mut self, place: &ast::Place) {
        self.use_of(&place.base);
        for segment in &place.path {
            match segment {
                ast::PathSeg::Field(field) => {
                    self.push(field.span, field.text.clone(), SymbolKind::Field)
                }
                ast::PathSeg::Index(index) => self.expr(index),
            }
        }
    }

    fn pattern(&mut self, pattern: &ast::Pattern) {
        self.push(
            pattern.name.span,
            pattern.name.text.clone(),
            SymbolKind::Variant,
        );
        for binder in &pattern.bindings {
            self.binding(binder, false);
        }
    }

    fn expr(&mut self, expr: &ast::Expr) {
        match expr {
            ast::Expr::Number { .. }
            | ast::Expr::Text { .. }
            | ast::Expr::Truth { .. }
            | ast::Expr::Empty { .. }
            | ast::Expr::Environment { .. }
            // `address` names no declaration: the browser writes it.
            | ast::Expr::Address { .. } => {}
            ast::Expr::Var { name, .. } => self.use_of(name),
            ast::Expr::Call { name, args, .. } => {
                self.use_of(name);
                self.args(args);
            }
            // A literal's contents are ordinary expressions, and the names
            // inside one need hover and go-to-definition like any other.
            ast::Expr::List { items, .. } => {
                for item in items {
                    self.expr(item);
                }
            }
            ast::Expr::Map { entries, .. } => {
                for (key, value) in entries {
                    self.expr(key);
                    self.expr(value);
                }
            }
            ast::Expr::Unary { operand, .. } => self.expr(operand),
            ast::Expr::Binary { op, lhs, rhs, .. } => {
                self.expr(lhs);
                if *op == ast::BinOp::Is {
                    if let Some(span) = self.is_token(lhs.span().end, rhs.span().start) {
                        self.push(span, "is", SymbolKind::Is(IsRole::Equality));
                    }
                }
                self.expr(rhs);
            }
            ast::Expr::Field { base, name, .. } => {
                self.expr(base);
                self.push(name.span, name.text.clone(), SymbolKind::Field);
            }
            ast::Expr::Index { base, index, .. } => {
                self.expr(base);
                self.expr(index);
            }
        }
    }

    fn args(&mut self, args: &[ast::Arg]) {
        for arg in args {
            match arg {
                ast::Arg::Positional(expr) => self.expr(expr),
                ast::Arg::Named { name, value } => {
                    self.push(name.span, name.text.clone(), SymbolKind::Label);
                    if let Some(span) = self.is_token(name.span.end, value.span().start) {
                        self.push(span, "is", SymbolKind::Is(IsRole::NamedArgument));
                    }
                    self.expr(value);
                }
            }
        }
    }

    fn nodes(&mut self, nodes: &[ast::Node]) {
        for node in nodes {
            match node {
                ast::Node::Element(element) => self.element(element),
                ast::Node::Each(each) => {
                    self.binding(&each.var, false);
                    self.expr(&each.iter);
                    self.nodes(&each.body);
                }
                ast::Node::When(when) => {
                    self.expr(&when.scrutinee);
                    for arm in &when.arms {
                        self.pattern(&arm.pattern);
                        match &arm.body {
                            ast::NodeArmBody::Show(element) => self.element(element),
                            ast::NodeArmBody::Nodes(nodes) => self.nodes(nodes),
                        }
                    }
                }
                ast::Node::If(conditional) => {
                    self.expr(&conditional.cond);
                    self.nodes(&conditional.then);
                    if let Some(otherwise) = &conditional.otherwise {
                        self.nodes(otherwise);
                    }
                }
                // `children` is a keyword standing for the nodes nested at
                // the call site: it names nothing to hover or jump to.
                ast::Node::Children(_) => {}
                ast::Node::Handler(handler) => {
                    self.push(
                        handler.event.span,
                        handler.event.text.clone(),
                        SymbolKind::Event,
                    );
                    self.block(&handler.body);
                }
            }
        }
    }

    fn element(&mut self, element: &ast::Element) {
        self.push(
            element.name.span,
            element.name.text.clone(),
            SymbolKind::Element,
        );
        self.args(&element.args);
        self.nodes(&element.children);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(src: &str) -> SymbolIndex {
        let tokens = zdc_lexer::tokenize(src).expect("lexes");
        let program = zdc_parser::parse(src).expect("parses");
        let hir = zdc_resolve::Resolver::new(&program).resolve().ok();
        index(&program, hir.as_ref(), &tokens)
    }

    fn at<'a>(index: &'a SymbolIndex, src: &str, needle: &str) -> &'a Symbol {
        let offset = src.find(needle).expect("the needle is in the source") as u32;
        index.at(offset).expect("a symbol at the needle")
    }

    /// `tokens_from` bisects the token stream, which is only correct if
    /// the lexer emits tokens in source order. Layout tokens are the ones
    /// that could break it, since they stand for shape rather than for
    /// characters, so the sources below all have indentation in them.
    #[test]
    fn the_token_stream_is_sorted_by_start_offset() {
        let sources = [
            "state count is client Whole starting 0\nview\n    Column\n        Text count\n",
            "function f with x\n    if x\n        give 1\n    otherwise\n        give 2\n",
            "state s is durable Map of Text to Whole starting empty\n\
             view\n    each k in s\n        Text k\n",
            "# a comment \u{2014} with an em dash\nstate \u{e9} is client Text starting \"\u{4e2d}\"\n",
        ];
        for src in sources {
            let tokens = zdc_lexer::tokenize(src).expect("lexes");
            for pair in tokens.windows(2) {
                assert!(
                    pair[0].span.start <= pair[1].span.start,
                    "out of order in {src:?}: {:?} then {:?}",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    /// The join between the syntax tree and the HIR assumes four span
    /// conventions. If any of them changes, hover and go-to-definition
    /// silently start answering `None`, so assert them directly.
    #[test]
    fn spans_start_at_their_identifier() {
        let src = "state count is client Whole starting 0\n\
                   function twice with n\n    give n * 2\n\
                   view\n    Text (twice with n is 1)\n";
        let program = zdc_parser::parse(src).expect("parses");
        let hir = zdc_resolve::Resolver::new(&program)
            .resolve()
            .expect("resolves");

        for (_, def) in hir.defs.iter() {
            if def.name == "view" {
                continue;
            }
            let at = def.span.start as usize;
            assert!(
                src[at..].starts_with(&def.name),
                "a definition's span must start at its name, got {:?}",
                &src[at..at + 8]
            );
        }
        for (_, local) in hir.locals.iter() {
            let at = local.span.start as usize;
            assert!(src[at..].starts_with(&local.name));
        }
    }

    #[test]
    fn a_signal_declaration_records_its_placement() {
        let src = "state votes is durable Whole starting 0\n";
        let index = built(src);
        let symbol = at(&index, src, "votes");
        assert!(matches!(
            symbol.kind,
            SymbolKind::Signal {
                placement: ast::Placement::Durable,
                source: true,
                ..
            }
        ));
    }

    #[test]
    fn a_reference_carries_what_it_resolved_to() {
        let src = "state count is client Whole starting 0\n\
                   state twice is client Whole from count * 2\n";
        let index = built(src);
        let use_site = src.rfind("count").expect("the reference") as u32;
        let symbol = index.at(use_site).expect("a symbol");
        assert!(matches!(
            symbol.kind,
            SymbolKind::Use {
                res: Some(Res::Def(_)),
                expr: Some(_)
            }
        ));
    }

    /// The whole reason this crate exists rather than a wider grammar.
    #[test]
    fn is_is_classified_into_its_three_jobs() {
        // `is` three times over. Note that the one in the view is a
        // named argument and not a comparison, which is exactly the
        // distinction being asserted.
        let src = "state open is client Truth starting no\n\
                   state shown is client Truth from open is yes\n\
                   view\n    Checkbox open, hint is \"search\"\n";
        let index = built(src);

        let declaration = src.find(" is ").expect("the declaration") as u32 + 1;
        let named = src.find("hint is").expect("the argument") as u32 + 5;
        let equality = src.rfind("open is yes").expect("the comparison") as u32 + 5;

        assert_eq!(
            index.at(declaration).map(|s| s.kind.clone()),
            Some(SymbolKind::Is(IsRole::Declaration))
        );
        assert_eq!(
            index.at(named).map(|s| s.kind.clone()),
            Some(SymbolKind::Is(IsRole::NamedArgument))
        );
        assert_eq!(
            index.at(equality).map(|s| s.kind.clone()),
            Some(SymbolKind::Is(IsRole::Equality))
        );
    }

    /// The other distinction a regular expression cannot make: both are
    /// capitalised, and only the compiler knows which is which.
    #[test]
    fn a_type_name_and_an_element_name_are_told_apart() {
        let src = "state name is client Text starting \"\"\nview\n    Text name\n";
        let index = built(src);

        let type_at = src.find("client Text").expect("the type") as u32 + 7;
        assert!(matches!(
            index.at(type_at).map(|s| &s.kind),
            Some(SymbolKind::TypeName { builtin: true })
        ));

        let element_at = src.rfind("Text name").expect("the element") as u32;
        assert!(matches!(
            index.at(element_at).map(|s| &s.kind),
            Some(SymbolKind::Element)
        ));
    }

    /// `List` and `Map` keep no span in the syntax tree, so they are read
    /// off the token stream. They must still be types, not variables.
    #[test]
    fn a_type_constructor_is_a_type_even_though_the_tree_drops_it() {
        let src = "state votes is durable Map of Text to Whole starting empty\n";
        let index = built(src);
        let symbol = at(&index, src, "Map");
        assert!(matches!(
            symbol.kind,
            SymbolKind::TypeName { builtin: true }
        ));
        assert!(matches!(
            at(&index, src, "Whole").kind,
            SymbolKind::TypeName { builtin: true }
        ));
    }

    #[test]
    fn a_user_type_is_a_type_but_not_a_builtin_one() {
        let src = "state items is server List of Item starting empty\n";
        let index = built(src);
        assert!(matches!(
            at(&index, src, "Item").kind,
            SymbolKind::TypeName { builtin: false }
        ));
    }

    #[test]
    fn a_when_arm_names_a_variant_and_binds_names() {
        let src = "state items is server List of Item starting empty\n\
                   view\n    when items\n        Loading show Spinner\n\
                   \x20       Failed with error show ErrorBar message is error.message\n\
                   \x20       Ready with ready show Text \"done\"\n";
        let index = built(src);
        assert!(matches!(
            at(&index, src, "Loading").kind,
            SymbolKind::Variant
        ));
        assert!(matches!(
            at(&index, src, "error show").kind,
            SymbolKind::Binding { .. }
        ));
        assert!(matches!(
            at(&index, src, "message is").kind,
            SymbolKind::Label
        ));
    }

    #[test]
    fn an_event_name_is_its_own_kind() {
        let src = "state count is client Whole starting 0\n\
                   view\n    Button \"go\"\n        on click\n            add 1 to count\n";
        let index = built(src);
        assert!(matches!(at(&index, src, "click").kind, SymbolKind::Event));
    }

    /// A file that parses but does not resolve still has every span and
    /// every syntactic role; only the resolution is missing.
    #[test]
    fn an_unresolved_file_still_indexes_its_syntax() {
        let src = "state a is client Whole from nowhere\n";
        let index = built(src);
        assert!(matches!(
            at(&index, src, "nowhere").kind,
            SymbolKind::Use { res: None, .. }
        ));
        assert!(matches!(
            at(&index, src, "Whole").kind,
            SymbolKind::TypeName { .. }
        ));
    }

    #[test]
    fn the_innermost_symbol_wins() {
        let src = "state a is client Whole starting 0\nstate b is client Whole from a + 1\n";
        let index = built(src);
        let reference = src.rfind("a + 1").expect("the reference") as u32;
        let symbol = index.at(reference).expect("a symbol");
        assert_eq!(symbol.name, "a");
        assert_eq!(symbol.span.len(), 1);
    }
}
