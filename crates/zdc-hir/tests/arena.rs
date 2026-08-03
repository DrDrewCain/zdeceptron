use zdc_hir::{Arena, ArenaId, DefId, ExprId};

#[test]
fn allocation_returns_dense_typed_ids() {
    let mut arena: Arena<DefId, &str> = Arena::new();
    let first = arena.alloc("first");
    let second = arena.alloc("second");

    assert_eq!(first.index(), 0);
    assert_eq!(second.index(), 1);
    assert_eq!(arena.len(), 2);
    assert_eq!(arena[first], "first");
    assert_eq!(arena[second], "second");
}

#[test]
fn values_can_be_updated_through_their_id() {
    let mut arena: Arena<ExprId, u32> = Arena::new();
    let expression = arena.alloc(10);

    *arena.get_mut(expression) = 20;

    assert_eq!(*arena.get(expression), 20);
}

#[test]
fn iteration_preserves_ids_and_allocation_order() {
    let mut arena: Arena<DefId, &str> = Arena::new();
    arena.alloc("a");
    arena.alloc("b");

    let entries: Vec<_> = arena
        .iter()
        .map(|(id, value)| (id.index(), *value))
        .collect();

    assert_eq!(entries, [(0, "a"), (1, "b")]);
}
