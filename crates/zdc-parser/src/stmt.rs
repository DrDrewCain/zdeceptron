use crate::cursor::{describe_found, ParseError, Parser};
use zdc_ast::{
    Arm, ArmBody, BindStmt, Binding, Block, EachStmt, Expr, IfStmt, Mutation, PathSeg, Pattern,
    PipelineClause, Place, Stmt, UnaryOp, WhenStmt,
};
use zdc_lexer::{Span, Token, TokenKind};

impl Parser {
    /// A newline-introduced, indented run of statements.
    pub fn block(&mut self) -> Result<Block, ParseError> {
        let (stmts, span) = self.indented(
            "before an indented block",
            "to open an indented block",
            |p| p.stmt(),
        )?;
        Ok(Block { stmts, span })
    }

    pub fn stmt(&mut self) -> Result<Stmt, ParseError> {
        use TokenKind as T;
        let stmt = match self.peek() {
            T::From | T::Keep | T::Sort | T::MapEach | T::Take => {
                Ok(Stmt::Pipeline(self.pipeline_clause()?))
            }
            T::Set | T::Add | T::Subtract | T::Append | T::Remove => {
                Ok(Stmt::Mutation(self.mutation()?))
            }
            T::Give => {
                self.bump();
                Ok(Stmt::Give(self.expr()?))
            }
            T::With => Ok(Stmt::Bind(self.bind_stmt()?)),
            T::When => Ok(Stmt::When(self.when_stmt()?)),
            T::Each => Ok(Stmt::Each(self.each_stmt()?)),
            T::If => Ok(Stmt::If(self.if_stmt()?)),
            other => Err(not_a_statement(other, self.peek_span())),
        }?;

        if matches!(
            stmt,
            Stmt::Pipeline(_) | Stmt::Mutation(_) | Stmt::Give(_) | Stmt::Bind(_)
        ) {
            self.expect(
                TokenKind::Newline,
                "after the statement. Each statement goes on its own line",
            )?;
        }

        Ok(stmt)
    }

    fn pipeline_clause(&mut self) -> Result<PipelineClause, ParseError> {
        use TokenKind as T;
        let Token { kind, span } = self.bump();
        match kind {
            T::From => Ok(PipelineClause::From(self.expr()?)),
            T::Keep => {
                self.expect(T::Each, "after `keep`")?;
                let var = self.expect_ident("after `keep each`")?;
                self.expect(T::Where, "after the loop name in `keep each`")?;
                Ok(PipelineClause::Keep {
                    var,
                    cond: self.expr()?,
                })
            }
            T::Sort => {
                self.expect(T::Each, "after `sort`")?;
                let var = self.expect_ident("after `sort each`")?;
                self.expect(T::By, "after the loop name in `sort each`")?;
                Ok(PipelineClause::Sort {
                    var,
                    key: self.expr()?,
                })
            }
            T::MapEach => {
                self.expect(T::Each, "after `map`")?;
                let var = self.expect_ident("after `map each`")?;
                self.expect(T::To, "after the loop name in `map each`")?;
                Ok(PipelineClause::MapEach {
                    var,
                    to: self.expr()?,
                })
            }
            T::Take => {
                self.expect(T::First, "after `take`")?;
                Ok(PipelineClause::TakeFirst(self.expr()?))
            }
            other => Err(not_a_statement(&other, span)),
        }
    }

    fn mutation(&mut self) -> Result<Mutation, ParseError> {
        use TokenKind as T;
        let Token { kind, span } = self.bump();
        match kind {
            T::Set => {
                let place = self.place()?;
                self.expect(T::To, "after the target of `set`")?;
                Ok(Mutation::Set {
                    place,
                    value: self.expr()?,
                })
            }
            T::Add => {
                let value = self.expr()?;
                self.expect(T::To, "after the value in `add`")?;
                Ok(Mutation::Add {
                    value,
                    place: self.place()?,
                })
            }
            T::Subtract => {
                let value = self.expr()?;
                self.expect(T::From, "after the value in `subtract`")?;
                Ok(Mutation::Subtract {
                    value,
                    place: self.place()?,
                })
            }
            // §14B.2 splits membership from arithmetic at the keyword, so
            // `append` and `remove` take the same shapes `add` and
            // `subtract` do and mean the collection operation instead.
            T::Append => {
                let value = self.expr()?;
                self.expect(T::To, "after the value in `append`")?;
                Ok(Mutation::Append {
                    value,
                    place: self.place()?,
                })
            }
            T::Remove => {
                let value = self.expr()?;
                self.expect(T::From, "after the value in `remove`")?;
                Ok(Mutation::Remove {
                    value,
                    place: self.place()?,
                })
            }
            other => Err(not_a_statement(&other, span)),
        }
    }

    fn place(&mut self) -> Result<Place, ParseError> {
        let base = self.expect_ident("as the target of a mutation")?;
        let mut path = Vec::new();
        let mut end = base.span;
        loop {
            if self.eat(&TokenKind::At) {
                // The same operand rule as `at` in an expression, from the
                // same function: `votes at i + 1` must mean `(votes at i)
                // + 1` on both sides of the language, not one thing in a
                // value and another in a mutation target.
                let index = self.index_operand()?;
                end = index.span();
                path.push(PathSeg::Index(index));
            } else if self.eat(&TokenKind::Dot) {
                let field = self.expect_ident("after `.`")?;
                end = field.span;
                path.push(PathSeg::Field(field));
            } else {
                break;
            }
        }
        let span = base.span.to(end);
        Ok(Place { base, path, span })
    }

    /// `bindStmt := "with" binding ("," binding)*`, where
    /// `binding := IDENT "is" argumentValue`.
    ///
    /// The value is parsed under §14G.1.1's argument restriction, the same
    /// one every other `with` imposes: without it the comma in `with total
    /// is sumOf with numbers is xs, count is 2` could not be told from the
    /// comma that separates one binding from the next. One rule, written
    /// once, applied wherever `with` takes a comma-separated list.
    ///
    /// A later binding on the same line sees the ones before it, because
    /// they are declared in source order — the reading order the line
    /// already has.
    fn bind_stmt(&mut self) -> Result<BindStmt, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::With, "to begin a binding")?;
        let mut bindings: Vec<Binding> = Vec::new();
        loop {
            let name =
                self.expect_ident("after `with`. A binding is written `with name is value`")?;
            // §4.2 merges `is not` into one operator in the lexer, so
            // `with done is not todo.done` arrives as `IsNot`. §14G.1.1
            // settles the same question for arguments: after a name, `is`
            // introduces the value. Splitting it back apart here is what
            // keeps a negated binding from lexing as an equality test.
            let negated = self.at(&TokenKind::IsNot);
            let operator = if negated {
                self.bump().span
            } else {
                self.expect(
                    TokenKind::Is,
                    "after the name in a binding. A binding is written `with name is value`",
                )?
                .span
            };
            let value = self.argument_value()?;
            let value = if negated {
                let span = operator.to(value.span());
                Expr::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(value),
                    span,
                }
            } else {
                value
            };
            bindings.push(Binding {
                span: name.span.to(value.span()),
                name,
                value,
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        let end = bindings.last().map(|binding| binding.span).unwrap_or(start);
        Ok(BindStmt {
            bindings,
            span: start.to(end),
        })
    }

    fn each_stmt(&mut self) -> Result<EachStmt, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Each, "to begin a loop")?;
        let var = self.expect_ident("after `each`")?;
        self.expect(TokenKind::In, "after the loop name")?;
        let iter = self.expr()?;
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(EachStmt {
            var,
            iter,
            body,
            span,
        })
    }

    fn if_stmt(&mut self) -> Result<IfStmt, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::If, "to begin a conditional")?;
        let cond = self.expr()?;
        let then = self.block()?;
        let mut end = then.span;
        let otherwise = if self.at(&TokenKind::Otherwise) {
            self.bump();
            let block = self.block()?;
            end = block.span;
            Some(block)
        } else {
            None
        };
        Ok(IfStmt {
            cond,
            then,
            otherwise,
            span: start.to(end),
        })
    }

    /// `pattern := IDENT ["with" IDENT ("," IDENT)*]`
    ///
    /// A variant's payload is a list of named fields, and a pattern binds
    /// a fresh name to each of them positionally (spec §14G.1.2), so
    /// `Archived with why, moment` binds two names. A single binder is the
    /// common case, not the only one.
    pub fn pattern(&mut self) -> Result<Pattern, ParseError> {
        let name = self.expect_ident("as a pattern")?;
        let mut end = name.span;
        let mut bindings = Vec::new();
        if self.eat(&TokenKind::With) {
            loop {
                let binder = self.expect_ident("after `with` in a pattern")?;
                end = binder.span;
                bindings.push(binder);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        Ok(Pattern {
            span: name.span.to(end),
            name,
            bindings,
        })
    }

    fn when_stmt(&mut self) -> Result<WhenStmt, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::When, "to begin a match")?;
        let scrutinee = self.expr()?;
        let (arms, end) =
            self.indented("before the match arms", "to open the match arms", |p| {
                p.arm()
            })?;
        Ok(WhenStmt {
            scrutinee,
            arms,
            span: start.to(end),
        })
    }

    fn arm(&mut self) -> Result<Arm, ParseError> {
        let start = self.peek_span();
        let pattern = self.pattern()?;
        let body = if self.eat(&TokenKind::Show) {
            let expr = self.expr()?;
            self.expect(
                TokenKind::Newline,
                "after the match arm. Each match arm goes on its own line",
            )?;
            ArmBody::Show(expr)
        } else {
            ArmBody::Block(self.block()?)
        };
        let end = match &body {
            ArmBody::Show(e) => e.span(),
            ArmBody::Block(b) => b.span,
        };
        Ok(Arm {
            pattern,
            body,
            span: start.to(end),
        })
    }
}

/// The one message for a token that cannot begin a statement.
///
/// `stmt` dispatches on the same token set that `pipeline_clause` and
/// `mutation` match on, so their final arms are not reachable today.
/// They used to be `unreachable!`, which is a panic — a compiler crash
/// with a Rust variant name in it — sitting behind an invariant kept by
/// hand in another function. Returning the ordinary message instead
/// costs nothing and cannot crash if the two ever disagree.
fn not_a_statement(kind: &TokenKind, span: Span) -> ParseError {
    ParseError {
        message: format!(
            "Expected a statement, found {}. Statements begin with `from`, `keep`, `sort`, \
             `map`, `take`, `set`, `add`, `subtract`, `append`, `remove`, `give`, `with`, \
             `when`, `each`, or `if`.",
            describe_found(kind)
        ),
        span,
    }
}

#[cfg(test)]
mod tests {
    use crate::Parser;
    use zdc_ast::{ArmBody, BinOp, Decl, Expr, Mutation, PathSeg, PipelineClause, Place, Stmt};

    fn block(src: &str) -> zdc_ast::Block {
        // `zdc_lexer::tokenize` never emits a leading `Newline` for the very
        // first token of a fresh call (see `leading_blank_line_does_not_emit_a_newline`
        // in zdc-lexer's layout tests): with nothing yet pushed to the output,
        // there is no preceding line for a Newline to terminate. In every real
        // parse a block is preceded by other tokens (a function name, `each
        // ... in ...`, etc.), so this never bites `block()` in practice — but
        // tokenizing a block's source in isolation does hit it. Prepend a
        // sentinel token and consume it first so the token stream has the same
        // shape `block()` sees in context.
        let tokens = zdc_lexer::tokenize(&format!("sentinel{src}")).expect("lexes");
        let mut p = Parser::new(tokens);
        p.bump();
        p.block().expect("parses")
    }

    /// The shape of an expression with its spans dropped, so trees parsed
    /// from different offsets can be compared for structure alone.
    fn shape(e: &Expr) -> String {
        match e {
            Expr::Var { name, .. } => name.text.clone(),
            Expr::Number { value, .. } => format!("{value}"),
            Expr::Field { base, name, .. } => format!("Field({}, {})", shape(base), name.text),
            Expr::Index { base, index, .. } => format!("Index({}, {})", shape(base), shape(index)),
            Expr::Binary { op, lhs, rhs, .. } => {
                format!("{op:?}({}, {})", shape(lhs), shape(rhs))
            }
            other => format!("{other:?}"),
        }
    }

    fn place_shape(p: &Place) -> String {
        p.path
            .iter()
            .fold(p.base.text.clone(), |acc, seg| match seg {
                PathSeg::Field(name) => format!("Field({acc}, {})", name.text),
                PathSeg::Index(index) => format!("Index({acc}, {})", shape(index)),
            })
    }

    fn first_stmt(src: &str) -> Stmt {
        let program = crate::parse(src).expect("parses");
        let Decl::Function(f) = &program.decls[0] else {
            panic!("expected a function")
        };
        f.body.stmts[0].clone()
    }

    /// `at` must mean one thing. Parsing a mutation target's index with
    /// the full `expr()` while an expression's index used only a primary
    /// plus projections made `votes at i + 1` bind as `votes at (i + 1)`
    /// in `add 1 to ...` and as `(votes at i) + 1` in `give ...`: six
    /// identical characters with two meanings, in a language whose whole
    /// claim is one phrasing per construct. Both sides now call
    /// `index_operand`.
    #[test]
    fn at_binds_the_same_way_in_a_value_and_in_a_mutation_target() {
        let Stmt::Give(value) = first_stmt("function f\n    give votes at item.id\n") else {
            panic!("expected a give statement")
        };
        let Stmt::Mutation(Mutation::Add { place, .. }) =
            first_stmt("function f\n    add 1 to votes at item.id\n")
        else {
            panic!("expected an add statement")
        };

        assert_eq!(shape(&value), "Index(votes, Field(item, id))");
        assert_eq!(place_shape(&place), shape(&value));
    }

    /// The other half of the same rule: arithmetic written after an index
    /// is never absorbed into it. In a value it applies to the indexed
    /// element; in a mutation target there is nothing for it to apply to,
    /// so it is reported rather than silently re-binding `at`.
    #[test]
    fn arithmetic_after_an_index_is_not_absorbed_by_at() {
        let Stmt::Give(value) = first_stmt("function f\n    give votes at i + 1\n") else {
            panic!("expected a give statement")
        };
        assert_eq!(shape(&value), "Add(Index(votes, i), 1)");

        let err = crate::parse("function f\n    add 1 to votes at i + 1\n").unwrap_err();
        assert!(err.message.contains("line break"), "got: {}", err.message);

        let err = crate::parse("function f\n    set votes at i + 1 to 0\n").unwrap_err();
        assert!(err.message.contains("`to`"), "got: {}", err.message);
    }

    #[test]
    fn parses_a_pipeline() {
        let b = block("\n    from items\n    keep each item where item.live\n    take first 20");
        assert_eq!(b.stmts.len(), 3);
        assert!(matches!(
            b.stmts[0],
            Stmt::Pipeline(PipelineClause::From(_))
        ));
        assert!(matches!(
            b.stmts[1],
            Stmt::Pipeline(PipelineClause::Keep { .. })
        ));
        assert!(matches!(
            b.stmts[2],
            Stmt::Pipeline(PipelineClause::TakeFirst(_))
        ));
    }

    #[test]
    fn parses_mutations() {
        let b = block("\n    add 1 to votes at item.id\n    set query to \"\"");
        assert!(matches!(b.stmts[0], Stmt::Mutation(Mutation::Add { .. })));
        assert!(matches!(b.stmts[1], Stmt::Mutation(Mutation::Set { .. })));
    }

    /// §14B.2 splits membership from arithmetic at the keyword, so the two
    /// pairs take the same shapes and mean different things.
    #[test]
    fn parses_the_membership_mutations() {
        let b = block("\n    append draft to todos\n    remove todo from todos");
        assert!(matches!(
            b.stmts[0],
            Stmt::Mutation(Mutation::Append { .. })
        ));
        assert!(matches!(
            b.stmts[1],
            Stmt::Mutation(Mutation::Remove { .. })
        ));
    }

    #[test]
    fn append_takes_to_and_remove_takes_from() {
        let err = crate::parse("function f\n    append a from xs\n").unwrap_err();
        assert!(err.message.contains("`to`"), "got: {}", err.message);
        let err = crate::parse("function f\n    remove a to xs\n").unwrap_err();
        assert!(err.message.contains("`from`"), "got: {}", err.message);
    }

    #[test]
    fn the_statement_list_names_the_membership_verbs() {
        let err = crate::parse("function f\n    nonsense\n").unwrap_err();
        assert!(err.message.contains("`append`"), "got: {}", err.message);
        assert!(err.message.contains("`remove`"), "got: {}", err.message);
    }

    /// §17.4.10's local binding, spelled with the word the language
    /// already uses to bind a name to a value.
    #[test]
    fn parses_a_local_binding() {
        let b = block("\n    with total is 0\n    give total");
        let Stmt::Bind(bind) = &b.stmts[0] else {
            panic!("expected a binding, got {:?}", b.stmts[0])
        };
        assert_eq!(bind.bindings.len(), 1);
        assert_eq!(bind.bindings[0].name.text, "total");
    }

    /// The comma separates bindings exactly as it separates arguments,
    /// because it is the same `with`.
    #[test]
    fn one_with_may_bind_several_names() {
        let b = block("\n    with total is 0, index is 1\n    give total");
        let Stmt::Bind(bind) = &b.stmts[0] else {
            panic!("expected a binding")
        };
        let names: Vec<&str> = bind
            .bindings
            .iter()
            .map(|binding| binding.name.text.as_str())
            .collect();
        assert_eq!(names, ["total", "index"]);
    }

    /// §14G.1.1's restriction is the reason the comma above is
    /// unambiguous, so a nested `with` has to be parenthesised here for
    /// the same reason it does in an argument.
    #[test]
    fn a_nested_call_in_a_binding_must_be_parenthesised() {
        let err = crate::parse("function f\n    with total is sumOf with a is 1\n    give total\n")
            .unwrap_err();
        assert!(err.message.contains("parenthesis"), "got: {}", err.message);
    }

    /// The `is not` merge happens in the lexer, before anything knows
    /// which of `is`'s roles this one is. A binding splits it back apart
    /// exactly as an argument does.
    #[test]
    fn a_binding_may_give_a_negated_value() {
        let b = block("\n    with hidden is not shown\n    give hidden");
        let Stmt::Bind(bind) = &b.stmts[0] else {
            panic!("expected a binding")
        };
        assert!(matches!(
            bind.bindings[0].value,
            Expr::Unary {
                op: zdc_ast::UnaryOp::Not,
                ..
            }
        ));
    }

    #[test]
    fn a_binding_needs_is_between_the_name_and_the_value() {
        let err = crate::parse("function f\n    with total 0\n    give total\n").unwrap_err();
        assert!(err.message.contains("`is`"), "got: {}", err.message);
    }

    #[test]
    fn the_statement_list_names_the_binding_keyword() {
        let err = crate::parse("function f\n    nonsense\n").unwrap_err();
        assert!(err.message.contains("`with`"), "got: {}", err.message);
    }

    #[test]
    fn parses_nested_each() {
        let b = block("\n    each item in items\n        give item");
        assert!(matches!(b.stmts[0], Stmt::Each(_)));
    }

    #[test]
    fn parses_when_with_show_and_block_arms() {
        let b = block(
            "\n    when ranked\n        Loading show \"loading\"\n        Ready with items\n            give items",
        );
        let Stmt::When(w) = &b.stmts[0] else {
            panic!("expected a when statement")
        };
        assert_eq!(w.arms.len(), 2);
        assert!(matches!(w.arms[0].body, ArmBody::Show(_)));
        assert!(matches!(w.arms[1].body, ArmBody::Block(_)));
    }

    /// A variant's payload is a list of named fields, so a pattern binds
    /// one fresh name per field (spec §14G.1.2).
    #[test]
    fn a_pattern_binds_one_name_per_named_field() {
        let b = block("\n    when entry\n        Archived with why, moment show why\n");
        let Stmt::When(w) = &b.stmts[0] else {
            panic!("expected a when statement")
        };
        let names: Vec<&str> = w.arms[0]
            .pattern
            .bindings
            .iter()
            .map(|i| i.text.as_str())
            .collect();
        assert_eq!(names, ["why", "moment"]);
    }

    /// The single-binder and payload-free forms are unchanged: one is a
    /// one-element list, the other an empty one.
    #[test]
    fn one_binder_and_no_binder_patterns_are_unchanged() {
        let b = block(
            "\n    when ranked\n        Loading show \"loading\"\n        Ready with items\n            give items",
        );
        let Stmt::When(w) = &b.stmts[0] else {
            panic!("expected a when statement")
        };
        assert!(w.arms[0].pattern.bindings.is_empty());
        assert_eq!(w.arms[1].pattern.bindings.len(), 1);
        assert_eq!(w.arms[1].pattern.bindings[0].text, "items");
    }

    #[test]
    fn a_trailing_comma_in_a_pattern_asks_for_the_next_name() {
        let err = crate::parse("function f\n    when e\n        Archived with why,\n").unwrap_err();
        assert!(err.message.contains("a name"), "got: {}", err.message);
    }

    // Regression test: `when` has no trailing `is` — indentation alone
    // delimits the arms — so the scrutinee is parsed with the full `expr()`,
    // not a restricted binding power. A decorative `is` that collided with
    // the equality operator used to make comparison scrutinees unparseable;
    // assert one now parses with its natural shape.
    #[test]
    fn when_scrutinee_may_be_a_comparison() {
        let b = block("\n    when a < b\n        Loading show \"loading\"\n        Ready with items\n            give items");
        let Stmt::When(w) = &b.stmts[0] else {
            panic!("expected a when statement")
        };
        assert!(matches!(
            w.scrutinee,
            Expr::Binary {
                op: BinOp::Less,
                ..
            }
        ));
        assert_eq!(w.arms.len(), 2);
    }
}
