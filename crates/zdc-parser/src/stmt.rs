use crate::cursor::{ParseError, Parser};
use zdc_ast::{
    Arm, ArmBody, Block, EachStmt, IfStmt, Mutation, PathSeg, Pattern, PipelineClause, Place, Stmt,
    WhenStmt,
};
use zdc_lexer::TokenKind;

impl Parser {
    /// A newline-introduced, indented run of statements.
    pub fn block(&mut self) -> Result<Block, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Newline, "before an indented block")?;
        self.expect(TokenKind::Indent, "to open an indented block")?;

        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::Dedent) || self.at(&TokenKind::Eof) {
                break;
            }
            stmts.push(self.stmt()?);
        }

        let end = self.peek_span();
        self.eat(&TokenKind::Dedent);
        Ok(Block {
            stmts,
            span: start.to(end),
        })
    }

    pub fn stmt(&mut self) -> Result<Stmt, ParseError> {
        use TokenKind as T;
        match self.peek() {
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
            other => Err(ParseError {
                message: format!(
                    "Expected a statement, found {other:?}. Statements begin with `from`, \
                     `keep`, `sort`, `map`, `take`, `set`, `add`, `subtract`, `give`, `when`, \
                     `each`, or `if`."
                ),
                span: self.peek_span(),
            }),
        }
    }

    fn pipeline_clause(&mut self) -> Result<PipelineClause, ParseError> {
        use TokenKind as T;
        match self.bump().kind {
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
            other => unreachable!("pipeline_clause called on {other:?}"),
        }
    }

    fn mutation(&mut self) -> Result<Mutation, ParseError> {
        use TokenKind as T;
        match self.bump().kind {
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
            other => unreachable!("mutation called on {other:?}"),
        }
    }

    fn place(&mut self) -> Result<Place, ParseError> {
        let base = self.expect_ident("as the target of a mutation")?;
        let mut path = Vec::new();
        let mut end = base.span;
        loop {
            if self.eat(&TokenKind::At) {
                let index = self.expr()?;
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
        let scrutinee = self.scrutinee()?;
        self.expect(TokenKind::Is, "after the value being matched")?;
        self.expect(TokenKind::Newline, "before the match arms")?;
        self.expect(TokenKind::Indent, "to open the match arms")?;

        let mut arms = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::Dedent) || self.at(&TokenKind::Eof) {
                break;
            }
            let arm_start = self.peek_span();
            let pattern = self.pattern()?;
            let body = if self.eat(&TokenKind::Show) {
                ArmBody::Show(self.expr()?)
            } else {
                ArmBody::Block(self.block()?)
            };
            let end = match &body {
                ArmBody::Show(e) => e.span(),
                ArmBody::Block(b) => b.span,
            };
            arms.push(Arm {
                pattern,
                body,
                span: arm_start.to(end),
            });
        }

        let end = self.peek_span();
        self.eat(&TokenKind::Dedent);
        Ok(WhenStmt {
            scrutinee,
            arms,
            span: start.to(end),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::Parser;
    use zdc_ast::{ArmBody, Mutation, PipelineClause, Stmt};

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

    // Regression test: `when EXPR is` must not let `expr()` swallow the
    // trailing `is` as the equality operator, which would leave the parser
    // expecting a right-hand operand where the match arms' newline
    // actually belongs. See `Parser::scrutinee`.
    #[test]
    fn parses_when_with_show_and_block_arms() {
        let b = block(
            "\n    when ranked is\n        Loading show \"loading\"\n        Ready with items\n            give items",
        );
        let Stmt::When(w) = &b.stmts[0] else {
            panic!("expected a when statement")
        };
        assert_eq!(w.arms.len(), 2);
        assert!(matches!(w.arms[0].body, ArmBody::Show(_)));
        assert!(matches!(w.arms[1].body, ArmBody::Block(_)));
    }
}
