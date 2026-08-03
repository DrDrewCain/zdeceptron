use crate::cursor::{describe_found, Nesting, ParseError, Parser};
use zdc_ast::{Arg, BinOp, Expr, UnaryOp};
use zdc_lexer::TokenKind;

/// Binding power for infix operators. Higher binds tighter.
fn infix_power(kind: &TokenKind) -> Option<(BinOp, u8)> {
    use TokenKind as T;
    Some(match kind {
        T::Or => (BinOp::Or, 1),
        T::And => (BinOp::And, 2),
        T::Is => (BinOp::Is, 3),
        T::IsNot => (BinOp::IsNot, 3),
        // §17.4.2 puts `contains` at the `cmpExpr` level, so `a + b
        // contains c` reads `(a + b) contains c` exactly as `is` does.
        T::Contains => (BinOp::Contains, 3),
        T::Less => (BinOp::Less, 3),
        T::Greater => (BinOp::Greater, 3),
        T::LessEq => (BinOp::LessEq, 3),
        T::GreaterEq => (BinOp::GreaterEq, 3),
        T::Plus => (BinOp::Add, 4),
        T::Minus => (BinOp::Sub, 4),
        T::Star => (BinOp::Mul, 5),
        T::Slash => (BinOp::Div, 5),
        _ => return None,
    })
}

fn is_comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Is
            | BinOp::IsNot
            | BinOp::Contains
            | BinOp::Less
            | BinOp::Greater
            | BinOp::LessEq
            | BinOp::GreaterEq
    )
}

impl Parser {
    pub fn expr(&mut self) -> Result<Expr, ParseError> {
        self.expr_bp(0)
    }

    fn expr_bp(&mut self, min_power: u8) -> Result<Expr, ParseError> {
        self.nested(Nesting::Expression, |p| p.expr_bp_inner(min_power))
    }

    fn expr_bp_inner(&mut self, min_power: u8) -> Result<Expr, ParseError> {
        // The left spine this loop builds is one level of tree per
        // iteration and no levels of stack, so it is charged against the
        // expression budget for as long as it is being built and handed
        // back once it is. Without that, `1 + 1 + …` is unbounded here and
        // aborts whichever later pass walks it first.
        let mark = self.depth_mark(Nesting::Expression);
        let built = self.infix_spine(min_power);
        self.unwind_to(Nesting::Expression, mark);
        built
    }

    fn infix_spine(&mut self, min_power: u8) -> Result<Expr, ParseError> {
        let mut lhs = self.unary()?;
        let mut saw_comparison = false;

        while let Some((op, power)) = infix_power(self.peek()) {
            if power < min_power {
                break;
            }
            if is_comparison(op) && saw_comparison {
                return Self::chained_comparison(self.peek_span());
            }
            saw_comparison |= is_comparison(op);
            self.deepen(Nesting::Expression)?;
            self.bump();
            // All infix operators are left-associative: requiring a strictly
            // higher power on the right makes `a - b - c` parse as
            // `(a - b) - c` rather than `a - (b - c)`.
            let rhs = self.expr_bp(power + 1)?;
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }

        Ok(lhs)
    }

    /// Out of line: `infix_spine` is on the stack once per level of `(`,
    /// and a refusal nobody reaches should not be paying for slots in it.
    #[inline(never)]
    fn chained_comparison(span: zdc_lexer::Span) -> Result<Expr, ParseError> {
        Err(ParseError {
            message: "Comparisons cannot be chained. Join separate comparisons with `and`, or \
                      add parentheses to make the intended comparison explicit."
                .to_string(),
            span,
        })
    }

    /// Guarded, because every expression recursion runs through here:
    /// `not not not …` directly, and a parenthesised expression by way of
    /// `primary`, which calls `expr` again while this frame is still on
    /// the stack.
    fn unary(&mut self) -> Result<Expr, ParseError> {
        self.nested(Nesting::Expression, |p| p.unary_inner())
    }

    fn unary_inner(&mut self) -> Result<Expr, ParseError> {
        let span = self.peek_span();
        if self.eat(&TokenKind::Not) {
            let operand = self.unary()?;
            let span = span.to(operand.span());
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                operand: Box::new(operand),
                span,
            });
        }
        if self.eat(&TokenKind::Minus) {
            let operand = self.unary()?;
            let span = span.to(operand.span());
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                operand: Box::new(operand),
                span,
            });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, ParseError> {
        // `x.f.f.f…` and `x at i at i…` are spines too, for the same
        // reason and with the same consequence.
        let mark = self.depth_mark(Nesting::Expression);
        let built = self.postfix_spine();
        self.unwind_to(Nesting::Expression, mark);
        built
    }

    fn postfix_spine(&mut self) -> Result<Expr, ParseError> {
        let mut base = self.primary()?;
        loop {
            if self.at(&TokenKind::At) {
                self.deepen(Nesting::Expression)?;
                self.bump();
                // The index operand binds any immediate `.` projections
                // before `at` wraps it, so `votes at item.id` indexes by
                // the *whole* projection `item.id`, not just `item`.
                let index = self.index_operand()?;
                let span = base.span().to(index.span());
                base = Expr::Index {
                    base: Box::new(base),
                    index: Box::new(index),
                    span,
                };
            } else if self.at(&TokenKind::Dot) {
                self.deepen(Nesting::Expression)?;
                self.bump();
                base = self.field_projection(base)?;
            } else {
                break;
            }
        }
        Ok(base)
    }

    /// A primary expression followed by any `.field` projections, used as
    /// the operand of `at` so that projection binds tighter than indexing.
    ///
    /// Shared with `place()`: `at` must bind the same way in a mutation
    /// target as it does in a value, or the same six characters mean two
    /// different things.
    pub(crate) fn index_operand(&mut self) -> Result<Expr, ParseError> {
        let mark = self.depth_mark(Nesting::Expression);
        let built = self.projection_spine();
        self.unwind_to(Nesting::Expression, mark);
        built
    }

    fn projection_spine(&mut self) -> Result<Expr, ParseError> {
        let mut base = self.primary()?;
        while self.at(&TokenKind::Dot) {
            self.deepen(Nesting::Expression)?;
            self.bump();
            base = self.field_projection(base)?;
        }
        Ok(base)
    }

    /// `ofOperand := ofExpr | primary ("." IDENT)*` — §17.4.2.
    ///
    /// The same shape `index_operand` has, deliberately: one precedence
    /// rule rather than two, so `length of names at 0` is `(length of
    /// names) at 0` and `length of item.parts` reads the whole projection.
    /// `of` recurses through `primary`, which is what makes it
    /// right-associative: `text of day of moment` is `text of (day of
    /// moment)`.
    fn of_operand(&mut self) -> Result<Expr, ParseError> {
        self.nested(Nesting::Expression, |p| p.index_operand())
    }

    /// Consume the field name after an already-eaten `.` and wrap `base`
    /// in an `Expr::Field`. Shared by `postfix` and `index_operand` so the
    /// two `.` sites can never drift out of sync.
    fn field_projection(&mut self, base: Expr) -> Result<Expr, ParseError> {
        let name = self.expect_ident("after `.`")?;
        let span = base.span().to(name.span);
        Ok(Expr::Field {
            base: Box::new(base),
            name,
            span,
        })
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        let span = self.peek_span();
        match self.peek().clone() {
            TokenKind::Number(value) => {
                self.bump();
                Ok(Expr::Number { value, span })
            }
            TokenKind::Text(value) => {
                self.bump();
                Ok(Expr::Text { value, span })
            }
            TokenKind::Yes => {
                self.bump();
                Ok(Expr::Truth { value: true, span })
            }
            TokenKind::No => {
                self.bump();
                Ok(Expr::Truth { value: false, span })
            }
            TokenKind::Empty => {
                self.bump();
                Ok(Expr::Empty { span })
            }
            // `append item to list` in expression position — the list
            // construction form. A statement beginning with `append` is
            // §14B.2's mutation and is parsed by `stmt`; the two never
            // compete, because a statement is never parsed here and an
            // expression never begins a statement.
            TokenKind::Append => self.append_expr(span),
            TokenKind::Environment => self.environment_expr(span),
            TokenKind::Address => {
                self.bump();
                Ok(Expr::Address { span })
            }
            // `build read "content/hello.md"` — one keyword, then a
            // capability name and its one operand. The operand is a
            // `primary` rather than a full `expr` so the form ends where
            // it looks like it ends: `build read path + ".md"` is
            // `(build read path) + ".md"`, and anything else is written
            // with the parentheses §14G.1.1 already asks for.
            TokenKind::Build => self.build_expr(span),
            TokenKind::LBracket => self.collection_literal(),
            TokenKind::LParen => {
                self.bump();
                // Parentheses say where the expression ends, which is the
                // whole reason §14G.1.1 asks for them — so inside one, a
                // call may use `with` freely again.
                let outer = self.set_argument_value(false);
                let inner = self.expr();
                self.set_argument_value(outer);
                let inner = inner?;
                self.expect(TokenKind::RParen, "to close a parenthesised expression")?;
                Ok(inner)
            }
            TokenKind::Ident(text) => self.name_expr(text, span),
            other => Self::not_a_value(&other, span),
        }
    }

    /// The failure arm, out of line for the reason the arms above it are:
    /// `format!` needs slots, and `primary` is on the stack once per level
    /// of `(`.
    #[inline(never)]
    fn not_a_value(found: &TokenKind, span: zdc_lexer::Span) -> Result<Expr, ParseError> {
        Err(ParseError {
            message: format!("Expected a value here, found {}.", describe_found(found)),
            span,
        })
    }

    // --- the arms `primary` does not hold open ---------------------------
    //
    // Each of these is one arm of `primary`'s match, moved into a frame of
    // its own and marked `#[inline(never)]`. The reason is stack, and it is
    // measured: a debug build gives every arm of a match its own slots in
    // the enclosing frame and reuses none of them, so `primary`'s frame was
    // the *sum* of every arm's locals — and `primary` is on the stack once
    // per level of `(`. Two branches' worth of arms met here in one merge
    // and took the worst case from roughly 0.4 KB per expression level to
    // roughly 6 KB, which overflowed a 2 MiB thread at the depth
    // `Nesting::Expression` already permits. Splitting them out puts each
    // arm's locals in a frame that exists only while that arm runs, which
    // is what the nesting limits were calibrated against.

    #[inline(never)]
    fn append_expr(&mut self, span: zdc_lexer::Span) -> Result<Expr, ParseError> {
        self.bump();
        let item = self.expr()?;
        self.expect(
            TokenKind::To,
            "after the element. A list is grown by writing `append item to list`",
        )?;
        // `unary`, not `expr`: the list operand is a postfix chain and
        // nothing wider, so `append a to append b to xs` nests to the right
        // and needs no parentheses, while `(append a to xs) + ys` still has
        // to say so.
        let list = self.unary()?;
        let span = span.to(list.span());
        Ok(Expr::Append {
            item: Box::new(item),
            list: Box::new(list),
            span,
        })
    }

    #[inline(never)]
    fn environment_expr(&mut self, span: zdc_lexer::Span) -> Result<Expr, ParseError> {
        self.bump();
        let key_span = self.peek_span();
        match self.peek().clone() {
            TokenKind::Text(key) => {
                self.bump();
                Ok(Expr::Environment {
                    key,
                    span: span.to(key_span),
                })
            }
            _ => Err(ParseError {
                message: "`environment` must be followed by a quoted name, as in `environment \
                          \"STRIPE_KEY\"`."
                    .to_string(),
                span: key_span,
            }),
        }
    }

    /// `build read "content/hello.md"` — one keyword, then a capability
    /// name and its one operand. The operand is a `primary` rather than a
    /// full `expr` so the form ends where it looks like it ends: `build
    /// read path + ".md"` is `(build read path) + ".md"`, and anything else
    /// is written with the parentheses §14G.1.1 already asks for.
    #[inline(never)]
    fn build_expr(&mut self, span: zdc_lexer::Span) -> Result<Expr, ParseError> {
        self.bump();
        let capability = self.expect_ident("after `build`")?;
        let argument = self.primary()?;
        let span = span.to(argument.span());
        Ok(Expr::Build {
            capability,
            argument: Box::new(argument),
            span,
        })
    }

    #[inline(never)]
    fn name_expr(&mut self, text: String, span: zdc_lexer::Span) -> Result<Expr, ParseError> {
        self.bump();
        let name = zdc_ast::Ident { text, span };
        // §14F.1's `of` prefix. `of` currently appears only in type
        // position, and a type is never parsed here, so this
        // introduces no ambiguity — which is the whole reason that
        // section chose the word.
        if self.at(&TokenKind::Of) {
            self.bump();
            let operand = self.of_operand()?;
            let span = span.to(operand.span());
            return Ok(Expr::Of {
                name,
                operand: Box::new(operand),
                span,
            });
        }
        if self.at(&TokenKind::With) {
            if self.in_argument_value() {
                // Both lists are comma-separated, so where this
                // call ends is genuinely ambiguous. Say the one
                // valid form rather than guessing (§4.1).
                return Err(ParseError {
                    message: format!(
                        "A call written with `with` must be parenthesised when it is an \
                             argument, because otherwise there is no way to tell which call a \
                             following `,` belongs to. Write `({} with …)`.",
                        name.text
                    ),
                    span: self.peek_span(),
                });
            }
            self.bump();
            let args = self.call_args()?;
            let end = args.last().map(arg_span).unwrap_or(span);
            Ok(Expr::Call {
                name,
                args,
                span: span.to(end),
            })
        } else {
            Ok(Expr::Var { name, span })
        }
    }

    /// `listLiteral := "[" [expr ("," expr)*] "]"` and
    /// `mapLiteral := "[" expr "to" expr ("," expr "to" expr)* "]"`.
    ///
    /// One production, because the two differ only in whether `to` follows
    /// the first element — which is exactly one token of lookahead, and no
    /// backtracking. `[]` is the empty *list*: `[` cannot introduce two
    /// things at once, so the empty map keeps `empty` and its written type
    /// (spec §14B.4).
    ///
    /// §14G.1.1's restriction applies *inside* a collection literal, and it
    /// is not lifted the way parentheses lift it. A bracket says where the
    /// collection ends but not where an item does, and items are separated
    /// by commas exactly as a `with` list is — so `[Todo with id is 1,
    /// title is "x"]` has the same two readings an argument list does. The
    /// item is parenthesised instead: `[(Todo with id is 1, title is
    /// "x")]`.
    fn collection_literal(&mut self) -> Result<Expr, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::LBracket, "to begin a collection")?;

        let outer = self.set_argument_value(true);
        let literal = self.collection_items(start);
        self.set_argument_value(outer);
        literal
    }

    fn collection_items(&mut self, start: zdc_lexer::Span) -> Result<Expr, ParseError> {
        if self.at(&TokenKind::RBracket) {
            let end = self.peek_span();
            self.bump();
            return Ok(Expr::List {
                items: Vec::new(),
                span: start.to(end),
            });
        }

        let first = self.expr()?;
        if self.eat(&TokenKind::To) {
            let mut entries = vec![(first, self.expr()?)];
            while self.eat(&TokenKind::Comma) {
                let key = self.expr()?;
                self.expect(
                    TokenKind::To,
                    "between a map key and its value. Every entry of a map literal is written \
                     `key to value`",
                )?;
                entries.push((key, self.expr()?));
            }
            let end = self.peek_span();
            self.expect(TokenKind::RBracket, "to close a map")?;
            return Ok(Expr::Map {
                entries,
                span: start.to(end),
            });
        }

        let mut items = vec![first];
        while self.eat(&TokenKind::Comma) {
            items.push(self.expr()?);
        }
        let end = self.peek_span();
        self.expect(TokenKind::RBracket, "to close a list")?;
        Ok(Expr::List {
            items,
            span: start.to(end),
        })
    }

    /// Arguments after `with`: positional expressions and `name is value`
    /// pairs, separated by commas.
    pub fn call_args(&mut self) -> Result<Vec<Arg>, ParseError> {
        let mut args = Vec::new();
        loop {
            args.push(self.one_arg()?);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        Ok(args)
    }

    fn one_arg(&mut self) -> Result<Arg, ParseError> {
        // `name is value` is a named argument; anything else is positional.
        if let TokenKind::Ident(text) = self.peek().clone() {
            let span = self.peek_span();
            // §4.2 merges `is not` into one operator in the lexer, before
            // the parser can see which of `is`'s three roles this one is.
            // §14G.1.1 settles it: `IDENT is expr` in argument position is
            // *always* a named argument, so `done is not todo.done` names
            // `done` and gives it `not todo.done`. Without this the merge
            // silently turns every such argument into an equality test.
            let negated = self.peek_at(1) == &TokenKind::IsNot;
            if negated || self.lookahead_is_named_arg() {
                self.bump();
                let operator = self.bump().span;
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
                return Ok(Arg::Named {
                    name: zdc_ast::Ident { text, span },
                    value,
                });
            }
        }
        Ok(Arg::Positional(self.argument_value()?))
    }

    /// Parse one argument's value under the §14G.1.1 restriction.
    ///
    /// The flag is restored rather than cleared on the way out, so that an
    /// argument list nested inside a parenthesised call still sees the
    /// restriction its own level imposes.
    pub(crate) fn argument_value(&mut self) -> Result<Expr, ParseError> {
        let outer = self.set_argument_value(true);
        let value = self.expr();
        self.set_argument_value(outer);
        value
    }

    fn lookahead_is_named_arg(&self) -> bool {
        self.peek_at(1) == &TokenKind::Is
    }
}

/// The span of an argument's value, whether it was written positionally
/// or as `name is value`. Shared by calls and by view elements.
pub(crate) fn arg_span(arg: &Arg) -> zdc_lexer::Span {
    match arg {
        Arg::Positional(e) => e.span(),
        Arg::Named { value, .. } => value.span(),
    }
}

#[cfg(test)]
mod tests {
    use crate::Parser;
    use zdc_ast::{BinOp, Expr, UnaryOp};

    fn parse(src: &str) -> Expr {
        let tokens = zdc_lexer::tokenize(src).expect("lexes");
        Parser::new(tokens).expr().expect("parses")
    }

    fn op_of(e: &Expr) -> BinOp {
        match e {
            Expr::Binary { op, .. } => *op,
            other => panic!("expected binary, got {other:?}"),
        }
    }

    #[test]
    fn and_binds_tighter_than_or() {
        // a or b and c  ==>  a or (b and c)
        let e = parse("a or b and c");
        assert_eq!(op_of(&e), BinOp::Or);
        if let Expr::Binary { rhs, .. } = &e {
            assert_eq!(op_of(rhs), BinOp::And);
        }
    }

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        let e = parse("a + b * c");
        assert_eq!(op_of(&e), BinOp::Add);
        if let Expr::Binary { rhs, .. } = &e {
            assert_eq!(op_of(rhs), BinOp::Mul);
        }
    }

    #[test]
    fn not_is_prefix() {
        let e = parse("not a");
        assert!(matches!(
            e,
            Expr::Unary {
                op: UnaryOp::Not,
                ..
            }
        ));
    }

    // These three guard the precedence defect the original `postfix`
    // implementation had: parsing the `at` index operand with a bare
    // `primary()` let a trailing `.` re-attach to the wrong node, turning
    // `votes at item.id` into `Field(Index(votes, item), id)` instead of
    // `Index(votes, Field(item, id))`. Assert exact tree shape, not just
    // that parsing succeeds, so a regression can't hide behind `matches!`.

    #[test]
    fn at_indexes_and_dot_projects() {
        // votes at item.id  ==>  Index(votes, Field(item, id))
        let e = parse("votes at item.id");
        if let Expr::Index { base, index, .. } = &e {
            assert!(matches!(**base, Expr::Var { .. }));
            if let Expr::Field {
                base: field_base, ..
            } = &**index
            {
                assert!(matches!(**field_base, Expr::Var { .. }));
            } else {
                panic!("expected index to be a field projection, got {index:?}");
            }
        } else {
            panic!("expected Expr::Index, got {e:?}");
        }
    }

    #[test]
    fn at_is_left_associative() {
        // a at b at c  ==>  Index(Index(a, b), c), not Index(a, Index(b, c))
        let e = parse("a at b at c");
        if let Expr::Index { base, .. } = &e {
            assert!(
                matches!(**base, Expr::Index { .. }),
                "expected outer Index's base to itself be an Index, got {base:?}"
            );
        } else {
            panic!("expected Expr::Index, got {e:?}");
        }
    }

    #[test]
    fn index_operand_takes_the_whole_dot_chain() {
        // a at b.c.d  ==>  Index(a, Field(Field(b, c), d))
        let e = parse("a at b.c.d");
        if let Expr::Index { index, .. } = &e {
            if let Expr::Field {
                base: inner_base, ..
            } = &**index
            {
                assert!(
                    matches!(**inner_base, Expr::Field { .. }),
                    "expected the index's own base to be a Field, got {inner_base:?}"
                );
            } else {
                panic!("expected index to be a Field, got {index:?}");
            }
        } else {
            panic!("expected Expr::Index, got {e:?}");
        }
    }

    #[test]
    fn is_not_parses_as_one_operator() {
        assert_eq!(op_of(&parse("a is not b")), BinOp::IsNot);
    }

    #[test]
    fn call_uses_with() {
        let e = parse("rank with votes");
        assert!(matches!(e, Expr::Call { .. }));
    }

    #[test]
    fn yes_and_no_are_truth_literals() {
        assert!(matches!(parse("yes"), Expr::Truth { value: true, .. }));
        assert!(matches!(parse("no"), Expr::Truth { value: false, .. }));
    }

    // --- collection and record literals (spec §14B.4) ---

    #[test]
    fn a_bracketed_run_of_values_is_a_list() {
        let Expr::List { items, .. } = parse(r#"["red", "green"]"#) else {
            panic!("expected a list literal")
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn brackets_with_nothing_in_them_are_the_empty_list() {
        let Expr::List { items, .. } = parse("[]") else {
            panic!("expected a list literal")
        };
        assert!(items.is_empty());
    }

    /// The map form reuses `to` from `Map of K to V`, so one word means one
    /// thing in type and value position alike.
    #[test]
    fn to_between_a_key_and_a_value_makes_it_a_map() {
        let Expr::Map { entries, .. } = parse(r#"["a" to 1, "b" to 2]"#) else {
            panic!("expected a map literal")
        };
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn a_map_entry_missing_its_to_names_the_one_form() {
        let tokens = zdc_lexer::tokenize(r#"["a" to 1, "b" 2]"#).expect("lexes");
        let err = Parser::new(tokens).expr().unwrap_err();
        assert!(
            err.message.contains("`key to value`"),
            "got: {}",
            err.message
        );
    }

    /// A bracket says where the collection ends but not where an item does,
    /// and items are comma-separated exactly as a `with` list is, so
    /// §14G.1.1's restriction applies inside one.
    #[test]
    fn a_with_expression_inside_a_collection_must_be_parenthesised() {
        let tokens = zdc_lexer::tokenize(r#"[Todo with id is 1, done is no]"#).expect("lexes");
        let err = Parser::new(tokens).expr().unwrap_err();
        assert!(
            err.message.contains("parenthesised"),
            "got: {}",
            err.message
        );

        let parenthesised = parse(r#"[(Todo with id is 1, done is no)]"#);
        let Expr::List { items, .. } = parenthesised else {
            panic!("expected a list literal")
        };
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Expr::Call { .. }));
    }

    /// A record literal shares its production with a call (§4.4), so the
    /// parser produces the same node and resolution decides which it is.
    #[test]
    fn a_record_literal_parses_as_a_call_with_named_arguments() {
        let Expr::Call { name, args, .. } = parse(r#"Todo with title is "x", done is no"#) else {
            panic!("expected a call")
        };
        assert_eq!(name.text, "Todo");
        assert_eq!(args.len(), 2);
        assert!(args
            .iter()
            .all(|arg| matches!(arg, zdc_ast::Arg::Named { .. })));
    }

    /// §4.2 merges `is not` into one operator before the parser can see
    /// which of `is`'s three roles this one is, and §14G.1.1 says a bare
    /// `IDENT is expr` in argument position is *always* a named argument.
    /// Without this the merge silently turns the argument into an equality.
    #[test]
    fn is_not_after_an_argument_name_is_a_named_argument_and_not_an_equality() {
        let Expr::Call { args, .. } = parse("Todo with done is not other.done") else {
            panic!("expected a call")
        };
        let [zdc_ast::Arg::Named { name, value }] = args.as_slice() else {
            panic!("expected one named argument, got {args:?}")
        };
        assert_eq!(name.text, "done");
        assert!(
            matches!(
                value,
                Expr::Unary {
                    op: UnaryOp::Not,
                    ..
                }
            ),
            "got: {value:?}"
        );
    }

    /// The equality itself is still available, parenthesised, exactly as
    /// §14G.1.1 says.
    #[test]
    fn a_parenthesised_call_is_still_comparable_with_is_not() {
        assert_eq!(op_of(&parse("(f with a) is not b")), BinOp::IsNot);
    }

    // --- `of` and `contains` (spec §14F.1, §17.4.2) ---------------------

    #[test]
    fn of_prefixes_a_unary_accessor() {
        let Expr::Of { name, operand, .. } = parse("length of posts") else {
            panic!("expected an `of` expression")
        };
        assert_eq!(name.text, "length");
        assert!(matches!(*operand, Expr::Var { .. }));
    }

    /// §17.4.2 makes `of` right-associative, so `text of day of moment` is
    /// `text of (day of moment)`. Left-associativity would read it as
    /// `(text of day) of moment`, which is not an expression at all.
    #[test]
    fn of_is_right_associative() {
        let Expr::Of { name, operand, .. } = parse("text of day of moment") else {
            panic!("expected an `of` expression")
        };
        assert_eq!(name.text, "text");
        let Expr::Of { name: inner, .. } = *operand else {
            panic!("expected the operand to be another `of`")
        };
        assert_eq!(inner.text, "day");
    }

    /// One precedence rule, not two: `ofOperand` has the exact shape
    /// `index_operand` has, so `length of names at 0` is `(length of
    /// names) at 0` rather than `length of (names at 0)`.
    #[test]
    fn at_applies_to_the_result_of_an_of_expression() {
        let Expr::Index { base, .. } = parse("length of names at 0") else {
            panic!("expected an index")
        };
        assert!(matches!(*base, Expr::Of { .. }), "got {base:?}");
    }

    /// And the operand still takes a whole projection, so `length of
    /// item.parts` counts the parts rather than reading a field off a
    /// count.
    #[test]
    fn an_of_operand_takes_the_whole_dot_chain() {
        let Expr::Of { operand, .. } = parse("length of item.parts") else {
            panic!("expected an `of` expression")
        };
        assert!(matches!(*operand, Expr::Field { .. }), "got {operand:?}");
    }

    #[test]
    fn contains_sits_at_the_comparison_level() {
        // a + b contains c  ==>  (a + b) contains c
        let e = parse("a + b contains c");
        assert_eq!(op_of(&e), BinOp::Contains);
        if let Expr::Binary { lhs, .. } = &e {
            assert_eq!(op_of(lhs), BinOp::Add);
        }
    }

    /// A comparison chain is refused whichever comparisons it mixes, so
    /// `a contains b is c` reads as the mistake it is rather than as one
    /// of two possible groupings.
    #[test]
    fn contains_cannot_be_chained_with_another_comparison() {
        let tokens = zdc_lexer::tokenize("a contains b is c").expect("lexes");
        let err = crate::Parser::new(tokens).expr().unwrap_err();
        assert!(err.message.contains("chained"), "got: {}", err.message);
    }

    #[test]
    fn arithmetic_binds_tighter_than_comparison() {
        // a + b is c  ==>  (a + b) is c
        let e = parse("a + b is c");
        assert_eq!(op_of(&e), BinOp::Is);
        if let Expr::Binary { lhs, .. } = &e {
            assert_eq!(op_of(lhs), BinOp::Add);
        }
    }

    /// `append item to list` in expression position — the construction
    /// form, spelled with the three words §14B.2 already spends on the
    /// mutation. §14G.7.7's reserved-word budget is untouched.
    #[test]
    fn append_parses_as_an_expression() {
        let e = parse("append x to xs");
        let Expr::Append { item, list, .. } = &e else {
            panic!("expected an append, got {e:?}")
        };
        assert!(matches!(**item, Expr::Var { .. }));
        assert!(matches!(**list, Expr::Var { .. }));
    }

    /// The list operand is a postfix chain and nothing wider, so appends
    /// nest to the right and need no parentheses between them.
    #[test]
    fn appends_nest_to_the_right() {
        let e = parse("append a to append b to xs");
        let Expr::Append { list, .. } = &e else {
            panic!("expected an append")
        };
        assert!(
            matches!(**list, Expr::Append { .. }),
            "the inner append is the outer one's list, got {list:?}"
        );
    }

    /// The element is a whole expression, so it may be arithmetic without
    /// parentheses; `to` is a keyword and no infix operator, so there is
    /// nowhere for the parse to run past.
    #[test]
    fn the_element_may_be_an_expression() {
        let e = parse("append a + 1 to xs");
        let Expr::Append { item, .. } = &e else {
            panic!("expected an append")
        };
        assert_eq!(op_of(item), BinOp::Add);
    }

    /// And the one phrasing is the only phrasing: `append x xs` says so
    /// rather than guessing where the element ended (§4.1).
    #[test]
    fn append_without_to_names_the_form_it_wanted() {
        let tokens = zdc_lexer::tokenize("append x xs").expect("lexes");
        let err = crate::Parser::new(tokens).expr().unwrap_err();
        assert!(
            err.message.contains("append item to list"),
            "got: {}",
            err.message
        );
    }
}
