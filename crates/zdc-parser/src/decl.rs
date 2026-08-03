use crate::cursor::{describe_found, ParseError, Parser};
use zdc_ast::{FunctionDecl, Init, Placement, StateDecl, TypeExpr};
use zdc_lexer::TokenKind;

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
        let name = self.expect_ident("as a type")?;
        match name.text.as_str() {
            "List" => {
                self.expect(TokenKind::Of, "after `List`")?;
                Ok(TypeExpr::List(Box::new(self.type_expr()?)))
            }
            "Option" => {
                self.expect(TokenKind::Of, "after `Option`")?;
                Ok(TypeExpr::Option(Box::new(self.type_expr()?)))
            }
            "Remote" => {
                self.expect(TokenKind::Of, "after `Remote`")?;
                Ok(TypeExpr::Remote(Box::new(self.type_expr()?)))
            }
            "Map" => {
                self.expect(TokenKind::Of, "after `Map`")?;
                let key = self.type_expr()?;
                self.expect(TokenKind::To, "between the key and value types of a `Map`")?;
                let value = self.type_expr()?;
                Ok(TypeExpr::Map(Box::new(key), Box::new(value)))
            }
            _ => Ok(TypeExpr::Named(name)),
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
    fn missing_placement_names_the_valid_forms() {
        let tokens = zdc_lexer::tokenize("state votes is Map of Id to Int starting empty").unwrap();
        let err = Parser::new(tokens).state_decl().unwrap_err();
        assert!(err.message.contains("client"), "got: {}", err.message);
        assert!(err.message.contains("durable"), "got: {}", err.message);
    }
}
