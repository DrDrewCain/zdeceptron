use crate::cursor::{describe_found, Nesting, ParseError, Parser};
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
            return Err(ParseError {
                message: "`secret` comes before `trusted`. Write `secret trusted state …`."
                    .to_string(),
                span: self.peek_span(),
            });
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
        } else {
            return Err(ParseError {
                message: "Expected `starting` or `from` after the type. Use `starting` for \
                          state you set directly, and `from` for state computed from other \
                          state."
                    .to_string(),
                span: self.peek_span(),
            });
        };

        let mut end = match &init {
            Init::Starting(e) | Init::From(e) => e.span(),
        };
        // §14C.3b: a `static` value may be *written* as well as read. The
        // clause is keyword-led and trailing, like every other clause of a
        // declaration, so a signal that emits nothing reads exactly as it
        // did before this existed (§4.1).
        let emits = if self.eat(&TokenKind::Emitting) {
            let span = self.peek_span();
            let TokenKind::Text(path) = self.peek().clone() else {
                return Err(ParseError {
                    message: format!(
                        "Expected a quoted path after `emitting`, found {}. Write the file the \
                         value is written to, such as `emitting \"rss.xml\"`.",
                        describe_found(self.peek())
                    ),
                    span,
                });
            };
            self.bump();
            end = span;
            Some(Emitted { path, span })
        } else {
            None
        };

        self.expect(
            TokenKind::Newline,
            "after the declaration. Each declaration goes on its own line",
        )?;
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

    fn placement(&mut self) -> Result<Placement, ParseError> {
        use TokenKind as T;
        let placement = match self.peek() {
            T::Client => Placement::Client,
            T::Static => Placement::Static,
            T::Server => Placement::Server,
            T::Durable => Placement::Durable,
            other => {
                return Err(ParseError {
                    message: format!(
                        "Expected a placement after `is`, found {}. Write `client` for browser \
                         memory, `static` for a value computed once at build time, `server` for \
                         a serverless invocation, or `durable` for persistent storage.",
                        describe_found(other)
                    ),
                    span: self.peek_span(),
                })
            }
        };
        self.bump();
        Ok(placement)
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
        let name = self.expect_ident("as a field name")?;
        self.expect(TokenKind::Is, "after the field name")?;
        let ty = self.type_expr()?;
        let end = self.last_span();
        self.expect(
            TokenKind::Newline,
            "after the field. Each field goes on its own line",
        )?;
        Ok(FieldDecl {
            span: name.span.to(end),
            name,
            ty,
        })
    }

    /// `variant := IDENT ["with" variantField ("," variantField)*] NEWLINE`
    fn variant_decl(&mut self) -> Result<VariantDecl, ParseError> {
        let name = self.expect_ident("as a variant name")?;
        let mut fields = Vec::new();
        if self.eat(&TokenKind::With) {
            loop {
                let field = self.expect_ident("as a field name after `with`")?;
                self.expect(TokenKind::Is, "after the field name")?;
                let ty = self.type_expr()?;
                fields.push(FieldDecl {
                    span: field.span.to(self.last_span()),
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
            return Err(ParseError {
                message: format!(
                    "Expected a quoted path after `use`, found {}. Write `use \"./model\" for \
                     Item` — the path is relative to this file and the `.zd` ending is implied.",
                    describe_found(self.peek())
                ),
                span: path_span,
            });
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
                        return Err(ParseError {
                            message: "`children` is written once. It names the nodes nested under \
                                      this component at its call site, and there is one such run."
                                .to_string(),
                            span,
                        });
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
            return Err(ParseError {
                message: format!(
                    "Expected a quoted URL after `is`, found {}. Write `Home is \"/\"` — the URL \
                     is a literal, and a parameter is written after `with` rather than spelled \
                     inside the string.",
                    describe_found(self.peek())
                ),
                span: path_span,
            });
        };
        self.bump();

        // §6 refuses embedded markup inside a string for the same reason
        // this refuses `[slug]`: a second grammar inside a literal is a
        // grammar nothing checks.
        if path.contains('[') || path.contains(':') || path.contains('{') {
            return Err(ParseError {
                message: "A route's URL is a literal prefix, and a parameter is declared after \
                          `with` rather than written inside the string. Write \
                          `BlogPost is \"/blog\" with slug is Text in postSlugs`."
                    .to_string(),
                span: path_span,
            });
        }
        if !path.starts_with('/') {
            return Err(ParseError {
                message: "A route's URL begins with `/`. Write `\"/blog\"` rather than `\"blog\"`."
                    .to_string(),
                span: path_span,
            });
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
    ///                    "from" STRING "as" EXPORT NEWLINE
    ///                    [ "takes" params | "takes" "of" IDENT "is" type ]
    ///                    "gives" ("view" | type) NEWLINE DEDENT`
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

        self.expect(TokenKind::From, "to name the module a foreign comes from")?;
        let module_span = self.peek_span();
        let module = self.expect_text("as the module a foreign comes from")?;
        self.expect_soft(SoftKeyword::As, "to name the symbol within the module")?;
        let (export, export_span) = self.foreign_export()?;
        self.expect(TokenKind::Newline, "after the module line")?;

        let (form, params) = self.foreign_params()?;

        self.expect_soft(SoftKeyword::Gives, "to declare what a foreign gives back")?;
        let result_span = self.peek_span();
        // `view` in this position is the DOM-owning form. It is the same
        // word the `view` declaration uses and it is unambiguous here,
        // because no `type` begins with it — the same licence §4.4 already
        // grants `is` in three roles.
        let result = if self.eat(&TokenKind::View) {
            ForeignResult::View
        } else {
            ForeignResult::Value(self.type_expr()?)
        };
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
            module,
            module_span,
            export,
            export_span,
            form,
            params,
            result,
            result_span: result_span.to(end),
            span: start.to(end),
        })
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
            return Err(ParseError {
                message: format!(
                    "`{text}` is not a name a JavaScript module can export. The `as` operand of \
                     a `foreign` is an identifier rather than a text literal: it is written into \
                     the generated `import` clause as syntax, so no escaping can make an \
                     arbitrary string safe there (spec §14E.1). Write a plain identifier — a \
                     letter, `_` or `$`, then letters, digits, `_` or `$` — as in `from \
                     \"./sparkline.js\" as \"mount\"`."
                ),
                span,
            });
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
        Err(ParseError {
            message: format!(
                "Expected where this foreign may run, found {}. Write `client`, `server`, or \
                 `anywhere` (spec §14E.2).",
                describe_found(self.peek())
            ),
            span: self.peek_span(),
        })
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
            let ty = self.type_expr()?;
            params.push(ForeignParam {
                span: name.span.to(self.last_span()),
                name,
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

#[cfg(test)]
mod tests {
    use crate::Parser;
    use zdc_ast::{Init, Placement, TypeExpr};

    fn state(src: &str) -> zdc_ast::StateDecl {
        let tokens = zdc_lexer::tokenize(src).expect("lexes");
        Parser::new(tokens).state_decl().expect("parses")
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
        assert_eq!(foreign.module, "zd:text");
        assert_eq!(foreign.export.as_str(), "split");
        assert_eq!(foreign.form, zdc_ast::CallForm::With);
        assert_eq!(foreign.params.len(), 2);
        assert!(matches!(
            foreign.result,
            zdc_ast::ForeignResult::Value(TypeExpr::List(_))
        ));
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

    #[test]
    fn missing_placement_names_the_valid_forms() {
        let tokens = zdc_lexer::tokenize("state votes is Map of Id to Int starting empty").unwrap();
        let err = Parser::new(tokens).state_decl().unwrap_err();
        assert!(err.message.contains("client"), "got: {}", err.message);
        assert!(err.message.contains("durable"), "got: {}", err.message);
    }
}
