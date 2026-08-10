use crate::codes;
use crate::cursor::{describe_found, found_word, ParseError, Parser};
use zdc_ast::{
    Arg, Decl, EachNode, Element, Handler, IfNode, Node, NodeArm, NodeArmBody, Program, ViewDecl,
    WhenNode,
};
use zdc_lexer::{Span, TokenKind};

impl Parser {
    /// `view`, optionally carrying the document's metadata.
    ///
    /// `view title is "…", description is "…"` reuses the argument list
    /// every element already has rather than adding a `page` or `document`
    /// declaration. That is one phrasing (§4.1) at a cost of zero reserved
    /// words — and §14G.2's own milestone-7 example writes `state page is
    /// …`, so reserving `page` would have broken the spec's example.
    pub fn view_decl(&mut self) -> Result<ViewDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::View, "to begin a view")?;
        let args: Vec<Arg> = if self.at(&TokenKind::Newline) {
            Vec::new()
        } else {
            self.call_args()?
        };
        let (nodes, end) = self.node_block()?;
        Ok(ViewDecl {
            args,
            nodes,
            span: start.to(end),
        })
    }

    /// A newline-introduced, indented run of view nodes.
    fn node_block(&mut self) -> Result<(Vec<Node>, Span), ParseError> {
        self.indented(
            "before an indented block",
            "to open an indented block",
            |p| p.node(),
        )
    }

    pub(crate) fn node(&mut self) -> Result<Node, ParseError> {
        match self.peek() {
            TokenKind::Each => Ok(Node::Each(self.each_node()?)),
            TokenKind::When => Ok(Node::When(self.when_node()?)),
            TokenKind::If => Ok(Node::If(self.if_node()?)),
            TokenKind::On => Ok(Node::Handler(self.handler()?)),
            TokenKind::Children => {
                let span = self.peek_span();
                self.bump();
                self.expect(
                    TokenKind::Newline,
                    "after `children`. Each view node goes on its own line",
                )?;
                Ok(Node::Children(span))
            }
            TokenKind::Ident(_) => Ok(Node::Element(self.element()?)),
            other => Err(ParseError::new(
                codes::NO_SUCH_CONSTRUCT,
                format!(
                    "Expected a view node, found {}. A view node is an element name, `each`, \
                     `when`, `if`, `on`, or `children`.",
                    describe_found(other)
                ),
                self.peek_span(),
            )
            .labelled(format!(
                "{} cannot begin a view node",
                found_word(other)
                    .map(|word| format!("`{word}`"))
                    .unwrap_or_else(|| describe_found(other))
            ))),
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
                end = crate::expr::arg_span(last);
            }
            parsed
        };

        // Children are present only when the next line is indented further.
        let children = if self.at(&TokenKind::Newline) && self.peek_at(1) == &TokenKind::Indent {
            let (block, block_end) = self.node_block()?;
            end = block_end;
            block
        } else {
            self.expect(
                TokenKind::Newline,
                "after the view node. Each view node goes on its own line",
            )?;
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
        let (body, end) = self.node_block()?;
        Ok(EachNode {
            var,
            iter,
            body,
            span: start.to(end),
        })
    }

    /// `ifNode := "if" expr NEWLINE INDENT node+ DEDENT
    ///             ["otherwise" NEWLINE INDENT node+ DEDENT]`
    ///
    /// The `otherwise` binds to the nearest `if` because it is a sibling
    /// line at the same indentation, so the dangling-else problem the
    /// grammar of a braced language has cannot arise here.
    fn if_node(&mut self) -> Result<IfNode, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::If, "to begin a conditional")?;
        let cond = self.expr()?;
        let (then, mut end) = self.node_block()?;

        let otherwise = if self.at(&TokenKind::Otherwise) {
            self.bump();
            let (nodes, block_end) = self.node_block()?;
            end = block_end;
            Some(nodes)
        } else {
            None
        };

        Ok(IfNode {
            cond,
            then,
            otherwise,
            span: start.to(end),
        })
    }

    fn when_node(&mut self) -> Result<WhenNode, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::When, "to begin a match")?;
        let scrutinee = self.expr()?;
        let (arms, end) =
            self.indented("before the match arms", "to open the match arms", |p| {
                p.node_arm()
            })?;
        Ok(WhenNode {
            scrutinee,
            arms,
            span: start.to(end),
        })
    }

    fn node_arm(&mut self) -> Result<NodeArm, ParseError> {
        let start = self.peek_span();
        let pattern = self.pattern()?;
        let (body, end) = if self.eat(&TokenKind::Show) {
            let element = self.element()?;
            let end = element.span;
            (NodeArmBody::Show(element), end)
        } else {
            let (nodes, end) = self.node_block()?;
            (NodeArmBody::Nodes(nodes), end)
        };
        Ok(NodeArm {
            pattern,
            body,
            span: start.to(end),
        })
    }

    /// `handler := "on" IDENT ["with" IDENT] block`
    ///
    /// The binder is the event the browser raised. `with` already means
    /// "and here are the names" in `function`, `component` and a `when`
    /// pattern, so this production spends no reserved word.
    fn handler(&mut self) -> Result<Handler, ParseError> {
        let start = self.peek_span();
        self.expect(TokenKind::On, "to begin an event handler")?;
        let event = self.expect_ident("after `on`")?;
        let payload = if self.eat(&TokenKind::With) {
            Some(self.expect_ident("after `with`, as the name of the event")?)
        } else {
            None
        };
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(Handler {
            event,
            payload,
            body,
            span,
        })
    }

    /// Every declaration in the file, or the first thing that stopped it.
    ///
    /// Kept beside [`Parser::program_all`] for the callers that want one
    /// error — the tests that assert on a specific message, and anything
    /// that only needs to know whether the file parses. The first element
    /// of the list is the error this used to return, unchanged.
    pub fn program(&mut self) -> Result<Program, ParseError> {
        self.program_all().map_err(|mut errors| errors.remove(0))
    }

    /// Every declaration in the file, or **every** syntax error in it.
    ///
    /// # Why recovery stops at declaration boundaries and nowhere else
    ///
    /// The resolver reports every undefined name in one run, which is the
    /// standard this project holds itself to; the parser stopped at the
    /// first error, so fixing a file with four typos in it was four runs.
    ///
    /// The failure mode of fixing that is worse than the problem. A parser
    /// that resumes in the middle of the construct it just failed to
    /// understand is guessing about what the author meant, and every
    /// guess that is wrong produces a diagnostic about a mistake nobody
    /// made. Three invented errors after one real one is a worse report
    /// than the one real one alone, because now the reader has to work out
    /// which of the four to believe.
    ///
    /// So the recovery here is deliberately the least ambitious one that
    /// is still useful. A ZDeceptron file is a sequence of top-level
    /// declarations, each of which begins with a keyword at the outermost
    /// indentation, on a line of its own. That is a synchronisation point
    /// the parser can recognise without guessing: after a failed
    /// declaration it discards what it was building and skips forward to
    /// the next line that can *only* be a new declaration.
    ///
    /// Two consequences, both of them the point:
    ///
    /// * one declaration produces at most one error, so a single mistake
    ///   is reported once no matter how much text follows it inside the
    ///   same construct;
    /// * a second mistake in a *different* declaration is reported, which
    ///   is the case a reader is fixing a file for.
    ///
    /// Nothing partial reaches the tree. A declaration that failed is
    /// dropped rather than repaired, so no later pass sees a node the
    /// author did not write — which is the other way a recovering parser
    /// invents diagnostics.
    pub fn program_all(&mut self) -> Result<Program, Vec<ParseError>> {
        let mut decls = Vec::new();
        let mut errors = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::Eof) {
                break;
            }
            let before = self.position();
            match self.declaration() {
                Ok(decl) => decls.push(decl),
                Err(error) => {
                    errors.push(error);
                    // A failure that consumed nothing would leave the
                    // cursor where it was and this loop would report the
                    // same error for ever. Progress is guaranteed here
                    // rather than assumed of every reporting site.
                    if self.position() <= before {
                        self.bump();
                    }
                    self.resynchronise();
                }
            }
        }
        if errors.is_empty() {
            Ok(Program { decls })
        } else {
            Err(errors)
        }
    }

    /// Skip to the next line that can only be a new declaration.
    ///
    /// The token stream is laid out (spec §4.2): a line break is a
    /// `Newline`, and a change of indentation is a run of `Indent` or
    /// `Dedent` that a `Newline` precedes. So "at the outermost
    /// indentation, at the start of a line" is decidable from the stream
    /// alone, which is what makes this a synchronisation point rather than
    /// a guess.
    ///
    /// The depth counter is relative and is allowed to go negative: the
    /// failure may have happened several blocks deep, in which case the
    /// `Dedent`s that close those blocks are still ahead. Anything at or
    /// below the depth this started at is therefore a candidate, and the
    /// requirement that the candidate begin a line is what rules out the
    /// keyword-shaped tokens that appear inside one.
    fn resynchronise(&mut self) {
        let mut depth: i32 = 0;
        loop {
            match self.peek() {
                TokenKind::Eof => return,
                TokenKind::Indent => depth += 1,
                TokenKind::Dedent => depth -= 1,
                _ => {
                    if depth <= 0 && self.at_line_start() && self.at_declaration() {
                        return;
                    }
                }
            }
            self.bump();
        }
    }

    /// Whether the token here can begin a top-level declaration.
    fn at_declaration(&self) -> bool {
        matches!(
            self.peek(),
            TokenKind::Secret
                | TokenKind::Trusted
                | TokenKind::State
                | TokenKind::Release
                | TokenKind::Function
                | TokenKind::View
                | TokenKind::Record
                | TokenKind::Choice
                | TokenKind::Component
                | TokenKind::Use
                | TokenKind::Route
        ) || self.at_soft(zdc_lexer::SoftKeyword::Foreign)
            || self.at_soft(zdc_lexer::SoftKeyword::Test)
    }

    /// One declaration, whatever kind it is.
    fn declaration(&mut self) -> Result<Decl, ParseError> {
        let decl = match self.peek() {
            TokenKind::Secret | TokenKind::Trusted | TokenKind::State => {
                Decl::State(self.state_decl()?)
            }
            TokenKind::Release => Decl::Release(self.release_decl()?),
            TokenKind::Function => Decl::Function(self.function_decl()?),
            TokenKind::View => Decl::View(self.view_decl()?),
            TokenKind::Record => Decl::Record(self.record_decl()?),
            TokenKind::Choice => Decl::Choice(self.choice_decl()?),
            TokenKind::Component => Decl::Component(self.component_decl()?),
            TokenKind::Use => Decl::Use(self.use_decl()?),
            TokenKind::Route => Decl::Route(self.route_decl()?),
            _ if self.at_soft(zdc_lexer::SoftKeyword::Foreign) => {
                Decl::Foreign(self.foreign_decl()?)
            }
            // Like `foreign`, a soft keyword in the one position where
            // it cannot be anything else (issue #169).
            _ if self.at_soft(zdc_lexer::SoftKeyword::Test) => Decl::Test(self.test_decl()?),
            other => {
                return Err(ParseError::new(
                    codes::NO_SUCH_CONSTRUCT,
                    format!(
                        "Expected a declaration, found {}. A file contains `use`, `state`, \
                             `record`, `choice`, `route`, `function`, `component`, \
                             `foreign`, `release`, `test`, and `view` declarations.",
                        describe_found(other)
                    ),
                    self.peek_span(),
                )
                .labelled(format!(
                    "{} cannot begin a declaration",
                    found_word(other)
                        .map(|word| format!("`{word}`"))
                        .unwrap_or_else(|| describe_found(other))
                )))
            }
        };
        Ok(decl)
    }
}

#[cfg(test)]
mod tests {
    use zdc_ast::{Arg, Decl, Node, NodeArmBody};

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

    /// `on click with press` binds the event. The binder is optional, so
    /// the old form is still exactly itself rather than a second spelling.
    #[test]
    fn a_handler_may_bind_the_event_it_handles() {
        let p = program(
            "view\n    Button \"go\"\n        on click with press\n            set x to press.x",
        );
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::Element(button) = &v.nodes[0] else {
            panic!("expected an element")
        };
        let Node::Handler(handler) = &button.children[0] else {
            panic!("expected a handler")
        };
        assert_eq!(handler.event.text, "click");
        assert_eq!(
            handler.payload.as_ref().map(|name| name.text.as_str()),
            Some("press")
        );
    }

    #[test]
    fn a_handler_that_binds_nothing_still_parses() {
        let p = program("view\n    Button \"go\"\n        on click\n            add 1 to n");
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::Element(button) = &v.nodes[0] else {
            panic!("expected an element")
        };
        let Node::Handler(handler) = &button.children[0] else {
            panic!("expected a handler")
        };
        assert!(handler.payload.is_none());
    }

    #[test]
    fn parses_when_with_show_and_block_arms() {
        let src = "view\n    when ranked\n        Loading show Spinner\n        Ready with items\n            Row items";
        let p = program(src);
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::When(w) = &v.nodes[0] else {
            panic!("expected a when")
        };
        assert_eq!(w.arms.len(), 2);
    }

    // Regression test: `when` no longer takes a trailing `is` (indentation
    // already delimits the arms), so the scrutinee is a full `expr()` with
    // no restricted binding power. Under the old `when EXPR is` design a
    // comparison scrutinee was unparseable — `is` would be swallowed as the
    // equality operator, leaving the parser expecting a right-hand operand
    // where the arms' newline belongs. Assert it parses cleanly now.
    #[test]
    fn when_scrutinee_may_be_a_comparison() {
        let src =
            "view\n    when a < b\n        Loading show Spinner\n        Ready with items\n            Row items";
        let p = program(src);
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::When(w) = &v.nodes[0] else {
            panic!("expected a when")
        };
        assert!(matches!(
            w.scrutinee,
            zdc_ast::Expr::Binary {
                op: zdc_ast::BinOp::Less,
                ..
            }
        ));
        assert_eq!(w.arms.len(), 2);
    }

    // In a view arm, `show` takes an element, not an expression: this is
    // what lets an arm render something with arguments, e.g.
    // `show ErrorBar message is e.message`. A bare `expr()` would read
    // `ErrorBar` as the whole value and then choke on what follows.
    #[test]
    fn show_renders_an_element_with_named_args() {
        let src = "view\n    when g\n        Failed with e show ErrorBar message is e.message\n";
        let p = program(src);
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::When(w) = &v.nodes[0] else {
            panic!("expected a when")
        };
        let NodeArmBody::Show(element) = &w.arms[0].body else {
            panic!("expected a show arm")
        };
        assert_eq!(element.name.text, "ErrorBar");
        assert_eq!(element.args.len(), 1);
        assert!(matches!(element.args[0], Arg::Named { .. }));
    }

    /// The multi-binder pattern form reaches view arms too, since both
    /// arm flavours parse their pattern with the same function.
    #[test]
    fn a_view_arm_binds_one_name_per_named_field() {
        let src = "view\n    when entry\n        Archived with why, moment show Text why\n";
        let p = program(src);
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::When(w) = &v.nodes[0] else {
            panic!("expected a when")
        };
        let names: Vec<&str> = w.arms[0]
            .pattern
            .bindings
            .iter()
            .map(|i| i.text.as_str())
            .collect();
        assert_eq!(names, ["why", "moment"]);
    }

    #[test]
    fn show_renders_a_bare_element() {
        let src = "view\n    when g\n        Loading show Spinner\n";
        let p = program(src);
        let Decl::View(v) = &p.decls[0] else {
            panic!("expected a view")
        };
        let Node::When(w) = &v.nodes[0] else {
            panic!("expected a when")
        };
        let NodeArmBody::Show(element) = &w.arms[0].body else {
            panic!("expected a show arm")
        };
        assert_eq!(element.name.text, "Spinner");
        assert!(element.args.is_empty());
    }

    // Element arguments follow the name directly; the comma *separates*
    // arguments rather than introducing them, so a leading comma is invalid.
    #[test]
    fn element_args_have_no_leading_comma() {
        let src = "view\n    ErrorBar, message is \"x\"\n";
        let err = crate::parse(src).unwrap_err();
        assert!(err.message.contains("value"), "got: {}", err.message);
    }

    /// §4.4's `node` production is unchanged by §17.4.10: a binding is a
    /// statement, and view position takes nodes. A node is a thing on the
    /// page; a binding puts nothing on the page, so there is no `node` for
    /// it to be. The diagnostic names the four that exist rather than
    /// leaving the programmer to guess.
    #[test]
    fn a_binding_is_not_a_view_node() {
        let src = "state n is client Whole starting 1\nview\n    Column\n        with doubled is n * 2\n        Text doubled\n";
        let err = crate::parse(src).unwrap_err();
        assert!(err.message.contains("view node"), "got: {}", err.message);
        assert!(err.message.contains("`with`"), "got: {}", err.message);
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
