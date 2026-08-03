use crate::cursor::{describe_found, Nesting, ParseError, Parser};
use zdc_ast::{
    ChoiceDecl, Emitted, FieldDecl, FunctionDecl, Init, Placement, RecordDecl, StateDecl, TypeExpr,
    VariantDecl,
};
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
    /// for something that is not one names all five forms.
    #[test]
    fn a_bad_declaration_names_record_and_choice_among_the_forms() {
        let err = crate::parse("nonsense\n").unwrap_err();
        assert!(err.message.contains("`record`"), "got: {}", err.message);
        assert!(err.message.contains("`choice`"), "got: {}", err.message);
    }

    #[test]
    fn missing_placement_names_the_valid_forms() {
        let tokens = zdc_lexer::tokenize("state votes is Map of Id to Int starting empty").unwrap();
        let err = Parser::new(tokens).state_decl().unwrap_err();
        assert!(err.message.contains("client"), "got: {}", err.message);
        assert!(err.message.contains("durable"), "got: {}", err.message);
    }
}
