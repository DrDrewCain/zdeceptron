use crate::cursor::{ParseError, Parser};
use zdc_ast::{
    Arg, Decl, EachNode, Element, Handler, Node, NodeArm, NodeArmBody, Program, ViewDecl, WhenNode,
};
use zdc_lexer::TokenKind;

impl Parser {
    pub fn view_decl(&mut self) -> Result<ViewDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::View, "to begin a view")?;
        let nodes = self.node_block()?;
        let end = self.peek_span();
        Ok(ViewDecl {
            nodes,
            span: start.to(end),
        })
    }

    /// A newline-introduced, indented run of view nodes.
    fn node_block(&mut self) -> Result<Vec<Node>, ParseError> {
        self.expect(TokenKind::Newline, "before an indented block")?;
        self.expect(TokenKind::Indent, "to open an indented block")?;

        let mut nodes = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::Dedent) || self.at(&TokenKind::Eof) {
                break;
            }
            nodes.push(self.node()?);
        }

        self.eat(&TokenKind::Dedent);
        Ok(nodes)
    }

    fn node(&mut self) -> Result<Node, ParseError> {
        match self.peek() {
            TokenKind::Each => Ok(Node::Each(self.each_node()?)),
            TokenKind::When => Ok(Node::When(self.when_node()?)),
            TokenKind::On => Ok(Node::Handler(self.handler()?)),
            TokenKind::Ident(_) => Ok(Node::Element(self.element()?)),
            other => Err(ParseError {
                message: format!(
                    "Expected a view node, found {other:?}. A view node is an element name, \
                     `each`, `when`, or `on`."
                ),
                span: self.peek_span(),
            }),
        }
    }

    fn element(&mut self) -> Result<Element, ParseError> {
        let name = self.expect_ident("as an element name")?;
        let mut end = name.span;

        let args: Vec<Arg> = if self.at(&TokenKind::Newline) {
            Vec::new()
        } else {
            let parsed = self.call_args()?;
            if let Some(last) = parsed.last() {
                end = match last {
                    Arg::Positional(e) => e.span(),
                    Arg::Named { value, .. } => value.span(),
                };
            }
            parsed
        };

        // Children are present only when the next line is indented further.
        let children = if self.at(&TokenKind::Newline) && self.peek_at(1) == &TokenKind::Indent {
            let block = self.node_block()?;
            end = self.peek_span();
            block
        } else {
            Vec::new()
        };

        // Compute the span before moving `name` into the struct.
        let span = name.span.to(end);
        Ok(Element {
            name,
            args,
            children,
            span,
        })
    }

    fn each_node(&mut self) -> Result<EachNode, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::Each, "to begin a loop")?;
        let var = self.expect_ident("after `each`")?;
        self.expect(TokenKind::In, "after the loop name")?;
        let iter = self.expr()?;
        let body = self.node_block()?;
        let end = self.peek_span();
        Ok(EachNode {
            var,
            iter,
            body,
            span: start.to(end),
        })
    }

    fn when_node(&mut self) -> Result<WhenNode, ParseError> {
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
                NodeArmBody::Show(self.expr()?)
            } else {
                NodeArmBody::Nodes(self.node_block()?)
            };
            let end = self.peek_span();
            arms.push(NodeArm {
                pattern,
                body,
                span: arm_start.to(end),
            });
        }

        let end = self.peek_span();
        self.eat(&TokenKind::Dedent);
        Ok(WhenNode {
            scrutinee,
            arms,
            span: start.to(end),
        })
    }

    fn handler(&mut self) -> Result<Handler, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::On, "to begin an event handler")?;
        let event = self.expect_ident("after `on`")?;
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(Handler { event, body, span })
    }

    pub fn program(&mut self) -> Result<Program, ParseError> {
        let mut decls = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::Eof) {
                break;
            }
            let decl = match self.peek() {
                TokenKind::Secret | TokenKind::State => Decl::State(self.state_decl()?),
                TokenKind::Function => Decl::Function(self.function_decl()?),
                TokenKind::View => Decl::View(self.view_decl()?),
                other => {
                    return Err(ParseError {
                        message: format!(
                            "Expected a declaration, found {other:?}. A file contains `state`, \
                             `function`, and `view` declarations."
                        ),
                        span: self.peek_span(),
                    })
                }
            };
            decls.push(decl);
        }
        Ok(Program { decls })
    }
}

#[cfg(test)]
mod tests {
    use zdc_ast::{Arg, Decl, Node};

    fn program(src: &str) -> zdc_ast::Program {
        crate::parse(src).expect("parses")
    }

    #[test]
    fn parses_nested_elements_with_named_args() {
        let p = program("view\n    Column\n        Input query, hint is \"search\"");
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::Element(column) = &v.nodes[0] else {
            panic!("expected an element")
        };
        assert_eq!(column.name.text, "Column");

        let Node::Element(input) = &column.children[0] else {
            panic!("expected a child")
        };
        assert_eq!(input.args.len(), 2);
        assert!(matches!(input.args[0], Arg::Positional(_)));
        assert!(matches!(input.args[1], Arg::Named { .. }));
    }

    #[test]
    fn parses_a_handler_inside_an_element() {
        let p = program("view\n    Row\n        on click\n            add 1 to votes at id");
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::Element(row) = &v.nodes[0] else {
            panic!("expected an element")
        };
        assert!(matches!(row.children[0], Node::Handler(_)));
    }

    #[test]
    fn parses_when_with_show_and_block_arms() {
        let src = "view\n    when ranked is\n        Loading show Spinner\n        Ready with items\n            Row items";
        let p = program(src);
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::When(w) = &v.nodes[0] else {
            panic!("expected a when")
        };
        assert_eq!(w.arms.len(), 2);
    }

    #[test]
    fn parses_a_program_with_all_three_declaration_kinds() {
        let src =
            "state q is client Text starting \"\"\nfunction f with a\n    give a\nview\n    Column";
        let p = program(src);
        assert_eq!(p.decls.len(), 3);
        assert!(matches!(p.decls[0], Decl::State(_)));
        assert!(matches!(p.decls[1], Decl::Function(_)));
        assert!(matches!(p.decls[2], Decl::View(_)));
    }
}
