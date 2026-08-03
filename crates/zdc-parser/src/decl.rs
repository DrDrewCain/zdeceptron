use crate::cursor::{describe_found, Nesting, ParseError, Parser};
use zdc_ast::{FunctionDecl, Init, Placement, StateDecl, TypeExpr};
use zdc_lexer::{TokenKind, TypeCtor};

impl Parser {
    pub fn state_decl(&mut self) -> Result<StateDecl, ParseError> {
        let start = self.peek_span();
        let secret = self.eat(&TokenKind::Secret);
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

        let end = match &init {
            Init::Starting(e) | Init::From(e) => e.span(),
        };
        self.expect(
            TokenKind::Newline,
            "after the declaration. Each declaration goes on its own line",
        )?;
        Ok(StateDecl {
            secret,
            name,
            placement,
            ty,
            init,
            span: start.to(end),
        })
    }

    fn placement(&mut self) -> Result<Placement, ParseError> {
        use TokenKind as T;
        let placement = match self.peek() {
            T::Client => Placement::Client,
            T::Server => Placement::Server,
            T::Durable => Placement::Durable,
            other => {
                return Err(ParseError {
                    message: format!(
                        "Expected a placement after `is`, found {}. Write `client` for browser \
                         memory, `server` for a serverless invocation, or `durable` for \
                         persistent storage.",
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

    pub fn function_decl(&mut self) -> Result<FunctionDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Function, "to begin a function")?;
        let name = self.expect_ident("after `function`")?;

        let mut params = Vec::new();
        if self.eat(&TokenKind::With) {
            loop {
                params.push(self.expect_ident("as a parameter name")?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }

        let body = self.block()?;
        let span = start.to(body.span);
        Ok(FunctionDecl {
            name,
            params,
            body,
            span,
        })
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

    #[test]
    fn parses_a_secret_derived_signal() {
        let d = state(r#"secret state apiKey is server Text from environment "STRIPE_KEY""#);
        assert!(d.secret);
        assert_eq!(d.placement, Placement::Server);
        assert!(matches!(d.init, Init::From(_)));
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

    #[test]
    fn missing_placement_names_the_valid_forms() {
        let tokens = zdc_lexer::tokenize("state votes is Map of Id to Int starting empty").unwrap();
        let err = Parser::new(tokens).state_decl().unwrap_err();
        assert!(err.message.contains("client"), "got: {}", err.message);
        assert!(err.message.contains("durable"), "got: {}", err.message);
    }
}
