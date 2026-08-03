use zdc_ast::Program;
use zdc_hir::{Arena, Local, LocalId};
use zdc_lexer::Span;
use zdc_resolve::{collect, Scopes};

fn program(src: &str) -> Program {
    zdc_parser::parse(src).expect("source parses")
}

fn local(arena: &mut Arena<LocalId, Local>, name: &str) -> LocalId {
    arena.alloc(Local {
        name: name.into(),
        span: Span::new(0, 0),
    })
}

#[test]
fn an_empty_program_collects_into_an_empty_table() {
    let table = collect(&Program { decls: Vec::new() }).expect("collects");

    assert!(table.is_empty());
    assert_eq!(table.len(), 0);
    assert_eq!(table.view, None);
}

#[test]
fn collected_indices_match_source_declaration_order() {
    let parsed = program(concat!(
        "state count is client Whole starting 0\n",
        "view\n",
        "    Text count\n",
        "function increment with value\n",
        "    give value + 1\n",
    ));

    let table = collect(&parsed).expect("collects");

    assert_eq!(table.lookup("count"), Some(0));
    assert_eq!(table.view, Some(1));
    assert_eq!(table.lookup("increment"), Some(2));
    assert_eq!(table.len(), 2, "the view is tracked separately");
}

#[test]
fn collection_reports_mixed_conflicts_in_one_pass() {
    let parsed = program(concat!(
        "state item is client Text starting \"\"\n",
        "state item is client Text starting \"\"\n",
        "view\n",
        "    Column\n",
        "view\n",
        "    Row\n",
        "function item\n",
        "    give empty\n",
    ));

    let errors = collect(&parsed).unwrap_err();

    assert_eq!(errors.len(), 3);
    assert_eq!(
        errors
            .iter()
            .filter(|error| error.message.contains("already declared"))
            .count(),
        2
    );
    assert_eq!(
        errors
            .iter()
            .filter(|error| error.message.contains("one `view`"))
            .count(),
        1
    );
}

#[test]
fn popping_an_inner_scope_restores_the_outer_binding() {
    let mut locals = Arena::new();
    let outer = local(&mut locals, "item");
    let middle = local(&mut locals, "item");
    let inner = local(&mut locals, "item");
    let mut scopes = Scopes::new();

    scopes.push();
    scopes.declare("item", outer);
    scopes.push();
    scopes.declare("item", middle);
    scopes.push();
    scopes.declare("item", inner);
    assert_eq!(scopes.lookup("item"), Some(inner));

    scopes.pop();
    assert_eq!(scopes.lookup("item"), Some(middle));
    scopes.pop();
    assert_eq!(scopes.lookup("item"), Some(outer));
    scopes.pop();
    assert_eq!(scopes.lookup("item"), None);
}

#[test]
fn names_in_sibling_scopes_do_not_leak() {
    let mut locals = Arena::new();
    let first = local(&mut locals, "first");
    let second = local(&mut locals, "second");
    let mut scopes = Scopes::default();

    scopes.push();
    scopes.declare("first", first);
    scopes.pop();
    scopes.push();
    scopes.declare("second", second);

    assert_eq!(scopes.lookup("first"), None);
    assert_eq!(scopes.lookup("second"), Some(second));
}
