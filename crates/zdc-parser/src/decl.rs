use crate::codes;
use crate::cursor::{describe_found, found_word, Nesting, ParseError, Parser};
use zdc_ast::{
    CallForm, ChoiceDecl, ComponentDecl, ComponentItem, Emitted, ExportName, FieldDecl,
    ForeignDecl, ForeignParam, ForeignResult, ForeignSite, FunctionDecl, Init, Placement,
    RecordDecl, RouteDecl, RouteParamDecl, RouteVariantDecl, StateDecl, TypeExpr, UseDecl,
    VariantDecl,
};
use zdc_lexer::{SoftKeyword, Span, TokenKind, TypeCtor};

impl Parser {
    pub fn state_decl(&mut self) -> Result<StateDecl, ParseError> {
        let start = self.peek_span();
        // `secret` and `trusted` are two independent lattices (§18.1.2),
        // so both may sit on one declaration and neither implies the
        // other. §4.1 gives each construct one phrasing, so the two
        // modifiers have one order rather than two: `secret trusted
        // state …`. Reversing them is a parse error naming the order, not
        // a second spelling.
        let secret = self.eat(&TokenKind::Secret);
        let trusted = self.eat(&TokenKind::Trusted);
        if self.at(&TokenKind::Secret) {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                "`secret` comes before `trusted`. Write `secret trusted state …`.",
                self.peek_span(),
            )
            .labelled("`secret` reads before `trusted`, not after it"));
        }
        self.expect(TokenKind::State, "to begin a state declaration")?;
        let name = self.expect_ident("after `state`")?;
        self.expect(TokenKind::Is, "after the state name")?;

        let placement = self.placement()?;
        let ty = self.type_expr()?;

        let init = if self.eat(&TokenKind::Starting) {
            Init::Starting(self.expr()?)
        } else if self.eat(&TokenKind::From) {
            Init::From(self.expr()?)
        } else if self.eat_soft(SoftKeyword::Takes) {
            let (params, body) = self.effect_init()?;
            Init::Effect { params, body }
        } else if self.at_soft(SoftKeyword::Every) || self.at_soft(SoftKeyword::After) {
            let (clock, span) = self.clock_init()?;
            // `every "90ms" starting 0 to n + 1` — the clock folds. The
            // clause is trailing and keyword-led like every other, so a
            // clock that only reads the time reads exactly as it did
            // before this existed.
            if self.eat(&TokenKind::Starting) {
                let start = self.expr()?;
                self.expect(
                    TokenKind::To,
                    "after the resting value. A stepping clock is written `every \"90ms\"                      starting <value> to <next>`, and the step may read the cell it writes",
                )?;
                let step = self.expr()?;
                let whole = span.to(step.span());
                Init::Stepping {
                    clock,
                    start: Box::new(start),
                    step: Box::new(step),
                    span: whole,
                }
            } else {
                Init::Clock(clock, span)
            }
        } else {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                "Expected `starting`, `from`, `takes`, `every` or `after` after the type. \
                 `zdc explain E0103` says what each one declares.",
                self.peek_span(),
            )
            .labelled("the declaration needs a value, and no clause is here"));
        };

        let mut end = match &init {
            Init::Starting(e) | Init::From(e) => e.span(),
            Init::Effect { body, .. } => body.span,
            Init::Clock(_, span) => *span,
            Init::Stepping { span, .. } => *span,
        };
        // §14C.3b: a `static` value may be *written* as well as read. The
        // clause is keyword-led and trailing, like every other clause of a
        // declaration, so a signal that emits nothing reads exactly as it
        // did before this existed (§4.1).
        let emits = if self.eat(&TokenKind::Emitting) {
            let span = self.peek_span();
            let TokenKind::Text(path) = self.peek().clone() else {
                return Err(ParseError::new(
                    codes::ONE_VALID_FORM,
                    format!(
                        "Expected a quoted path after `emitting`, found {}. Write the file the \
                         value is written to, such as `emitting \"rss.xml\"`.",
                        describe_found(self.peek())
                    ),
                    span,
                )
                .labelled("the file this value is written to belongs here"));
            };
            self.bump();
            end = span;
            Some(Emitted { path, span })
        } else {
            None
        };

        // An effect's block has already consumed its own terminator, so the
        // declaration is complete without a further newline. Every other
        // init form ends on an expression and still needs one.
        if !matches!(init, Init::Effect { .. }) {
            self.expect(
                TokenKind::Newline,
                "after the declaration. Each declaration goes on its own line",
            )?;
        }
        Ok(StateDecl {
            secret,
            trusted,
            name,
            placement,
            ty,
            init,
            emits,
            span: start.to(end),
        })
    }

    /// The placement clause of a `state` declaration.
    ///
    /// This is the most common error in the language: every `state`
    /// declaration passes through here, and omitting the placement is the
    /// first mistake most people make. What it used to print was a
    /// four-clause paragraph of language documentation at the site of a
    /// one-word omission, under a caret that said `here`, and it never
    /// named the word the reader had actually written.
    ///
    /// Three things replace it. The message names what was found. The
    /// caret says what the span it covers *is*, which for the ordinary
    /// case is a type with the placement missing in front of it. And the
    /// repair is shown as a line rather than described as four choices:
    /// `client` is offered because it is the placement a value with no
    /// other requirement wants, and the other three stay one
    /// `zdc explain E0101` away.
    fn placement(&mut self) -> Result<Placement, ParseError> {
        use TokenKind as T;
        // `remembered` is a soft keyword, so it arrives as an `Ident` and
        // is matched before the four hard ones rather than beside them.
        // One token settles it: a placement is *mandatory* after a `state`
        // declaration's `is` — omitting one is the error arm below — so
        // nothing else can stand in this position, and a program may still
        // name a signal, a field or a parameter `remembered`.
        if let T::Ident(word) = self.peek() {
            if zdc_lexer::word_to_soft_keyword(word) == Some(zdc_lexer::SoftKeyword::Remembered) {
                self.bump();
                return Ok(Placement::Remembered);
            }
        }
        let placement = match self.peek() {
            T::Client => Placement::Client,
            T::Static => Placement::Static,
            T::Server => Placement::Server,
            T::Durable => Placement::Durable,
            other => {
                let span = self.peek_span();
                // A name here is the type the declaration is about to
                // read, so the caret can say which of the two words is
                // missing rather than merely that one is. Anything else is
                // not a type either, and the caret says only what belongs.
                let label = match found_word(other) {
                    Some(word) if matches!(other, T::Ident(_)) => {
                        format!("`{word}` is the type, and a placement goes before it")
                    }
                    _ => "a placement goes here".to_string(),
                };
                return Err(ParseError::new(
                    codes::PLACEMENT,
                    format!(
                        "Expected a placement after `is`, found {}. A `state` declaration \
                         says where its value lives.",
                        found_word(other)
                            .map(|word| format!("`{word}`"))
                            .unwrap_or_else(|| describe_found(other))
                    ),
                    span,
                )
                .labelled(label)
                .suggesting(Span::new(span.start, span.start), "client "));
            }
        };
        self.bump();
        if placement == Placement::Durable {
            self.refuse_per_visitor()?;
        }
        Ok(placement)
    }

    /// `durable per visitor` — recognised, and refused (spec §13, issue #17).
    ///
    /// Checked here rather than left to fall out of the grammar because
    /// falling out of the grammar is what it used to do, and the result
    /// was a diagnostic about a missing `starting` clause: `per` is a soft
    /// keyword, so it parsed as the declaration's *type* and the caret
    /// landed on `visitor`. A reader who asks for per-visitor state has
    /// asked a coherent question, and the answer is that the language has
    /// no principal to key it by — not that they mistyped a type.
    ///
    /// Both words are required before this refuses. A bare `durable per`
    /// is a `state per is durable …`-shaped mistake, or a signal genuinely
    /// named `per`, and neither is this rule; `visitor` is what makes the
    /// phrase the construct §14G.3a names. See [`codes::NO_SUCH_PRINCIPAL`]
    /// for why the construct is refused rather than built.
    fn refuse_per_visitor(&mut self) -> Result<(), ParseError> {
        let start = self.peek_span();
        if !self.at_soft(SoftKeyword::Per) || !self.at_soft_at(1, SoftKeyword::Visitor) {
            return Ok(());
        }
        self.bump();
        self.bump();
        let span = start.to(self.last_span());
        Err(ParseError::new(
            codes::NO_SUCH_PRINCIPAL,
            "`durable per visitor` is not a placement. Per-visitor storage needs a principal, \
             and the language has no way to establish one: nothing authenticates a request.",
            span,
        )
        .labelled("there is no visitor the compiler could key this by"))
    }

    pub fn type_expr(&mut self) -> Result<TypeExpr, ParseError> {
        self.nested(Nesting::Type, |p| p.type_expr_inner())
    }

    /// Which words construct a type is the lexer's dialect table to
    /// decide, not this function's: a dialect replaces `word_to_kind` and
    /// `word_to_type_ctor` together, and no English spelling appears
    /// here. The context strings quote the word the user actually wrote
    /// for the same reason.
    fn type_expr_inner(&mut self) -> Result<TypeExpr, ParseError> {
        // A parenthesised type is refused here rather than by
        // `expect_ident`, because "a name belongs here" is true and
        // useless: a reader who wrote `Map of Text to (Option of Whole)`
        // wrote the parentheses to *group*, and being told a name was
        // expected reads as though the type were wrong rather than the
        // grouping (#256).
        //
        // Refused rather than accepted-and-ignored, which was the other
        // option. Accepting them is a grammar change, and §4.1 admits one
        // phrasing per construct — two spellings of the same type is
        // exactly what that rule exists to prevent. So the message states
        // the rule and shows the form that works.
        if matches!(self.peek(), TokenKind::LParen) {
            let span = self.peek_span();
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                "Types are not parenthesised. A type reads right to left — `Map of Text to \
                 Option of Whole` — so there is nothing for grouping to change."
                    .to_string(),
                span,
            )
            .labelled("no parentheses in a type"));
        }
        let name = self.expect_ident("as a type")?;
        let Some(ctor) = zdc_lexer::word_to_type_ctor(&name.text) else {
            return Ok(TypeExpr::Named(name));
        };

        let after_the_word = format!("after `{}`", name.text);
        self.expect(TokenKind::Of, &after_the_word)?;

        match ctor {
            TypeCtor::List => Ok(TypeExpr::List(Box::new(self.type_expr()?))),
            TypeCtor::Option => Ok(TypeExpr::Option(Box::new(self.type_expr()?))),
            TypeCtor::Remote => Ok(TypeExpr::Remote(Box::new(self.type_expr()?))),
            TypeCtor::Map => {
                let key = self.type_expr()?;
                self.expect(
                    TokenKind::To,
                    &format!("between the key and value types of a `{}`", name.text),
                )?;
                let value = self.type_expr()?;
                Ok(TypeExpr::Map(Box::new(key), Box::new(value)))
            }
            // The same two-operand shape a `Map` has, and the same `to`
            // between them: an entry of a `Map of K to V` is a
            // `Pair of K to V`, so one reading of `to` covers both.
            TypeCtor::Pair => {
                let first = self.type_expr()?;
                self.expect(
                    TokenKind::To,
                    &format!("between the two halves of a `{}`", name.text),
                )?;
                let second = self.type_expr()?;
                Ok(TypeExpr::Pair(Box::new(first), Box::new(second)))
            }
        }
    }

    /// `recordDecl := "record" IDENT NEWLINE INDENT field+ DEDENT`
    ///
    /// A record is a product type with named fields (spec §14B.1). Fields
    /// are written `name is type`, reusing `is` exactly as a named argument
    /// does, so one phrasing carries one meaning across the language.
    pub fn record_decl(&mut self) -> Result<RecordDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Record, "to begin a record")?;
        let name = self.expect_ident("after `record`")?;
        let (fields, end) = self.indented(
            "before a record's fields",
            "to open a record's fields. A record declares its fields indented under its name",
            |p| p.field_decl(),
        )?;
        Ok(RecordDecl {
            name,
            fields,
            span: start.to(end),
        })
    }

    /// `choiceDecl := "choice" IDENT NEWLINE INDENT variant+ DEDENT`
    ///
    /// §14G.1.2: a variant carries *named* fields, and a `when` pattern
    /// binds fresh names to them positionally. Construction is therefore by
    /// name and elimination by position, which is one rule for every choice
    /// including the built-in `Option` and `Remote`.
    pub fn choice_decl(&mut self) -> Result<ChoiceDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Choice, "to begin a choice")?;
        let name = self.expect_ident("after `choice`")?;
        let (variants, end) = self.indented(
            "before a choice's variants",
            "to open a choice's variants. A choice declares its variants indented under its name",
            |p| p.variant_decl(),
        )?;
        Ok(ChoiceDecl {
            name,
            variants,
            span: start.to(end),
        })
    }

    /// `field := IDENT "is" type NEWLINE`
    fn field_decl(&mut self) -> Result<FieldDecl, ParseError> {
        // `unique` leads the field, so the production stays LL(1) on one
        // token: the word cannot also be a field name, which is exactly
        // what reserving it bought (§14G.7.7).
        let start = self.peek_span();
        let unique = self.eat(&TokenKind::Unique);
        let name = self.expect_ident("as a field name")?;
        self.expect(TokenKind::Is, "after the field name")?;
        let ty = self.type_expr()?;
        let end = self.last_span();
        self.expect(
            TokenKind::Newline,
            "after the field. Each field goes on its own line",
        )?;
        Ok(FieldDecl {
            span: if unique {
                start.to(end)
            } else {
                name.span.to(end)
            },
            unique,
            name,
            ty,
        })
    }

    /// `variant := IDENT ["is" TEXT] ["with" variantField ("," variantField)*] NEWLINE`
    ///
    /// The optional label sits between the name and `with`, which is where
    /// a `route` puts its URL — `Home is "/" with slug is Text in slugs`
    /// and `DirtBike is "Dirt Bike"` are the same production shape on
    /// purpose. Both are the string the variant is known by outside the
    /// program; neither is readable by anything inside it.
    fn variant_decl(&mut self) -> Result<VariantDecl, ParseError> {
        let name = self.expect_ident("as a variant name")?;

        let (label, label_span) = if self.eat(&TokenKind::Is) {
            let span = self.peek_span();
            let TokenKind::Text(text) = self.peek().clone() else {
                return Err(ParseError::new(
                    codes::ONE_VALID_FORM,
                    format!(
                        "Expected a quoted label after `is`, found {}. Write `DirtBike is \"Dirt \
                         Bike\"`; a variant's *field* goes after `with`.",
                        describe_found(self.peek())
                    ),
                    span,
                )
                .labelled("the variant's label belongs here, in quotes"));
            };
            self.bump();
            (Some(text), Some(span))
        } else {
            (None, None)
        };

        let mut fields = Vec::new();
        if self.eat(&TokenKind::With) {
            loop {
                let field = self.expect_ident("as a field name after `with`")?;
                self.expect(TokenKind::Is, "after the field name")?;
                let ty = self.type_expr()?;
                fields.push(FieldDecl {
                    span: field.span.to(self.last_span()),
                    // A variant's field is never an identity: `unique`
                    // exists so a *list of rows* reconciles by identity,
                    // and a variant is a shape, not a row. The production
                    // above takes a bare name, so writing `unique` here is
                    // already refused as a missing field name.
                    unique: false,
                    name: field,
                    ty,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        let end = self.last_span();
        self.expect(
            TokenKind::Newline,
            "after the variant. Each variant goes on its own line",
        )?;
        Ok(VariantDecl {
            span: name.span.to(end),
            name,
            label,
            label_span,
            fields,
        })
    }

    /// `useDecl := "use" TEXT "for" IDENT ("," IDENT)* NEWLINE`
    ///
    /// Every `.zd` file is a module and every top-level declaration in one
    /// is importable, but nothing is imported implicitly: the names come
    /// after `for` and the list is the whole of what this file borrows
    /// (spec §14D.2).
    pub fn use_decl(&mut self) -> Result<UseDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Use, "to begin an import")?;

        let path_span = self.peek_span();
        let TokenKind::Text(path) = self.peek().clone() else {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                format!(
                    "Expected a quoted path after `use`, found {}. Write `use \"./model\" for \
                     Item` — the path is relative to this file and the `.zd` ending is implied.",
                    describe_found(self.peek())
                ),
                path_span,
            )
            .labelled("the file being imported belongs here, in quotes"));
        };
        self.bump();

        self.expect(
            TokenKind::For,
            "after the path. An import names what it brings in: `use \"./model\" for Item`",
        )?;

        let mut names = Vec::new();
        loop {
            names.push(self.expect_ident("as an imported name")?);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }

        let end = self.last_span();
        self.expect(
            TokenKind::Newline,
            "after the import. Each import goes on its own line",
        )?;
        Ok(UseDecl {
            path,
            path_span,
            names,
            span: start.to(end),
        })
    }

    /// `componentDecl := "component" IDENT ["with" param ("," param)*]
    ///                    NEWLINE INDENT componentItem+ DEDENT`
    ///
    /// The parameter list reuses `with` exactly as `function` does, and one
    /// of the parameters may be the keyword `children` (§14D.1). A body
    /// item is either the component's own `state` or a view node, because a
    /// component body contains nodes rather than statements.
    pub fn component_decl(&mut self) -> Result<ComponentDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Component, "to begin a component")?;
        let name = self.expect_ident("after `component`")?;

        let mut params = Vec::new();
        let mut children = None;
        if self.eat(&TokenKind::With) {
            loop {
                if self.at(&TokenKind::Children) {
                    let span = self.peek_span();
                    self.bump();
                    if children.is_some() {
                        return Err(ParseError::new(
                            codes::ONE_VALID_FORM,
                            "`children` is written once. It names the nodes nested under this \
                             component at its call site, and there is one such run.",
                            span,
                        )
                        .labelled("`children` is already declared on this component"));
                    }
                    children = Some(span);
                } else {
                    params.push(self.expect_ident("as a parameter name")?);
                }
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }

        let (body, end) = self.indented(
            "before a component's body",
            "to open a component's body. A component declares its nodes indented under its name",
            |p| p.component_item(),
        )?;
        Ok(ComponentDecl {
            name,
            params,
            children,
            body,
            span: start.to(end),
        })
    }

    fn component_item(&mut self) -> Result<ComponentItem, ParseError> {
        if self.at(&TokenKind::State) || self.at(&TokenKind::Secret) || self.at(&TokenKind::Trusted)
        {
            return Ok(ComponentItem::State(self.state_decl()?));
        }
        Ok(ComponentItem::Node(self.node()?))
    }

    /// `routeDecl := "route" IDENT NEWLINE INDENT routeVariant+ DEDENT`
    ///
    /// Spelled exactly as `choice` is, because that is what it is: a route
    /// is a choice plus a bijection onto URLs (spec §14G.2). Declaring it
    /// rather than deriving it from a directory keeps the URL space inside
    /// the language, where the collision check and the parameter types can
    /// reach it.
    pub fn route_decl(&mut self) -> Result<RouteDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Route, "to begin a route")?;
        let name = self.expect_ident("after `route`")?;
        let (variants, end) = self.indented(
            "before a route's variants",
            "to open a route's variants. A route declares its URLs indented under its name",
            |p| p.route_variant_decl(),
        )?;
        Ok(RouteDecl {
            name,
            variants,
            span: start.to(end),
        })
    }

    /// `routeVariant := IDENT "is" TEXT ["with" routeParam ("," routeParam)*] NEWLINE`
    fn route_variant_decl(&mut self) -> Result<RouteVariantDecl, ParseError> {
        let name = self.expect_ident("as a route name")?;
        self.expect(TokenKind::Is, "after the route name")?;

        let path_span = self.peek_span();
        let TokenKind::Text(path) = self.peek().clone() else {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                format!(
                    "Expected a quoted URL after `is`, found {}. Write `Home is \"/\"` — the URL \
                     is a literal, and a parameter is written after `with` rather than spelled \
                     inside the string.",
                    describe_found(self.peek())
                ),
                path_span,
            )
            .labelled("the route's URL belongs here, in quotes"));
        };
        self.bump();

        // §6 refuses embedded markup inside a string for the same reason
        // this refuses `[slug]`: a second grammar inside a literal is a
        // grammar nothing checks.
        if path.contains('[') || path.contains(':') || path.contains('{') {
            return Err(ParseError::new(
                codes::ROUTE_URL,
                "A route's URL is a literal prefix, and a parameter is declared after `with` \
                 rather than written inside the string. Write `BlogPost is \"/blog\" with slug \
                 is Text in postSlugs`.",
                path_span,
            )
            .labelled("this URL has a parameter written inside it"));
        }
        if !path.starts_with('/') {
            return Err(ParseError::new(
                codes::ROUTE_URL,
                "A route's URL begins with `/`. Write `\"/blog\"` rather than `\"blog\"`.",
                path_span,
            )
            .labelled("this URL is relative")
            .suggesting(Span::new(path_span.start + 1, path_span.start + 1), "/"));
        }
        if !route_path_is_safe(&path) {
            return Err(ParseError::new(
                codes::ROUTE_URL,
                "A route's URL must be a canonical absolute path: each segment uses only \
                 letters, digits, `-`, `_` or `.`, and neither `.` nor `..` is a segment.",
                path_span,
            )
            .labelled("this URL is not in canonical form"));
        }

        let mut params = Vec::new();
        if self.eat(&TokenKind::With) {
            loop {
                params.push(self.route_param_decl()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }

        let end = self.last_span();
        self.expect(
            TokenKind::Newline,
            "after the route. Each route goes on its own line",
        )?;
        Ok(RouteVariantDecl {
            span: name.span.to(end),
            name,
            path,
            path_span,
            params,
        })
    }

    /// `routeParam := IDENT "is" type ["in" IDENT]`
    ///
    /// `in` takes a bare name and never an expression (§14G.2 revision 4).
    /// An undelimited expression here would be swallowed by the greedy
    /// comma list that follows, so `Archive is "/a" with slug is Text in
    /// slugsIn with items is posts, page is Whole` would silently parse as
    /// one parameter. The ambiguity is gone by construction.
    fn route_param_decl(&mut self) -> Result<RouteParamDecl, ParseError> {
        let name = self.expect_ident("as a route parameter name")?;
        self.expect(TokenKind::Is, "after the route parameter name")?;
        let ty = self.type_expr()?;
        let enumerated_in = if self.eat(&TokenKind::In) {
            Some(self.expect_ident(
                "after `in`. It names a `static` signal holding every value this parameter \
                 ranges over",
            )?)
        } else {
            None
        };
        Ok(RouteParamDecl {
            span: name.span.to(self.last_span()),
            name,
            ty,
            enumerated_in,
        })
    }

    /// `funcDecl := "function" IDENT (("with" params) | ("of" IDENT)) block`
    ///
    /// §17.4.2. The `of` form declares a unary accessor and is called
    /// `length of items`; the `with` form is called `join with …`. The
    /// declaration decides, so §4.1 still holds: a caller never chooses
    /// between two spellings of one call.
    pub fn function_decl(&mut self) -> Result<FunctionDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Function, "to begin a function")?;
        let name = self.expect_ident("after `function`")?;

        let mut params = Vec::new();
        let mut form = CallForm::With;
        if self.eat(&TokenKind::With) {
            loop {
                params.push(self.expect_ident("as a parameter name")?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        } else if self.eat(&TokenKind::Of) {
            form = CallForm::Of;
            params.push(self.expect_ident("as the parameter of an `of` function")?);
        }

        let body = self.block()?;
        let span = start.to(body.span);
        Ok(FunctionDecl {
            name,
            form,
            params,
            body,
            span,
        })
    }

    /// `foreignDecl := "foreign" IDENT "is" site NEWLINE INDENT
    ///                    source "as" EXPORT NEWLINE
    ///                    [ "takes" params | "takes" "of" IDENT "is" type ]
    ///                    "gives" [ "new" ] ("view" | type) NEWLINE DEDENT`
    ///
    /// `source := "from" STRING | "on" "Handle"`
    ///
    /// Spec §14E.1 as amended by §17.4.2. `foreign` lands at this plan
    /// rather than the one §14E named, because the prelude's primitive
    /// layer (§17.4.10) is written with it.
    ///
    /// One declaration form covers both the value-returning FFI and the
    /// DOM-owning one. They differ in the `gives` clause and nowhere else:
    /// `gives view` says the foreign owns a node and hands back no
    /// ZDeceptron value, `gives Text` that it returns one. Two
    /// declaration forms would be the §4.1 violation this language exists
    /// to avoid, and `view` is already a keyword so the DOM-owning form
    /// costs no reserved word of its own.
    ///
    /// The clause order is fixed, per §4.1: `from`, then `takes`, then
    /// `gives`. Writing them in another order is a parse error naming the
    /// order rather than a second accepted spelling.
    pub fn foreign_decl(&mut self) -> Result<ForeignDecl, ParseError> {
        let start = self.peek_span();
        self.expect_soft(SoftKeyword::Foreign, "to begin a foreign declaration")?;
        let name = self.expect_ident("after `foreign`")?;
        self.expect(TokenKind::Is, "after the foreign name")?;
        let site_span = self.peek_span();
        let site = self.foreign_site()?;
        self.expect(
            TokenKind::Newline,
            "after the placement. A foreign declaration's details are indented under it",
        )?;
        self.expect(
            TokenKind::Indent,
            "to open a foreign declaration. Its module, parameters and result are indented under \
             its name",
        )?;

        let source = self.foreign_source()?;
        self.expect_soft(SoftKeyword::As, "to name the symbol")?;
        let (export, export_span) = self.foreign_export()?;
        self.expect(TokenKind::Newline, "after the source line")?;

        let (form, params) = self.foreign_params()?;

        self.expect_soft(SoftKeyword::Gives, "to declare what a foreign gives back")?;
        // `gives [ pure | trusted ] ( view | type )`. The grant's span is
        // taken before it is eaten, so a refusal below points at the word
        // the author wrote rather than at the whole `gives` line.
        let grant_span = self.peek_span();
        let result_grant = self.foreign_result_grant();
        // `gives new T` — the export is a class, so the call constructs.
        // Taken before the result span so a refusal below can point at the
        // whole `new T`, and eaten before `view` is looked for because
        // `gives new view` is a contradiction the arm below names.
        let new_span = self.peek_span();
        let constructs = self.eat_soft(SoftKeyword::New);
        let result_span = self.peek_span();
        // `view` in this position is the DOM-owning form. It is the same
        // word the `view` declaration uses and it is unambiguous here,
        // because no `type` begins with it — the same licence §4.4 already
        // grants `is` in three roles.
        let result = if self.eat(&TokenKind::View) {
            ForeignResult::View
        } else if self.eat_soft(SoftKeyword::Nothing) {
            // `nothing` is soft and lowercase, and every type name in this
            // language is capitalised, so it can never be mistaken for one
            // — `gives Nothing` still parses as `gives` a type no
            // declaration defines, and the checker says so. Eaten before
            // the `constructs` branch for the same reason `view` is:
            // `gives new nothing` is a contradiction the arm below names,
            // and it can only name it if the word was recognised.
            ForeignResult::Nothing
        } else if constructs {
            ForeignResult::New(self.type_expr()?)
        } else {
            ForeignResult::Value(self.type_expr()?)
        };
        // `gives new view` claims the export is a class *and* that it owns
        // a DOM node the runtime hands it. A `gives view` foreign is called
        // by the runtime with a node and a props object, which is a
        // function's contract and not a constructor's, so the two forms are
        // alternatives rather than a combination.
        if let (ForeignResult::View, true) = (&result, constructs) {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                "`gives new view` is two answers to one question. A `gives view` foreign is \
                 handed a DOM node by the runtime, which is a call and not a construction \
                 (spec §14E.1). Write `gives view`."
                    .to_string(),
                new_span.to(result_span),
            )
            .labelled("`new` constructs, and a view foreign is called")
            .suggesting(new_span.to(result_span), "gives view"));
        }
        // `gives new nothing` claims the export is a class *and* that a
        // call to it produces no value. A construction produces one by
        // definition — that is what `new` is for — so the two words
        // contradict each other rather than composing.
        if let (ForeignResult::Nothing, true) = (&result, constructs) {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                format!(
                    "`gives new nothing` is two answers to one question. `new` constructs a host \
                     object, which is a value, and `nothing` says there is none (spec §14E.1). \
                     Write `gives new {}` or `gives nothing`.",
                    zdc_ast::HANDLE_TYPE_NAME
                ),
                new_span.to(result_span),
            )
            .labelled("`new` produces a value, and `nothing` is not one")
            .suggesting(new_span.to(result_span), "gives nothing"));
        }
        // A grant describes a result the checker can then reason about.
        // A constructed host object is not one: it is opaque, its label is
        // the join of what was passed to the constructor, and `pure` or
        // `trusted` here would be a way to declare that join away. That is
        // the laundering hole the whole handle design is arranged to avoid,
        // so the modifier is refused rather than ignored.
        if let (ForeignResult::New(_), Some(word)) = (&result, result_grant.describe()) {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                format!(
                    "`gives {word} new` claims something about a host object the compiler \
                     cannot see into. A constructed value carries the join of its arguments \
                     and nothing weaker (spec §19.2). Write `gives new Handle`."
                ),
                grant_span.to(result_span),
            )
            .labelled(format!(
                "`{word}` describes a value, and a handle is opaque"
            ))
            .suggesting(grant_span.to(result_span), "gives new Handle"));
        }
        // A `gives view` foreign owns a node and hands back no ZDeceptron
        // value; a `gives nothing` foreign hands back none either. So
        // `pure` and `trusted` have no result to be claims about in either
        // form, and it is one refusal because it is one reason. Refused
        // rather than ignored: §21.9's grants are conspicuous by design,
        // and a modifier the compiler silently drops reads to its author as
        // one that was accepted.
        let resultless = match &result {
            ForeignResult::View => Some("view"),
            ForeignResult::Nothing => Some("nothing"),
            ForeignResult::Value(_) | ForeignResult::New(_) => None,
        };
        if let (Some(form), Some(word)) = (resultless, result_grant.describe()) {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                // Inside the inline budget: the repair is carried as a
                // suggestion, which renders as the corrected line, so
                // spelling it out in prose as well spent characters on
                // something the reader is already shown.
                format!(
                    "`gives {word} {form}` claims something about a result that does not exist. \
                     A `gives {form}` foreign hands back no value, so `{word}` has nothing to \
                     describe (spec §14E.1, §21.9)."
                ),
                grant_span.to(result_span),
            )
            .labelled(format!(
                "`{word}` describes a result, and `{form}` is not one"
            ))
            .suggesting(grant_span.to(result_span), format!("gives {form}")));
        }
        let end = self.last_span();
        self.expect(TokenKind::Newline, "after the result type")?;
        self.expect(
            TokenKind::Dedent,
            "to close a foreign declaration. `gives` is its last line",
        )?;

        Ok(ForeignDecl {
            name,
            site,
            site_span,
            source,
            export,
            export_span,
            form,
            params,
            result_grant,
            result,
            result_span: result_span.to(end),
            span: start.to(end),
        })
    }

    /// `requestDecl := "request" IDENT "is" placement NEWLINE INDENT
    ///                    "from" STRING NEWLINE
    ///                    [ "with" args NEWLINE ]
    ///                    "gives" type NEWLINE DEDENT`
    ///
    /// Issue #19's last part. The clause order is fixed, per §4.1: `from`,
    /// then `with`, then `gives`, which is the order `foreign` already
    /// established for source-then-arguments-then-result.
    ///
    /// **`from` takes a string and nothing else, and that is the security
    /// property this function is responsible for.** Every other `from` in
    /// the language takes an expression; this one is the destination of an
    /// outbound request, so a name there would mean the host a program
    /// talks to is decided at run time by a value — which no pass can
    /// check and no Content-Security-Policy can name. The refusal is here,
    /// at the token, rather than downstream on a lowered expression,
    /// because here is where the alternative was never constructed.
    pub fn request_decl(&mut self) -> Result<zdc_ast::RequestDecl, ParseError> {
        let start = self.peek_span();
        self.expect_soft(SoftKeyword::Request, "to begin a request declaration")?;
        let name = self.expect_ident("after `request`")?;
        self.expect(TokenKind::Is, "after the request name")?;
        let placement_span = self.peek_span();
        let placement = self.placement()?;
        self.expect(
            TokenKind::Newline,
            "after the placement. A request declaration's details are indented under it",
        )?;
        self.expect(
            TokenKind::Indent,
            "to open a request declaration. Its destination, arguments and result are indented \
             under its name",
        )?;

        self.expect(
            TokenKind::From,
            "to name the destination a request is sent to",
        )?;
        let destination_span = self.peek_span();
        // **One quoted literal, alone on its line.** Not `self.expr()`
        // followed by a check, because there would then be a moment at
        // which a computed destination existed as a tree — and the whole
        // security property is that it never does. The literal is taken by
        // hand and anything else on the line is refused with the same
        // sentence, so `from apiKey` and `from "https://x/?k=" + apiKey`
        // read the same refusal rather than one about a missing newline.
        let literal = match self.peek() {
            TokenKind::Text(_) => Some(self.expect_text("as a request's destination")?),
            _ => None,
        };
        let Some(destination) = literal.filter(|_| self.at(&TokenKind::Newline)) else {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                "A request's destination is one quoted URL, written down rather than computed. \
                 Only a literal can be checked, or named in the emitted policy.",
                destination_span.to(self.peek_span()),
            )
            .labelled("a quoted URL, and nothing else, goes here"));
        };
        self.expect(TokenKind::Newline, "after the destination")?;

        let args = if self.eat(&TokenKind::With) {
            let args = self.call_args()?;
            self.expect(TokenKind::Newline, "after the request's arguments")?;
            args
        } else {
            Vec::new()
        };

        self.expect_soft(SoftKeyword::Gives, "to declare what a request gives back")?;
        let gives_span = self.peek_span();
        let gives = self.type_expr()?;
        let end = self.last_span();
        self.expect(TokenKind::Newline, "after the result type")?;
        self.expect(
            TokenKind::Dedent,
            "to close a request declaration. `gives` is its last line",
        )?;

        Ok(zdc_ast::RequestDecl {
            name,
            placement,
            placement_span,
            destination,
            destination_span,
            args,
            gives,
            gives_span: gives_span.to(end),
            span: start.to(end),
        })
    }

    /// Where the symbol lives: a module, or the call's first argument.
    ///
    /// ```text
    /// source := "from" STRING | "on" "Handle" | "of" "Handle" | "set" "Handle"
    /// ```
    ///
    /// LL(1), and it costs no reserved word. `on` is already a keyword —
    /// `on click` — `of` is one too — `List of Text`, `length of items` —
    /// and so is `set`, which is §8's mutation statement. No alternative
    /// can begin another, so one token settles it. What follows is the
    /// same `as` clause in all four cases, because the question it answers
    /// ("which symbol") is the same question whichever side of the
    /// alternation was taken.
    ///
    /// **`on`, `of` and `set` are one trio and read as one.** A method is
    /// something you do *on* a host object, a property is something it
    /// has, and a write gives it one: `on Handle as "add"` emits
    /// `x.add(…)`, `of Handle as "domElement"` emits `x.domElement`, and
    /// `set Handle as "roughness"` emits `x.roughness = v`. Spelling the
    /// property with `on` would emit `x.domElement()` and call a canvas;
    /// spelling the write with `of` would read a value and discard it.
    ///
    /// **`set` is the word because the language already writes places with
    /// it.** `set depth to 4` is how a signal is given a value, and a
    /// property of a host object is a place in exactly that sense. A new
    /// word here would be a second spelling of one idea, which is what
    /// §4.1 spends its budget avoiding — and it would cost a reserved word
    /// §14G.7.7 has not got.
    ///
    /// All three write the receiver's type out rather than leaving it
    /// implicit. It is the only type a receiver may have, so nothing is
    /// being chosen — but a reader meeting `on as "add"` would have to know
    /// that to read the line, and a reader meeting `on Handle as "add"` is
    /// told.
    fn foreign_source(&mut self) -> Result<zdc_ast::ForeignSource, ParseError> {
        let span = self.peek_span();
        if self.eat(&TokenKind::On) {
            let receiver = self.foreign_receiver("on", "A method is looked up on", "methods")?;
            return Ok(zdc_ast::ForeignSource::Receiver {
                span: span.to(receiver),
            });
        }
        if self.eat(&TokenKind::Of) {
            let receiver = self.foreign_receiver("of", "A property is read off", "properties")?;
            return Ok(zdc_ast::ForeignSource::Property {
                span: span.to(receiver),
            });
        }
        if self.eat(&TokenKind::Set) {
            let receiver =
                self.foreign_receiver("set", "A property is written on", "properties")?;
            return Ok(zdc_ast::ForeignSource::Write {
                span: span.to(receiver),
            });
        }
        self.expect(
            TokenKind::From,
            "to name the module a foreign comes from, `on Handle` for a method, `of Handle` for \
             a property, or `set Handle` to write one",
        )?;
        let module_span = self.peek_span();
        let module = self.expect_text("as the module a foreign comes from")?;
        Ok(zdc_ast::ForeignSource::Import {
            module,
            module_span,
        })
    }

    /// The `Handle` after `on`, `of` or `set`, and its span.
    ///
    /// One function for all three because the refusal is one rule: a host
    /// object is the only thing in the language that has a name looked up
    /// on it at run time, so the word after the leader is `Handle` or the
    /// line does not mean anything. Three copies of that sentence is two
    /// copies that can drift.
    ///
    /// `role` is that sentence in the words of whichever form was written
    /// — "A method is looked up on", "A property is written on" — and it
    /// is a parameter rather than derived from `has`, because a derived
    /// one was already wrong once: it read the plural back and could
    /// distinguish only two forms, so the third would have silently
    /// described itself as the second.
    fn foreign_receiver(
        &mut self,
        leader: &str,
        role: &str,
        has: &str,
    ) -> Result<Span, ParseError> {
        let receiver = self.expect_ident(&format!(
            "after `{leader}`, naming the type {role}. `{}` is the only one",
            zdc_ast::HANDLE_TYPE_NAME
        ))?;
        if receiver.text != zdc_ast::HANDLE_TYPE_NAME {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                format!(
                    "`{leader} {}` names a receiver that cannot exist. {role} a host object at \
                     the call, and `{}` is the language's one name for one (spec §14E.1).",
                    receiver.text,
                    zdc_ast::HANDLE_TYPE_NAME
                ),
                receiver.span,
            )
            .labelled(format!("only a handle has {has}"))
            .suggesting(receiver.span, zdc_ast::HANDLE_TYPE_NAME));
        }
        Ok(receiver.span)
    }

    /// The optional modifier between `gives` and the result type.
    ///
    /// ```text
    /// "gives" [ "pure" | "trusted" ] ("view" | type)
    /// ```
    ///
    /// LL(1) at its decision point: `pure` and `trusted` begin no type, and
    /// the two are alternatives rather than a sequence, so `gives pure
    /// trusted T` does not parse and no consumer has to rule on what it
    /// would have meant.
    ///
    /// **Absent means [`zdc_ast::ForeignGrant::Opaque`]** — §21.9's
    /// default, and the direction of the default is the point: an unmarked
    /// `foreign` is impure, because the failure mode of guessing the other
    /// way is a silent leak.
    fn foreign_result_grant(&mut self) -> zdc_ast::ForeignGrant {
        // `trusted` is a hard keyword (§18.1.1 budgets it); `pure` is soft,
        // so it costs no identifier anywhere outside this one position.
        if self.eat(&TokenKind::Trusted) {
            return zdc_ast::ForeignGrant::Trusted;
        }
        if self.eat_soft(SoftKeyword::Pure) {
            return zdc_ast::ForeignGrant::Pure;
        }
        zdc_ast::ForeignGrant::Opaque
    }

    /// ```text
    /// releaseDecl := "release" IDENT ["with" params] NEWLINE INDENT
    ///                  "gives" type NEWLINE
    ///                  { "trusted" IDENT NEWLINE }
    ///                  [ "limit" NUMBER "per" "visitor" NEWLINE ]
    ///                  stmt+ DEDENT
    /// ```
    ///
    /// Spec §19.1 as amended by §19.10.2. Clause order is fixed, so the
    /// parser never backtracks: `trusted` begins no statement form legal in
    /// a release body, so one token of lookahead ends the endorsement list.
    ///
    /// A release is called exactly like a function, so call sites do not
    /// advertise that a boundary was crossed — the declaration is where the
    /// grant lives and it is conspicuous (§19.1).
    pub fn release_decl(&mut self) -> Result<zdc_ast::ReleaseDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Release, "to begin a release declaration")?;
        let name = self.expect_ident("after `release`")?;

        let mut params = Vec::new();
        if self.eat(&TokenKind::With) {
            loop {
                params.push(self.expect_ident("as a parameter name")?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(
            TokenKind::Newline,
            "after the release's name. Its clauses and body are indented under it",
        )?;
        self.expect(
            TokenKind::Indent,
            "to open a release. `gives` is its first line, and its body follows",
        )?;

        self.expect_soft(
            SoftKeyword::Gives,
            "as a release's first line. A release declares its bandwidth per evaluation",
        )?;
        let gives = self.type_expr()?;
        self.expect(TokenKind::Newline, "after the `gives` type")?;

        // The endorsement clauses. Each names a parameter whose argument may
        // be Untrusted at a call site — REL-ARG's `endorsed(f)`. It grants
        // nothing anywhere else, and it bounds nothing: an endorsed release
        // launders exactly as freely as an unendorsed one (§21.7.9 item 6).
        let mut endorsed = Vec::new();
        while self.eat(&TokenKind::Trusted) {
            endorsed
                .push(self.expect_ident("after `trusted`, naming a parameter of this release")?);
            self.expect(
                TokenKind::Newline,
                "after the endorsed parameter. Each `trusted` clause goes on its own line",
            )?;
        }

        let limit = self.release_limit()?;

        let body = self.block_body("a release's body")?;
        let end = self.last_span();
        self.expect(
            TokenKind::Dedent,
            "to close a release. Its body is the last thing in it",
        )?;

        Ok(zdc_ast::ReleaseDecl {
            name,
            params,
            gives,
            endorsed,
            limit,
            body,
            span: start.to(end),
        })
    }

    /// `testDecl := "test" TEXT NEWLINE INDENT "expect" expr NEWLINE DEDENT`
    /// — issue #169.
    ///
    /// Both words are soft keywords ([`SoftKeyword::Test`],
    /// [`SoftKeyword::Expect`]), so the construct costs nothing against
    /// §14G.7.7's reserved-identifier budget. The positions are
    /// unambiguous: only a declaration begins a top-level line, and only a
    /// statement begins a line inside a block, and neither of those starts
    /// with a bare identifier.
    ///
    /// The claim is a `Text` literal rather than an identifier because it
    /// is a **sentence**. A test named `insertionSortMakes119Comparisons`
    /// is a name that has to be decoded; `"insertion sort makes 119
    /// comparisons"` is the thing itself, and it is what the report prints.
    /// Nothing refers to a test by name, so there is no reason for it to be
    /// spellable as one.
    ///
    /// The body is an indented block with one line in it rather than a
    /// trailing clause on the `test` line, so it reads the way every other
    /// declaration in the language reads and so that a longer expectation
    /// has somewhere to go.
    pub fn test_decl(&mut self) -> Result<zdc_ast::TestDecl, ParseError> {
        let start = self.peek_span();
        self.expect_soft(SoftKeyword::Test, "to begin a test")?;

        let claim_span = self.peek_span();
        let claim = self.expect_text(
            "after `test`, as the claim this test makes. A test is named by the sentence it \
             asserts, in quotes",
        )?;
        // An empty claim is refused here rather than passed on, because the
        // claim is the *only* thing the report has to identify a test by:
        // `test ""` produces a failure nobody can look up. Compare §7.3 —
        // a diagnostic that names nothing is one the reader cannot act on.
        if claim.trim().is_empty() {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                "A test is named by the claim it makes, and this one is blank. Write the \
                 sentence the expectation asserts — `test \"reversing twice gives the original \
                 list\"`.",
                claim_span,
            )
            .labelled("this test has no claim to report"));
        }

        self.expect(
            TokenKind::Newline,
            "after the test's claim. Its expectation is indented under it",
        )?;
        self.expect(
            TokenKind::Indent,
            "to open a test. `expect` and the claim's expression are its one line",
        )?;

        let expect_start = self.peek_span();
        self.expect_soft(
            SoftKeyword::Expect,
            "as a test's one line. A test states exactly one expectation, so that a failure can \
             name the claim and point at the expression that broke it",
        )?;
        let expectation = self.nested(Nesting::Block, |p| p.expr())?;
        let end = self.last_span();
        self.expect(
            TokenKind::Newline,
            "after the expectation. A test has one `expect` line",
        )?;
        // A second line in the block is refused with the reason rather than
        // with `expected the end of an indented block`: the reader wrote a
        // second claim, and what they need to know is that it belongs in a
        // second `test`.
        if !self.at(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                "A test makes one claim, and this one makes a second. Give the second claim its \
                 own `test`, so that a failure names the sentence that is false rather than the \
                 group it was written in.",
                self.peek_span(),
            )
            .labelled("this belongs in a `test` of its own"));
        }
        self.expect(
            TokenKind::Dedent,
            "to close a test. Its expectation is the last thing in it",
        )?;

        Ok(zdc_ast::TestDecl {
            claim,
            claim_span,
            expectation,
            expectation_span: expect_start.to(end),
            span: start.to(end),
        })
    }

    /// `limit NUMBER per visitor` — spec §19.1.
    ///
    /// **What this clause does not do.** It counts evaluations of *this
    /// declaration* against *one anonymous session*. `k` declarations of the
    /// same computation give `kN` evaluations; clearing a cookie mints a
    /// fresh budget; top-level sequencing of releases is legal and
    /// cumulative; and none of it is enforced until `DurableStore` exists.
    /// `limit` is not a disclosure bound and the compiler must not let a
    /// user believe it is (spec §21.8.7, residual risk R3).
    fn release_limit(&mut self) -> Result<Option<zdc_ast::ReleaseLimit>, ParseError> {
        let start = self.peek_span();
        if !self.eat(&TokenKind::Limit) {
            return Ok(None);
        }
        let count_span = self.peek_span();
        let TokenKind::Number(count) = self.peek().clone() else {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                format!(
                    "Expected a whole number after `limit`, found {}. Write `limit 10 per \
                     visitor` — the number is how many times one session may evaluate this \
                     release.",
                    describe_found(self.peek())
                ),
                count_span,
            )
            .labelled("the count of evaluations belongs here"));
        };
        self.bump();
        if count.fract() != 0.0 || count < 0.0 || count > u32::MAX as f64 {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                "A `limit` is a count of evaluations, so it is a whole number of them.",
                count_span,
            )
            .labelled("this is not a whole number of evaluations"));
        }
        self.expect_soft(SoftKeyword::Per, "after the count")?;
        self.expect_soft(
            SoftKeyword::Visitor,
            "as the principal a budget counts against. `visitor` is the only one there is",
        )?;
        let end = self.last_span();
        self.expect(TokenKind::Newline, "after the `limit` clause")?;
        Ok(Some(zdc_ast::ReleaseLimit {
            count: count as u32,
            span: start.to(end),
        }))
    }

    /// The `as` operand of a `foreign`'s `from` clause (spec §14E.1).
    ///
    /// Quoted like a string and constrained like a name. The module
    /// specifier beside it is emitted as a *string literal*, so escaping
    /// is what makes it safe; the export is emitted as *syntax*, into
    /// `import { … } from …`, so nothing escapes it and the only
    /// available answer is to refuse the literal outright.
    ///
    /// Refusing it here rather than at emission is the point. A literal
    /// rejected at parse time is rejected once, for every consumer of the
    /// tree — `zdc check`, the language server, and every pass that never
    /// thought about JavaScript — and the span it points at is the text
    /// the author actually wrote.
    fn foreign_export(&mut self) -> Result<(ExportName, Span), ParseError> {
        let span = self.peek_span();
        let text = self.expect_text("as the symbol within the module")?;
        let Some(export) = ExportName::parse(&text) else {
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                format!(
                    "`{text}` is not a name a JavaScript module can export. Write a plain \
                     identifier — a letter, `_` or `$`, then letters, digits, `_` or `$` — as \
                     in `from \"./sparkline.js\" as \"mount\"`."
                ),
                span,
            )
            .labelled("this goes into the generated `import` clause as syntax"));
        };
        Ok((export, span))
    }

    fn foreign_site(&mut self) -> Result<ForeignSite, ParseError> {
        if self.eat(&TokenKind::Client) {
            return Ok(ForeignSite::Client);
        }
        if self.eat(&TokenKind::Server) {
            return Ok(ForeignSite::Server);
        }
        if self.eat_soft(SoftKeyword::Anywhere) {
            return Ok(ForeignSite::Anywhere);
        }
        Err(ParseError::new(
            codes::ONE_VALID_FORM,
            format!(
                "Expected where this foreign may run, found {}. Write `client`, `server`, or \
                 `anywhere` (spec §14E.2).",
                describe_found(self.peek())
            ),
            self.peek_span(),
        )
        .labelled("which bundles this module may be linked into belongs here"))
    }

    /// The parameter list and block of an effect, with `takes` already eaten.
    ///
    /// The parameters are typed for the reason a `foreign`'s are: they cross
    /// a boundary, so the type cannot be inferred from a caller that is in a
    /// different bundle. Placement is *not* read from them — it is on the
    /// declaration — which is what keeps §14G.6c's rejected semantics 15,
    /// where a call's placement was joined from what the callee's body
    /// touched, from coming back. A `takes` with no parameters is still
    /// well defined, and that is the zero-parameter counterexample which
    /// sank the inference rule.
    fn effect_init(&mut self) -> Result<(Vec<ForeignParam>, zdc_ast::Block), ParseError> {
        let mut params = Vec::new();
        loop {
            let name = self.expect_ident("as a parameter name after `takes`")?;
            self.expect(TokenKind::Is, "after the parameter name")?;
            let trusted = self.eat(&TokenKind::Trusted);
            let ty = self.type_expr()?;
            params.push(ForeignParam {
                span: name.span.to(self.last_span()),
                name,
                trusted,
                ty,
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        // No `expect(Newline)` here: `block` opens with `indented`, which
        // consumes the line break itself. Taking it first is what makes the
        // block look unterminated to the layout rules.
        let body = self.block()?;
        Ok((params, body))
    }

    /// `every "250ms"`, `every frame` or `after "2s"`, with neither word
    /// eaten yet.
    ///
    /// **LL(1) at its decision point, and bounded on both sides.** The five
    /// `init` leaders — `starting`, `from`, `takes`, `every`, `after` — are
    /// distinct, a `type` precedes the slot and no type begins with any of
    /// them. `every` is followed by exactly one of a `Text` literal or the
    /// word `frame`, and nothing else in the language may appear there, so
    /// the two soft keywords never have to be told apart from an ordinary
    /// name by lookahead.
    ///
    /// **No block, and that is the design.** A `takes` clause opens an
    /// indented block because an effect is statements; a clock clause
    /// cannot, because there is nothing for a statement to be. What the
    /// clause declares is the *value* of the cell at every instant, and the
    /// program reads it with `from` like any other signal. That is the
    /// whole of how a periodic event enters the dataflow graph without a
    /// callback: it never becomes control flow at all.
    fn clock_init(&mut self) -> Result<(zdc_ast::Clock, Span), ParseError> {
        let start = self.peek_span();
        let repeating = self.eat_soft(SoftKeyword::Every);
        if !repeating {
            self.expect_soft(SoftKeyword::After, "to begin a delay")?;
        }
        if repeating && self.at_soft(SoftKeyword::Frame) {
            let span = start.to(self.peek_span());
            self.bump();
            return Ok((zdc_ast::Clock::Frame, span));
        }
        let span = self.peek_span();
        let TokenKind::Text(written) = self.peek().clone() else {
            let word = if repeating { "every" } else { "after" };
            // `every frame` is offered only where it is legal. `after
            // frame` would mean "once, at the next repaint", which is a
            // real thing a program might want and is not this construct:
            // it is a mount hook, and inventing one here would be a second
            // meaning for a word that has one.
            let forms = if repeating {
                "Write a quoted duration such as `every \"250ms\"`, or `every frame` for the \
                 display's own refresh."
            } else {
                "Write a quoted duration, such as `after \"2s\"`."
            };
            return Err(ParseError::new(
                codes::ONE_VALID_FORM,
                format!(
                    "Expected a quoted duration after `{word}`, found {}. {forms}",
                    describe_found(self.peek())
                ),
                span,
            )
            .labelled("how often this cell is written belongs here"));
        };
        self.bump();
        let ms = zdc_ast::parse_duration(&written).map_err(|reason| {
            let detail = match reason {
                zdc_ast::DurationError::NoUnit | zdc_ast::DurationError::NoNumber => {
                    "A duration is a number and a unit, with no space: `ms`, `s` or `m`, as in \
                     `\"250ms\"`, `\"1.5s\"` or `\"2m\"`."
                        .to_string()
                }
                // Both bounds name the browser behaviour they exist to
                // avoid, because a reader's first question is "says who".
                zdc_ast::DurationError::TooShort => format!(
                    "The shortest interval is `\"{}ms\"`, because browsers clamp anything \
                     shorter. For motion, write `every frame` — that is the refresh rate the \
                     display actually has.",
                    zdc_ast::SHORTEST_CLOCK_MS as u64
                ),
                zdc_ast::DurationError::TooLong => {
                    "The longest interval is `\"60m\"`, because a browser timer past \
                     `2^31 - 1` milliseconds fires immediately instead of late. \
                     Something genuinely periodic at that scale is a scheduled `server` state, \
                     which this construct is not."
                        .to_string()
                }
            };
            ParseError::new(
                codes::ONE_VALID_FORM,
                format!("`{written}` is not a duration. {detail}"),
                span,
            )
            .labelled("this is the interval, and it is not readable as one")
        })?;
        let clock = if repeating {
            zdc_ast::Clock::Interval(ms)
        } else {
            zdc_ast::Clock::Delay(ms)
        };
        Ok((clock, start.to(span)))
    }

    /// `takes value is Text, index is Whole` or `takes of value is Text`,
    /// or neither — a `foreign` with no parameters, such as the clock.
    fn foreign_params(&mut self) -> Result<(CallForm, Vec<ForeignParam>), ParseError> {
        if !self.eat_soft(SoftKeyword::Takes) {
            return Ok((CallForm::With, Vec::new()));
        }
        let form = if self.eat(&TokenKind::Of) {
            CallForm::Of
        } else {
            CallForm::With
        };
        let mut params = Vec::new();
        loop {
            let name = self.expect_ident("as a parameter name after `takes`")?;
            self.expect(TokenKind::Is, "after the parameter name")?;
            // `takes key is trusted Text` — obligation site A2. In parameter
            // position `trusted` is a *demand* on the caller; on a `release`
            // clause the same word is a *grant*. §19.10.2 is why the two are
            // in different syntactic slots rather than the same one.
            let trusted = self.eat(&TokenKind::Trusted);
            let ty = self.type_expr()?;
            params.push(ForeignParam {
                span: name.span.to(self.last_span()),
                name,
                trusted,
                ty,
            });
            if form == CallForm::Of || !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::Newline, "after the parameter list")?;
        Ok((form, params))
    }
}

/// A route path is also an output path after its leading slash is removed.
/// Keep it canonical here so `out.join(document_path(url))` cannot climb
/// out of the build directory or give two URLs the same file.
fn route_path_is_safe(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    if !path.starts_with('/') || path.ends_with('/') {
        return false;
    }
    path[1..].split('/').all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    })
}

#[cfg(test)]
mod tests {
    use crate::codes;
    use crate::Parser;
    use zdc_ast::{Init, Placement, TypeExpr};

    fn state(src: &str) -> zdc_ast::StateDecl {
        let tokens = zdc_lexer::tokenize(src).expect("lexes");
        Parser::new(tokens).state_decl().expect("parses")
    }

    /// §14G.8 item 14 — the externally-initiated effect (#211).
    ///
    /// The fifth `init` leader, and the first one whose body is a block.
    /// `takes` is reused from `foreign` rather than reserved afresh, so
    /// the construct costs nothing against §14G.7.7's keyword budget.
    #[test]
    fn parses_a_client_initiated_effect() {
        let d = state(
            "state signUp is server Remote of Outcome takes form is Draft\n    give Accepted\n",
        );
        assert_eq!(d.name.text, "signUp");
        assert_eq!(d.placement, Placement::Server);
        let Init::Effect { params, body } = &d.init else {
            panic!("expected an effect init, found {:?}", d.init);
        };
        assert_eq!(params.len(), 1, "one declared parameter");
        assert_eq!(params[0].name.text, "form");
        assert_eq!(body.stmts.len(), 1, "the block is the effect");
    }

    /// #19's timer half. The sixth and seventh `init` leaders, both soft.
    #[test]
    fn parses_the_three_clock_clauses() {
        for (src, expected) in [
            (
                "state elapsed is client Decimal every \"250ms\"\n",
                zdc_ast::Clock::Interval(250.0),
            ),
            (
                "state elapsed is client Decimal every \"1.5s\"\n",
                zdc_ast::Clock::Interval(1_500.0),
            ),
            (
                "state elapsed is client Decimal every \"2m\"\n",
                zdc_ast::Clock::Interval(120_000.0),
            ),
            (
                "state motion is client Decimal every frame\n",
                zdc_ast::Clock::Frame,
            ),
            (
                "state ready is client Truth after \"2s\"\n",
                zdc_ast::Clock::Delay(2_000.0),
            ),
        ] {
            let d = state(src);
            assert_eq!(d.placement, Placement::Client);
            let Init::Clock(clock, _) = d.init else {
                panic!("expected a clock init for `{src}`, found {:?}", d.init);
            };
            assert_eq!(clock, expected, "{src}");
        }
    }

    /// The clause round-trips: what `zdc doc` prints parses back to the
    /// same clock. Without this, `written` is free to drift into a
    /// spelling the grammar does not have.
    #[test]
    fn a_written_clock_clause_parses_back_to_itself() {
        for original in [
            zdc_ast::Clock::Interval(250.0),
            zdc_ast::Clock::Interval(1_500.0),
            zdc_ast::Clock::Interval(120_000.0),
            zdc_ast::Clock::Frame,
            zdc_ast::Clock::Delay(2_000.0),
        ] {
            let src = format!("state c is client Decimal {}\n", original.written());
            let Init::Clock(parsed, _) = state(&src).init else {
                panic!("`{src}` did not parse as a clock");
            };
            assert_eq!(parsed, original, "{src}");
        }
    }

    /// **`every`, `after` and `frame` stay ordinary identifiers**, which
    /// is the whole of what makes them cost nothing against §14G.7.7's
    /// budget. Pinned the way `pure_is_still_an_ordinary_identifier` pins
    /// `pure`: a reserved word here would delete every program with a
    /// field called `frame`.
    #[test]
    fn the_clock_words_are_still_ordinary_identifiers() {
        for word in ["every", "after", "frame"] {
            let d = state(&format!("state {word} is client Whole starting 1\n"));
            assert_eq!(d.name.text, word);
            assert!(matches!(d.init, Init::Starting(_)));
        }
        // And as a type name, which is the position immediately before the
        // slot the words mean something in.
        let d = state("state x is client frame starting 1\n");
        assert!(matches!(d.ty, TypeExpr::Named(ref n) if n.text == "frame"));
    }

    /// A duration is a number and a unit, and every other spelling is
    /// refused with the reason rather than with "unexpected token".
    #[test]
    fn a_duration_that_is_not_one_is_refused_by_name() {
        for (src, expected) in [
            (
                "state c is client Decimal every \"250\"\n",
                "is not a duration",
            ),
            (
                "state c is client Decimal every \"soon\"\n",
                "is not a duration",
            ),
            // Below the clamp browsers apply, and above the range a
            // 32-bit delay holds.
            (
                "state c is client Decimal every \"1ms\"\n",
                "shortest interval",
            ),
            ("state c is client Decimal every \"2m\"\n", ""),
            (
                "state c is client Decimal every \"120m\"\n",
                "longest interval",
            ),
            // `f64::from_str` accepts all three of these and none is a
            // duration.
            (
                "state c is client Decimal every \"infms\"\n",
                "is not a duration",
            ),
            (
                "state c is client Decimal every \"1e9ms\"\n",
                "is not a duration",
            ),
            (
                "state c is client Decimal every \"+5s\"\n",
                "is not a duration",
            ),
            // `after frame` has no meaning; the message says which forms do.
            ("state c is client Truth after frame\n", "quoted duration"),
            ("state c is client Decimal every 250\n", "quoted duration"),
        ] {
            let tokens = zdc_lexer::tokenize(src).expect("lexes");
            let parsed = Parser::new(tokens).state_decl();
            if expected.is_empty() {
                assert!(parsed.is_ok(), "`{src}` should parse");
                continue;
            }
            let error = parsed.expect_err(&format!("`{src}` should be refused"));
            assert_eq!(error.code, codes::ONE_VALID_FORM);
            assert!(
                error.message.contains(expected),
                "`{src}` said: {}",
                error.message
            );
        }
    }

    #[test]
    fn parses_a_durable_source_signal() {
        let d = state("state votes is durable Map of Id to Int starting empty");
        assert_eq!(d.name.text, "votes");
        assert_eq!(d.placement, Placement::Durable);
        assert!(!d.secret);
        assert!(matches!(d.ty, TypeExpr::Map(_, _)));
        assert!(matches!(d.init, Init::Starting(_)));
    }

    /// §14C.3b's fourth placement. It parses exactly like the other
    /// three, which is the point: nothing about the declaration form
    /// changed to admit it.
    #[test]
    fn parses_a_static_signal() {
        let d = state(
            r#"state posts is static List of Post from readPosts with directory is "content""#,
        );
        assert_eq!(d.placement, Placement::Static);
        assert!(matches!(d.ty, TypeExpr::List(_)));
        assert!(matches!(d.init, Init::From(_)));
    }

    #[test]
    fn parses_a_secret_derived_signal() {
        let d = state(r#"secret state apiKey is server Text from environment "STRIPE_KEY""#);
        assert!(d.secret);
        assert_eq!(d.placement, Placement::Server);
        assert!(matches!(d.init, Init::From(_)));
    }

    /// `trusted` sits in the slot `secret` occupies, and the two compose.
    #[test]
    fn parses_a_trusted_signal() {
        let d = state("trusted state orders is durable Map of Text to Order starting empty");
        assert!(d.trusted);
        assert!(!d.secret);
        assert_eq!(d.placement, Placement::Durable);
    }

    #[test]
    fn parses_a_signal_that_is_both_secret_and_trusted() {
        let d = state("secret trusted state orders is durable Text starting \"\"");
        assert!(d.secret);
        assert!(d.trusted);
    }

    /// §4.1 gives one phrasing per construct, so the modifiers have one
    /// order and the other is a parse error naming it.
    #[test]
    fn the_modifiers_have_exactly_one_order() {
        let tokens =
            zdc_lexer::tokenize("trusted secret state orders is durable Text starting \"\"\n")
                .expect("lexes");
        let error = Parser::new(tokens).state_decl().expect_err("is refused");
        assert!(
            error.message.contains("secret trusted"),
            "{}",
            error.message
        );
    }

    #[test]
    fn parses_list_types() {
        let d = state("state ranked is server List of Item starting empty");
        assert!(matches!(d.ty, TypeExpr::List(_)));
    }

    #[test]
    fn parses_a_type_nested_inside_a_type() {
        let d = state("state x is client List of List of Item starting empty");
        let TypeExpr::List(outer) = &d.ty else {
            panic!("expected a list, got {:?}", d.ty)
        };
        let TypeExpr::List(inner) = outer.as_ref() else {
            panic!("expected a list of lists, got {outer:?}")
        };
        let TypeExpr::Named(name) = inner.as_ref() else {
            panic!("expected a named element type, got {inner:?}")
        };
        assert_eq!(name.text, "Item");
    }

    #[test]
    fn parses_a_constructed_type_as_a_map_value() {
        let d = state("state x is durable Map of Id to List of Item starting empty");
        let TypeExpr::Map(key, value) = &d.ty else {
            panic!("expected a map, got {:?}", d.ty)
        };
        assert!(matches!(key.as_ref(), TypeExpr::Named(_)));
        let TypeExpr::List(element) = value.as_ref() else {
            panic!("expected the value type to be a list, got {value:?}")
        };
        assert!(matches!(element.as_ref(), TypeExpr::Named(_)));
    }

    /// `Pair of K to V` is spelled the way `Map of K to V` is, because an
    /// entry of that map is exactly one of these.
    #[test]
    fn parses_a_pair_type_with_the_to_a_map_uses() {
        let d = state("state x is client List of Pair of Text to Whole starting empty");
        let TypeExpr::List(element) = &d.ty else {
            panic!("expected a list, got {:?}", d.ty)
        };
        let TypeExpr::Pair(first, second) = element.as_ref() else {
            panic!("expected a pair element type, got {element:?}")
        };
        assert!(matches!(first.as_ref(), TypeExpr::Named(_)));
        assert!(matches!(second.as_ref(), TypeExpr::Named(_)));
    }

    /// And one phrasing only: the `to` is not optional, and the
    /// diagnostic says which word was expected and where.
    #[test]
    fn a_pair_without_to_is_refused() {
        let err =
            crate::parse("state x is client Pair of Text Whole starting empty\n").unwrap_err();
        assert!(
            err.message.contains("`to`"),
            "expected the missing `to` to be named, got: {}",
            err.message
        );
    }

    #[test]
    fn parses_option_and_remote_types() {
        assert!(matches!(
            state("state x is client Option of Item starting empty").ty,
            TypeExpr::Option(_)
        ));
        assert!(matches!(
            state("state x is server Remote of Item from f").ty,
            TypeExpr::Remote(_)
        ));
    }

    /// A type constructor is a constructor only where a type is expected.
    /// Reserving these four words outright would take `List` away from
    /// every view element and field that wants it.
    #[test]
    fn a_type_constructor_word_is_an_ordinary_name_elsewhere() {
        let program = crate::parse("view\n    List items\n").expect("parses");
        let zdc_ast::Decl::View(view) = &program.decls[0] else {
            panic!("expected a view")
        };
        let zdc_ast::Node::Element(element) = &view.nodes[0] else {
            panic!("expected an element")
        };
        assert_eq!(element.name.text, "List");
    }

    /// A constructor with no `of` names the word the user wrote, so a
    /// dialect's spelling appears in the message rather than the English
    /// one.
    #[test]
    fn a_constructor_without_of_names_the_word_that_was_written() {
        let tokens = zdc_lexer::tokenize("state x is client List starting empty").unwrap();
        let err = Parser::new(tokens).state_decl().unwrap_err();
        assert!(err.message.contains("`of`"), "got: {}", err.message);
        assert!(err.message.contains("after `List`"), "got: {}", err.message);
    }

    // --- record and choice (spec §14B.1 as amended by §14G.1.2) ---

    fn only_decl(src: &str) -> zdc_ast::Decl {
        crate::parse(src).expect("parses").decls.remove(0)
    }

    /// `record … unique` — #2. The identity key that lets `each` reconcile
    /// by identity instead of by position.
    #[test]
    fn a_record_field_can_be_declared_unique() {
        let zdc_ast::Decl::Record(record) =
            only_decl("record Todo\n    unique id is Whole\n    title is Text\n")
        else {
            panic!("expected a record")
        };
        assert_eq!(record.fields[0].name.text, "id");
        assert!(record.fields[0].unique, "`unique id` marks the identity");
        assert_eq!(record.fields[1].name.text, "title");
        assert!(!record.fields[1].unique, "an ordinary field is not one");
    }

    /// Reserving `unique` is what keeps the field production LL(1), which
    /// §14G.7.7 records as the reason `key` was rejected for the job:
    /// `key is Text` is a plausible field and `unique is Text` is not.
    ///
    /// The refusal arrives as `KEYWORD_AS_NAME` rather than as a complaint
    /// about `unique` itself: the word is taken as the modifier it now is,
    /// and the `is` behind it is what fails to be a field name. That is
    /// the honest reading of the line, and it is the price of the word.
    #[test]
    fn unique_is_reserved_so_it_cannot_be_a_field_name() {
        let err = crate::parse("record Todo\n    unique is Text\n").unwrap_err();
        assert_eq!(err.code, codes::KEYWORD_AS_NAME);
    }

    #[test]
    fn a_record_declares_named_fields_in_order() {
        let zdc_ast::Decl::Record(record) =
            only_decl("record Todo\n    id is Text\n    done is Truth\n")
        else {
            panic!("expected a record")
        };
        assert_eq!(record.name.text, "Todo");
        let fields: Vec<&str> = record
            .fields
            .iter()
            .map(|field| field.name.text.as_str())
            .collect();
        assert_eq!(fields, ["id", "done"]);
        assert!(matches!(record.fields[1].ty, TypeExpr::Named(_)));
    }

    #[test]
    fn a_record_field_may_have_a_constructed_type() {
        let zdc_ast::Decl::Record(record) = only_decl("record Board\n    items is List of Item\n")
        else {
            panic!("expected a record")
        };
        assert!(matches!(record.fields[0].ty, TypeExpr::List(_)));
    }

    /// §14G.1.2: a variant carries *named* fields, not one anonymous slot.
    #[test]
    fn a_choice_variant_carries_named_fields() {
        let zdc_ast::Decl::Choice(choice) = only_decl(
            "choice Status\n    Active\n    Archived with reason is Text, moment is Whole\n",
        ) else {
            panic!("expected a choice")
        };
        assert_eq!(choice.name.text, "Status");
        assert_eq!(choice.variants.len(), 2);
        assert!(choice.variants[0].fields.is_empty());
        let fields: Vec<&str> = choice.variants[1]
            .fields
            .iter()
            .map(|field| field.name.text.as_str())
            .collect();
        assert_eq!(fields, ["reason", "moment"]);
    }

    #[test]
    fn a_field_without_is_asks_for_it() {
        let err = crate::parse("record Todo\n    id Text\n").unwrap_err();
        assert!(err.message.contains("`is`"), "got: {}", err.message);
    }

    #[test]
    fn a_record_with_no_fields_asks_for_an_indented_block() {
        let err = crate::parse("record Todo\nstate a is client Text starting \"\"\n").unwrap_err();
        assert!(err.message.contains("indented"), "got: {}", err.message);
    }

    /// A type declaration is a declaration like any other, so the message
    /// for something that is not one names every form.
    #[test]
    fn a_bad_declaration_names_record_and_choice_among_the_forms() {
        let err = crate::parse("nonsense\n").unwrap_err();
        assert!(err.message.contains("`record`"), "got: {}", err.message);
        assert!(err.message.contains("`choice`"), "got: {}", err.message);
        assert!(err.message.contains("`component`"), "got: {}", err.message);
        assert!(err.message.contains("`use`"), "got: {}", err.message);
    }

    // --- components and modules (spec §14D) ---

    #[test]
    fn a_component_declares_parameters_with_the_same_word_a_function_does() {
        let zdc_ast::Decl::Component(component) =
            only_decl("component VoteCard with item, votes\n    Row item.name\n")
        else {
            panic!("expected a component")
        };
        assert_eq!(component.name.text, "VoteCard");
        let names: Vec<&str> = component
            .params
            .iter()
            .map(|param| param.text.as_str())
            .collect();
        assert_eq!(names, ["item", "votes"]);
        assert!(component.children.is_none());
        assert_eq!(component.body.len(), 1);
    }

    /// `children` is a keyword, so it is recorded separately rather than
    /// as a parameter: it is never passed at the call site, and keeping it
    /// out of the list is what lets positional arguments ignore it.
    #[test]
    fn children_is_a_parameter_that_is_not_in_the_parameter_list() {
        let zdc_ast::Decl::Component(component) =
            only_decl("component Panel with label, children\n    Column\n        children\n")
        else {
            panic!("expected a component")
        };
        let names: Vec<&str> = component
            .params
            .iter()
            .map(|param| param.text.as_str())
            .collect();
        assert_eq!(names, ["label"]);
        assert!(component.children.is_some());
    }

    #[test]
    fn a_component_body_mixes_state_and_nodes() {
        let zdc_ast::Decl::Component(component) = only_decl(
            "component Panel with label\n\
             \x20   state open is client Truth starting no\n\
             \x20   Column\n\
             \x20       Text label\n",
        ) else {
            panic!("expected a component")
        };
        assert!(matches!(
            component.body[0],
            zdc_ast::ComponentItem::State(_)
        ));
        assert!(matches!(component.body[1], zdc_ast::ComponentItem::Node(_)));
    }

    #[test]
    fn children_may_only_be_written_once() {
        let err = crate::parse("component P with children, children\n    Column\n").unwrap_err();
        assert!(err.message.contains("once"), "got: {}", err.message);
    }

    #[test]
    fn an_import_names_what_it_brings_in() {
        let zdc_ast::Decl::Use(import) = only_decl("use \"./model\" for Item, Status\n") else {
            panic!("expected an import")
        };
        assert_eq!(import.path, "./model");
        let names: Vec<&str> = import.names.iter().map(|name| name.text.as_str()).collect();
        assert_eq!(names, ["Item", "Status"]);
    }

    #[test]
    fn an_import_without_for_asks_for_it() {
        let err = crate::parse("use \"./model\"\n").unwrap_err();
        assert!(err.message.contains("`for`"), "got: {}", err.message);
    }

    #[test]
    fn an_import_without_a_path_says_what_a_path_looks_like() {
        let err = crate::parse("use model for Item\n").unwrap_err();
        assert!(err.message.contains("quoted path"), "got: {}", err.message);
        assert!(err.message.contains("./model"), "got: {}", err.message);
    }

    /// §14D.1's own `Disclosure` writes `if` in node position, which §4.4's
    /// view grammar did not have.
    #[test]
    fn a_view_node_may_be_a_conditional() {
        let zdc_ast::Decl::View(view) = only_decl(
            "view\n\
             \x20   if open\n\
             \x20       Text \"yes\"\n\
             \x20   otherwise\n\
             \x20       Text \"no\"\n",
        ) else {
            panic!("expected a view")
        };
        let zdc_ast::Node::If(conditional) = &view.nodes[0] else {
            panic!("expected a conditional, got {:?}", view.nodes[0])
        };
        assert_eq!(conditional.then.len(), 1);
        assert_eq!(
            conditional
                .otherwise
                .as_ref()
                .expect("an otherwise branch")
                .len(),
            1
        );
    }

    #[test]
    fn a_conditional_view_node_needs_no_otherwise() {
        let zdc_ast::Decl::View(view) = only_decl("view\n    if open\n        Text \"yes\"\n")
        else {
            panic!("expected a view")
        };
        let zdc_ast::Node::If(conditional) = &view.nodes[0] else {
            panic!("expected a conditional")
        };
        assert!(conditional.otherwise.is_none());
    }

    /// The message for a line that is not a view node names every form the
    /// view grammar has, including the two components added.
    #[test]
    fn an_unknown_view_node_names_if_and_children() {
        let err = crate::parse("view\n    5\n").unwrap_err();
        assert!(err.message.contains("`if`"), "got: {}", err.message);
        assert!(err.message.contains("`children`"), "got: {}", err.message);
    }

    // --- routing (spec §14G.2) ---

    #[test]
    fn a_route_declares_a_url_per_variant() {
        let zdc_ast::Decl::Route(route) =
            only_decl("route Site\n    Home is \"/\"\n    Writing is \"/writing\"\n")
        else {
            panic!("expected a route")
        };
        assert_eq!(route.name.text, "Site");
        let urls: Vec<&str> = route
            .variants
            .iter()
            .map(|variant| variant.path.as_str())
            .collect();
        assert_eq!(urls, ["/", "/writing"]);
        assert!(route.variants[0].params.is_empty());
    }

    /// §14G.1.2 called this right: a route parameter *is* a variant
    /// field, written the same way one is.
    #[test]
    fn a_route_parameter_is_written_like_a_variant_field() {
        let zdc_ast::Decl::Route(route) =
            only_decl("route Site\n    Post is \"/post\" with slug is Text in slugs\n")
        else {
            panic!("expected a route")
        };
        let param = &route.variants[0].params[0];
        assert_eq!(param.name.text, "slug");
        assert!(matches!(param.ty, TypeExpr::Named(_)));
        assert_eq!(
            param.enumerated_in.as_ref().map(|name| name.text.as_str()),
            Some("slugs")
        );
    }

    /// §18.1 semantics 5: a parameter with no `in` is not enumerable, and
    /// that is what makes it untrusted. The grammar has to let it be
    /// written for the distinction to exist.
    #[test]
    fn a_route_parameter_may_have_no_enumeration() {
        let zdc_ast::Decl::Route(route) =
            only_decl("route Site\n    Draft is \"/draft\" with id is Text\n")
        else {
            panic!("expected a route")
        };
        assert!(route.variants[0].params[0].enumerated_in.is_none());
    }

    /// §14G.2 revision 4. `in` takes a bare name: an undelimited
    /// expression before a comma-separated list is swallowed by the list,
    /// so `Archive … in slugsIn with items is posts, page is Whole` would
    /// silently parse as one parameter.
    #[test]
    fn in_is_followed_by_a_name_rather_than_a_call() {
        let error = crate::parse(
            "route Site\n    A is \"/a\" with slug is Text in slugsIn with items is posts\n",
        )
        .expect_err("a call after `in` must not parse");
        assert!(!error.message.is_empty());
    }

    #[test]
    fn a_url_must_be_a_quoted_literal_beginning_with_a_slash() {
        let error = crate::parse("route Site\n    Home is \"home\"\n").unwrap_err();
        assert!(
            error.message.contains("begins with `/`"),
            "{}",
            error.message
        );
        let error = crate::parse("route Site\n    Home is home\n").unwrap_err();
        assert!(error.message.contains("quoted URL"), "{}", error.message);
    }

    /// A parameter is declared after `with`, never spelled inside the
    /// string — the same refusal §6 makes of embedded markup.
    #[test]
    fn a_url_may_not_carry_meta_syntax() {
        for url in ["/post/[slug]", "/post/:slug", "/post/{slug}"] {
            let error = crate::parse(&format!("route Site\n    Post is \"{url}\"\n")).unwrap_err();
            assert!(error.message.contains("`with`"), "{url}: {}", error.message);
        }
    }

    #[test]
    fn static_is_a_placement() {
        let d = state("state slugs is static List of Text starting empty");
        assert_eq!(d.placement, Placement::Static);
        assert!(!d.trusted);
    }

    /// §18.1.1: one word, in the three slots `secret` already occupies.
    /// The two lattices are independent, so both may sit on one
    /// declaration and neither implies the other.
    ///
    /// Independent does not mean interchangeable in the source: §4.1 gives
    /// the pair one order, which `the_modifiers_have_exactly_one_order`
    /// above is about. Written either way they are two separate bits, and
    /// that is what this checks.
    #[test]
    fn trusted_and_secret_are_independent() {
        let trusted_only = state("trusted state a is durable Text starting \"\"");
        assert!(trusted_only.trusted && !trusted_only.secret);
        let secret_only = state("secret state a is durable Text starting \"\"");
        assert!(secret_only.secret && !secret_only.trusted);
        let both = state("secret trusted state a is durable Text starting \"\"");
        assert!(both.trusted && both.secret);
    }

    #[test]
    fn address_is_an_expression() {
        let d = state("state page is client Option of Site starting address");
        assert!(matches!(
            d.init,
            Init::Starting(zdc_ast::Expr::Address { .. })
        ));
    }

    #[test]
    fn a_bad_declaration_names_route_among_the_forms() {
        let err = crate::parse("nonsense\n").unwrap_err();
        assert!(err.message.contains("`route`"), "got: {}", err.message);
    }

    // --- `function … of` and `foreign` (spec §14E.1, §17.4.2) -----------

    #[test]
    fn a_function_may_declare_a_single_of_parameter() {
        let zdc_ast::Decl::Function(function) =
            only_decl("function first of items\n    give items\n")
        else {
            panic!("expected a function")
        };
        assert_eq!(function.name.text, "first");
        assert_eq!(function.form, zdc_ast::CallForm::Of);
        assert_eq!(function.params.len(), 1);
        assert_eq!(function.params[0].text, "items");
    }

    #[test]
    fn a_with_function_keeps_its_form() {
        let zdc_ast::Decl::Function(function) = only_decl("function f with a, b\n    give a\n")
        else {
            panic!("expected a function")
        };
        assert_eq!(function.form, zdc_ast::CallForm::With);
        assert_eq!(function.params.len(), 2);
    }

    #[test]
    fn a_foreign_declares_its_module_symbol_parameters_and_result() {
        let zdc_ast::Decl::Foreign(foreign) = only_decl(
            "foreign split is anywhere\n\
             \x20   from \"zd:text\" as \"split\"\n\
             \x20   takes value is Text, using is Text\n\
             \x20   gives List of Text\n",
        ) else {
            panic!("expected a foreign")
        };
        assert_eq!(foreign.name.text, "split");
        assert_eq!(foreign.site, zdc_ast::ForeignSite::Anywhere);
        assert_eq!(foreign.module(), Some("zd:text"));
        assert_eq!(foreign.export.as_str(), "split");
        assert_eq!(foreign.form, zdc_ast::CallForm::With);
        assert_eq!(foreign.params.len(), 2);
        assert!(matches!(
            foreign.result,
            zdc_ast::ForeignResult::Value(TypeExpr::List(_))
        ));
    }

    /// A `request` declaration keeps its destination as the literal it is,
    /// and its arguments as ordinary named arguments (#19).
    #[test]
    fn a_request_declares_a_destination_arguments_and_a_result() {
        let zdc_ast::Decl::Request(request) = only_decl(
            "request feed is client\n\
             \x20   from  \"https://api.example.org/v1/feed\"\n\
             \x20   with  topic is subject, page is 2\n\
             \x20   gives Text\n",
        ) else {
            panic!("expected a request")
        };
        assert_eq!(request.name.text, "feed");
        assert_eq!(request.placement, Placement::Client);
        assert_eq!(request.destination, "https://api.example.org/v1/feed");
        assert_eq!(request.args.len(), 2);
        assert!(request
            .args
            .iter()
            .all(|arg| matches!(arg, zdc_ast::Arg::Named { .. })));
        assert!(matches!(&request.gives, TypeExpr::Named(name) if name.text == "Text"));
    }

    /// The `with` clause is optional, because a request need send nothing.
    #[test]
    fn a_request_may_take_no_arguments() {
        let zdc_ast::Decl::Request(request) =
            only_decl("request feed is client\n\x20   from  \"/feed.json\"\n\x20   gives Text\n")
        else {
            panic!("expected a request")
        };
        assert!(request.args.is_empty());
        assert_eq!(request.destination, "/feed.json");
    }

    /// **The destination is one quoted literal and nothing else.** Every
    /// line below is a way of computing a URL, and each of them would put
    /// the host a program talks to beyond the reach of the flow pass and
    /// of the emitted policy.
    #[test]
    fn a_computed_destination_is_refused_at_the_token() {
        for written in [
            "apiKey",
            "\"https://api.example.org/?k=\" + apiKey",
            "hostOf with key is apiKey",
            "(\"https://api.example.org\")",
        ] {
            let source =
                format!("request feed is client\n\x20   from  {written}\n\x20   gives Text\n");
            let error =
                crate::parse(&source).expect_err(&format!("`from {written}` must not parse"));
            assert_eq!(error.code, codes::ONE_VALID_FORM);
            assert!(
                error.message.contains("written down rather than computed"),
                "{}",
                error.message
            );
        }
    }

    /// `takes of value is Text` marks the accessor form without
    /// duplicating the parameter list (§17.4.2).
    #[test]
    fn a_foreign_may_take_a_single_of_parameter() {
        let zdc_ast::Decl::Foreign(foreign) = only_decl(
            "foreign trim is anywhere\n\
             \x20   from \"zd:text\" as \"trim\"\n\
             \x20   takes of value is Text\n\
             \x20   gives Text\n",
        ) else {
            panic!("expected a foreign")
        };
        assert_eq!(foreign.form, zdc_ast::CallForm::Of);
        assert_eq!(foreign.params.len(), 1);
    }

    /// The DOM-owning form differs from the value-returning one in the
    /// `gives` clause and nowhere else — which is what makes it one
    /// construct with one phrasing (§4.1) rather than two declarations.
    #[test]
    fn a_foreign_may_give_a_view_instead_of_a_value() {
        let zdc_ast::Decl::Foreign(foreign) = only_decl(
            "foreign Sparkline is client\n\
             \x20   from \"./sparkline.js\" as \"mount\"\n\
             \x20   takes level is Whole\n\
             \x20   gives view\n",
        ) else {
            panic!("expected a foreign")
        };
        assert_eq!(foreign.export.as_str(), "mount");
        assert_eq!(foreign.params.len(), 1);
        assert!(foreign.owns_view());
        assert!(matches!(foreign.result, zdc_ast::ForeignResult::View));
    }

    /// `gives new Handle` — the third form of the one `gives` clause.
    ///
    /// `new` is soft, so it is still an ordinary identifier everywhere
    /// else: the test below writes it as a parameter name in the same
    /// program, which is what `prelude/text.zd`'s `replace with value,
    /// old, new` does today and what reserving the word would have broken.
    #[test]
    fn a_foreign_may_construct_instead_of_calling() {
        let program = crate::parse(
            "foreign vector is client\n\
             \x20   from \"./three.module.js\" as \"Vector3\"\n\
             \x20   takes x is Decimal\n\
             \x20   gives new Handle\n\
             function replace with value, old, new\n\
             \x20   give new\n",
        )
        .expect("`new` is a soft keyword and stays a name outside the `gives` clause");
        let zdc_ast::Decl::Foreign(foreign) = &program.decls[0] else {
            panic!("expected a foreign")
        };
        assert!(foreign.constructs());
        assert!(!foreign.owns_view());
        assert!(matches!(
            foreign.result,
            zdc_ast::ForeignResult::New(TypeExpr::Named(_))
        ));

        let zdc_ast::Decl::Function(function) = &program.decls[1] else {
            panic!("expected a function")
        };
        assert_eq!(function.params[2].text, "new");
    }

    /// `gives nothing` — the fourth form of the one `gives` clause, and
    /// the one that makes `scene.add(mesh)` writable.
    ///
    /// `nothing` is soft, so it stays an ordinary identifier everywhere
    /// else: the same program below names a signal with it, which
    /// reserving the word would have deleted.
    #[test]
    fn a_foreign_may_give_nothing() {
        let program = crate::parse(
            "foreign draw is client\n\
             \x20   on Handle as \"render\"\n\
             \x20   takes r is Handle, world is Handle\n\
             \x20   gives nothing\n\
             state nothing is client Whole starting 0\n",
        )
        .expect("`nothing` is a soft keyword and stays a name outside the `gives` clause");
        let zdc_ast::Decl::Foreign(foreign) = &program.decls[0] else {
            panic!("expected a foreign")
        };
        assert!(matches!(foreign.result, zdc_ast::ForeignResult::Nothing));
        assert!(!foreign.owns_view());
        assert!(!foreign.constructs());

        let zdc_ast::Decl::State(state) = &program.decls[1] else {
            panic!("expected a state declaration")
        };
        assert_eq!(state.name.text, "nothing");
    }

    /// `new` produces a value by definition, so `gives new nothing` is two
    /// answers to one question rather than a combination.
    #[test]
    fn a_foreign_may_not_both_construct_and_give_nothing() {
        let err = crate::parse(
            "foreign make is client\n\
             \x20   from \"./m.js\" as \"Box\"\n\
             \x20   gives new nothing\n",
        )
        .expect_err("`gives new nothing` is two answers to one question");
        assert!(
            err.message.contains("`gives new nothing`"),
            "got: {}",
            err.message
        );
    }

    /// A grant describes a result, and `nothing` is not one — the same
    /// refusal `gives pure view` gets, for the same reason.
    #[test]
    fn a_grant_may_not_describe_an_absent_result() {
        for word in ["pure", "trusted"] {
            let source = format!(
                "foreign draw is client\n\
                 \x20   from \"./m.js\" as \"draw\"\n\
                 \x20   gives {word} nothing\n"
            );
            let err = crate::parse(&source).expect_err("a grant on nothing is refused");
            assert!(
                err.message.contains("a result that does not exist"),
                "got: {}",
                err.message
            );
        }
    }

    /// `on Handle as "add"` — the symbol is a method, and nothing is
    /// imported. It costs no reserved word: `on` is already a keyword and
    /// `as` already names the symbol on the line this replaces.
    #[test]
    fn a_foreign_may_name_a_method_instead_of_a_module() {
        let zdc_ast::Decl::Foreign(foreign) = only_decl(
            "foreign plus is client\n\
             \x20   on Handle as \"add\"\n\
             \x20   takes target is Handle, other is Handle\n\
             \x20   gives Handle\n",
        ) else {
            panic!("expected a foreign")
        };
        assert!(foreign.is_method());
        assert_eq!(foreign.module(), None);
        assert_eq!(foreign.export.as_str(), "add");
        assert_eq!(foreign.params.len(), 2);
    }

    /// A method name reaches the emitted JavaScript after a dot, which is
    /// the same syntactic position an export reaches inside an `import`
    /// clause — so it is the same refusal and the same type carries it.
    #[test]
    fn a_method_name_that_is_not_an_identifier_is_refused() {
        crate::parse(
            "foreign plus is client\n\
             \x20   on Handle as \"add(); evil(); //\"\n\
             \x20   takes target is Handle\n\
             \x20   gives Handle\n",
        )
        .expect_err("a method name that is not an identifier is refused");
    }

    /// `of Handle as "domElement"` — the symbol is a property, read off
    /// the receiver and not called. It costs no reserved word either: `of`
    /// is already a keyword, in `List of Text` and in `length of items`.
    #[test]
    fn a_foreign_may_name_a_property_instead_of_a_method() {
        let zdc_ast::Decl::Foreign(foreign) = only_decl(
            "foreign canvasOf is client\n\
             \x20   of Handle as \"domElement\"\n\
             \x20   takes of r is Handle\n\
             \x20   gives Handle\n",
        ) else {
            panic!("expected a foreign")
        };
        assert!(foreign.is_property());
        assert!(!foreign.is_method());
        assert_eq!(foreign.module(), None);
        assert_eq!(foreign.export.as_str(), "domElement");
        assert_eq!(foreign.params.len(), 1);
    }

    /// `of` is the property leader only on a `foreign`'s source line. It
    /// is a type constructor's word and a call form's word everywhere
    /// else, and both still parse in the same program.
    #[test]
    fn of_still_means_what_it_meant_outside_a_source_line() {
        let program = crate::parse(
            "foreign canvasOf is client\n\
             \x20   of Handle as \"domElement\"\n\
             \x20   takes of r is Handle\n\
             \x20   gives Handle\n\
             state names is client List of Text starting empty\n\
             function first of items\n\
             \x20   give items\n",
        )
        .expect("`of` keeps its other two jobs");
        assert_eq!(program.decls.len(), 3);
    }

    /// A property name reaches the emitted JavaScript after a dot, the
    /// same syntactic position a method name does — so it is the same
    /// refusal and the same type carries it.
    #[test]
    fn a_property_name_that_is_not_an_identifier_is_refused() {
        crate::parse(
            "foreign canvasOf is client\n\
             \x20   of Handle as \"x; evil(); //\"\n\
             \x20   takes of r is Handle\n\
             \x20   gives Handle\n",
        )
        .expect_err("a property name that is not an identifier is refused");
    }

    /// Only a handle has properties a program can name.
    #[test]
    fn a_property_may_only_be_read_off_a_handle() {
        let err = crate::parse(
            "foreign lengthOf is client\n\
             \x20   of Text as \"length\"\n\
             \x20   takes of t is Text\n\
             \x20   gives Whole\n",
        )
        .expect_err("`of Text` names a receiver that cannot exist");
        assert!(
            err.message.contains("names a receiver that cannot exist"),
            "got: {}",
            err.message
        );
    }

    /// `set Handle as "roughness"` — the symbol is a property being
    /// written. It costs no reserved word either: `set` is already the
    /// keyword §8's mutation statement begins with, which is also the
    /// reason it is the right word here.
    #[test]
    fn a_foreign_may_name_a_property_it_writes() {
        let zdc_ast::Decl::Foreign(foreign) = only_decl(
            "foreign setRoughness is client\n\
             \x20   set Handle as \"roughness\"\n\
             \x20   takes surface is Handle, value is Decimal\n\
             \x20   gives nothing\n",
        ) else {
            panic!("expected a foreign")
        };
        assert!(foreign.writes_property());
        assert!(!foreign.is_property());
        assert!(!foreign.is_method());
        assert!(foreign.has_receiver());
        assert_eq!(foreign.module(), None);
        assert_eq!(foreign.export.as_str(), "roughness");
        assert_eq!(foreign.params.len(), 2);
    }

    /// `set` is the write leader only on a `foreign`'s source line. It is
    /// §8's mutation statement everywhere else, and both still parse in
    /// the same program.
    #[test]
    fn set_still_means_what_it_meant_outside_a_source_line() {
        let program = crate::parse(
            "foreign setWidth is client\n\
             \x20   set Handle as \"width\"\n\
             \x20   takes box is Handle, value is Whole\n\
             \x20   gives nothing\n\
             state n is client Whole starting 0\n\
             function reset with x\n\
             \x20   set x to 4\n\
             \x20   give x\n",
        )
        .expect("`set` keeps its other job");
        assert_eq!(program.decls.len(), 3);
    }

    /// A written property's name reaches the emitted JavaScript on the
    /// left of an `=`, which is a syntactic position exactly as a method
    /// name's is — so it is the same refusal and the same type carries it.
    #[test]
    fn a_written_property_name_that_is_not_an_identifier_is_refused() {
        crate::parse(
            "foreign setWidth is client\n\
             \x20   set Handle as \"x = 1; evil(); //\"\n\
             \x20   takes box is Handle, value is Whole\n\
             \x20   gives nothing\n",
        )
        .expect_err("a written property name that is not an identifier is refused");
    }

    /// Only a handle has properties a program can write.
    #[test]
    fn a_property_may_only_be_written_on_a_handle() {
        let err = crate::parse(
            "foreign setLength is client\n\
             \x20   set Text as \"length\"\n\
             \x20   takes t is Text, value is Whole\n\
             \x20   gives nothing\n",
        )
        .expect_err("`set Text` names a receiver that cannot exist");
        assert!(
            err.message
                .contains("A property is written on a host object"),
            "the refusal is in the write's own words, not a method's or a read's; got: {}",
            err.message
        );
    }

    /// Only a handle has methods a program can name.
    #[test]
    fn a_method_may_only_be_looked_up_on_a_handle() {
        let err = crate::parse(
            "foreign plus is client\n\
             \x20   on Whole as \"add\"\n\
             \x20   takes target is Whole\n\
             \x20   gives Whole\n",
        )
        .expect_err("`on Whole` names a receiver that cannot exist");
        assert!(
            err.message.contains("names a receiver that cannot exist"),
            "got: {}",
            err.message
        );
        // The refusal is in the *method's* words. It used to be in the
        // property's: the sentence was re-derived from a plural that was
        // never compared successfully, so every bad receiver — `on`
        // included — was told a property is read off a host object.
        assert!(
            err.message
                .contains("A method is looked up on a host object"),
            "got: {}",
            err.message
        );
    }

    /// The two answers to "what does this hand back" are alternatives.
    #[test]
    fn a_foreign_may_not_both_construct_and_own_a_view() {
        let err = crate::parse(
            "foreign gauge is client\n\
             \x20   from \"./gauge.js\" as \"mount\"\n\
             \x20   gives new view\n",
        )
        .expect_err("`gives new view` is two answers to one question");
        assert!(
            err.message.contains("`gives new view`"),
            "got: {}",
            err.message
        );
    }

    /// A grant on a constructed result would be a way to declare away the
    /// join of what went into it, which is the laundering hole the design
    /// is arranged to avoid (spec §19.2).
    #[test]
    fn a_grant_may_not_describe_a_constructed_handle() {
        for word in ["pure", "trusted"] {
            let source = format!(
                "foreign vector is client\n\
                 \x20   from \"./three.module.js\" as \"Vector3\"\n\
                 \x20   gives {word} new Handle\n"
            );
            let err = crate::parse(&source).expect_err("a grant on a handle is refused");
            assert!(
                err.message.contains("cannot see into"),
                "got: {}",
                err.message
            );
        }
    }

    /// The export reaches the generated `import` as *syntax*, so no
    /// escaping makes an arbitrary string safe there and the only
    /// available answer is to refuse the literal (spec §14E.1).
    #[test]
    fn a_foreign_export_that_closes_the_import_is_refused() {
        let err = crate::parse(
            "foreign parse is anywhere\n\
             \x20   from \"marked\" as \"mount } from 'evil'; //\"\n\
             \x20   gives Text\n",
        )
        .expect_err("an export that is not an identifier is refused");
        let message = format!("{err:?}");
        assert!(
            message.contains("not a name a JavaScript module can export"),
            "the refusal should name the rule, got {message}"
        );
    }

    /// Every character an injection needs is outside the identifier
    /// grammar, so the refusal is one rule rather than a blocklist.
    #[test]
    fn every_export_that_is_not_an_identifier_is_refused() {
        for export in [
            "mount } from 'evil'; //",
            "",
            "1mount",
            "a-b",
            "a b",
            "a\"b",
            "a\nb",
            "a.b",
            "*",
            "default",
        ] {
            let source = format!(
                "foreign parse is anywhere\n\
                 \x20   from \"marked\" as \"{export}\"\n\
                 \x20   gives Text\n"
            );
            let parsed = crate::parse(&source);
            if export == "default" {
                assert!(parsed.is_ok(), "`default` is a bare identifier");
                continue;
            }
            assert!(
                parsed.is_err(),
                "`{export}` is not an identifier and must be refused"
            );
        }
    }

    /// §4.4 writes a callable with no parameters as a bare name, so a
    /// `foreign` may declare none — which is what `clock` is.
    #[test]
    fn a_foreign_may_take_nothing_at_all() {
        let zdc_ast::Decl::Foreign(foreign) = only_decl(
            "foreign clock is anywhere\n\
             \x20   from \"zd:time\" as \"now\"\n\
             \x20   gives Whole\n",
        ) else {
            panic!("expected a foreign")
        };
        assert!(foreign.params.is_empty());
        assert_eq!(
            foreign.result_grant,
            zdc_ast::ForeignGrant::Opaque,
            "an unmarked `gives` line claims nothing, and `clock` is why the default runs this \
             way (§21.9)"
        );
    }

    /// **`gives pure T`, the purity marker (§21.9).**
    ///
    /// The word is soft: it means the marker between `gives` and a type
    /// inside a `foreign` block, and it is an ordinary identifier
    /// everywhere else, so it costs nothing against §14G.7.7's budget.
    ///
    /// The placement is unchanged in all three declarations below, which is
    /// the point of the separation — `is anywhere` cannot decide this and
    /// never could.
    #[test]
    fn a_foreign_may_declare_its_result_pure_or_trusted_or_neither() {
        let head = "foreign f is anywhere\n\
                    \x20   from \"m\" as \"s\"\n\
                    \x20   takes value is Text\n";
        for (gives, expected) in [
            ("    gives Text\n", zdc_ast::ForeignGrant::Opaque),
            ("    gives pure Text\n", zdc_ast::ForeignGrant::Pure),
            ("    gives trusted Text\n", zdc_ast::ForeignGrant::Trusted),
        ] {
            let zdc_ast::Decl::Foreign(foreign) = only_decl(&format!("{head}{gives}")) else {
                panic!("expected a foreign")
            };
            assert_eq!(foreign.result_grant, expected, "parsing `{gives}`");
            assert!(matches!(
                foreign.result,
                zdc_ast::ForeignResult::Value(TypeExpr::Named(_))
            ));
        }
    }

    /// The two markers are alternatives, not a sequence. §4.1 admits one
    /// phrasing per construct, and a declaration claiming both would leave
    /// every consumer to decide which won.
    #[test]
    fn a_foreign_may_not_declare_both_markers() {
        for both in [
            "foreign f is anywhere\n    from \"m\" as \"s\"\n    gives pure trusted Text\n",
            "foreign f is anywhere\n    from \"m\" as \"s\"\n    gives trusted pure Text\n",
        ] {
            assert!(crate::parse(both).is_err(), "`{both}` must not parse");
        }
    }

    /// `pure` outside the one slot that wants it is an ordinary name.
    #[test]
    fn pure_is_still_an_ordinary_identifier() {
        let zdc_ast::Decl::Function(function) = only_decl("function f with pure\n    give pure\n")
        else {
            panic!("expected a function")
        };
        assert_eq!(function.params[0].text, "pure");
    }

    /// `remembered` is a placement in the placement slot and an ordinary
    /// name everywhere else, which is what makes it cost nothing against
    /// §14G.7.7's budget.
    ///
    /// Three positions in one test, because one of them passing would
    /// prove nothing: the slot itself, a signal *named* `remembered`, and
    /// the two together — a `remembered` signal called `remembered`, which
    /// is the case that fails if the parser decides the word before it
    /// looks at where it is.
    #[test]
    fn remembered_is_still_an_ordinary_identifier() {
        let zdc_ast::Decl::State(state) =
            only_decl("state theme is remembered Text starting \"a\"\n")
        else {
            panic!("expected a state")
        };
        assert_eq!(state.placement, Placement::Remembered);
        assert_eq!(state.name.text, "theme");

        let zdc_ast::Decl::State(named) =
            only_decl("state remembered is client Text starting \"a\"\n")
        else {
            panic!("expected a state")
        };
        assert_eq!(named.placement, Placement::Client);
        assert_eq!(named.name.text, "remembered");

        let zdc_ast::Decl::State(both) =
            only_decl("state remembered is remembered Text starting \"a\"\n")
        else {
            panic!("expected a state")
        };
        assert_eq!(both.placement, Placement::Remembered);
        assert_eq!(both.name.text, "remembered");

        let zdc_ast::Decl::Function(function) =
            only_decl("function f with remembered\n    give remembered\n")
        else {
            panic!("expected a function")
        };
        assert_eq!(function.params[0].text, "remembered");
    }

    /// `media` takes a quoted query and nothing else. The literal is what
    /// keeps the set of subscribed queries decidable at compile time, so
    /// the refusal names the shape rather than merely rejecting.
    #[test]
    fn a_media_query_must_be_written_out() {
        let err = crate::parse("state dark is client Truth from media wanted\n").unwrap_err();
        assert!(err.message.contains("quoted"), "got: {}", err.message);
    }

    #[test]
    fn a_foreign_without_a_site_names_the_three_that_exist() {
        let err = crate::parse(
            "foreign clock is somewhere\n\
             \x20   from \"zd:time\" as \"now\"\n\
             \x20   gives Whole\n",
        )
        .unwrap_err();
        assert!(err.message.contains("anywhere"), "got: {}", err.message);
        assert!(err.message.contains("client"), "got: {}", err.message);
    }

    /// The words the `foreign` grammar needs are soft keywords, so a
    /// program may still name something `takes` or `gives`.
    #[test]
    fn the_words_a_foreign_uses_are_still_available_as_names() {
        crate::parse(
            "state gives is client Whole starting 1\nstate takes is client Whole from gives\n",
        )
        .expect("`gives` and `takes` are ordinary names");
    }

    /// This test used to assert that the message named `client` *and*
    /// `durable`, because the message listed all four placements with a
    /// clause of documentation apiece and never said which word the reader
    /// had written. It now asserts the opposite of one half of that: the
    /// four are behind `zdc explain E0101`, the message names `Map`, and
    /// the repair is carried as an edit rather than as a list.
    #[test]
    fn missing_placement_names_the_word_written_and_carries_the_repair() {
        let source = "state votes is Map of Id to Int starting empty";
        let tokens = zdc_lexer::tokenize(source).unwrap();
        let err = Parser::new(tokens).state_decl().unwrap_err();

        assert!(err.message.contains("`Map`"), "got: {}", err.message);
        assert!(
            !err.message.contains("durable"),
            "the four-placement tutorial is still inline: {}",
            err.message
        );
        assert_eq!(err.code, codes::PLACEMENT);

        let suggestion = err.suggestion.expect("the repair is one word");
        assert_eq!(suggestion.replacement, "client ");
        assert_eq!(
            suggestion.span.start as usize,
            source.find("Map").expect("the type is in the source"),
            "the insertion point must be in front of the type"
        );
    }
}
