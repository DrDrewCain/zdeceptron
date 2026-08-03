use crate::cursor::{describe_found, ParseError, Parser};
use zdc_ast::{
    Arm, ArmBody, Block, EachStmt, IfStmt, Mutation, PathSeg, Pattern, PipelineClause, Place, Stmt,
    WhenStmt,
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
            T::Set | T::Add | T::Subtract => Ok(Stmt::Mutation(self.mutation()?)),
            T::Give => {
                self.bump();
                Ok(Stmt::Give(self.expr()?))
            }
            T::When => Ok(Stmt::When(self.when_stmt()?)),
            T::Each => Ok(Stmt::Each(self.each_stmt()?)),
            T::If => Ok(Stmt::If(self.if_stmt()?)),
            other => Err(not_a_statement(other, self.peek_span())),
        }?;

        if matches!(stmt, Stmt::Pipeline(_) | Stmt::Mutation(_) | Stmt::Give(_)) {
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

    pub fn pattern(&mut self) -> Result<Pattern, ParseError> {
        let name = self.expect_ident("as a pattern")?;
        let mut end = name.span;
        let binding = if self.eat(&TokenKind::With) {
            let b = self.expect_ident("after `with` in a pattern")?;
            end = b.span;
            Some(b)
        } else {
            None
        };
        Ok(Pattern {
            span: name.span.to(end),
            name,
            binding,
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
             `map`, `take`, `set`, `add`, `subtract`, `give`, `when`, `each`, or `if`.",
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
