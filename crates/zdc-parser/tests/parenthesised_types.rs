//! Types are not parenthesised, and the message says so — issue #256.
//!
//! ```zd
//! state m is client Map of Text to (Option of Whole) starting empty
//! ```
//!
//! used to be refused with *"Expected a name as a type, found `(`"* and a
//! caret labelled *"a name belongs here"*. Both true, and both unhelpful:
//! the unparenthesised form parses, so the type is expressible and the
//! parentheses are the only thing being refused. A reader who wrote them
//! wrote them to **group**, and being told a name was expected reads as
//! though the type were wrong rather than the grouping.
//!
//! The other option was to accept redundant parentheses and ignore them.
//! That is a grammar change, and §4.1 admits one phrasing per construct —
//! two spellings of one type is exactly what that rule exists to prevent.
//! So the message states the rule and shows the form that works.

/// The message names the rule and demonstrates it, rather than describing
/// the token that was found.
#[test]
fn a_parenthesised_type_is_refused_by_naming_the_rule() {
    let src = "state m is client Map of Text to (Option of Whole) starting empty\n";
    let err = zdc_parser::parse(src).unwrap_err();

    assert!(
        err.message.contains("not parenthesised"),
        "the message should state the rule, and said: {}",
        err.message
    );
    assert!(
        err.message.contains("Map of Text to Option of Whole"),
        "the message should show the form that works, and said: {}",
        err.message
    );
    // The old message. Asserted absent because "a name belongs here" is
    // what made this worth an issue: it is true of the position and says
    // nothing about the mistake.
    assert!(
        !err.message.contains("Expected a name as a type"),
        "the generic name-expected message is back: {}",
        err.message
    );
}

/// The caret points at the parenthesis, which is the character to delete.
#[test]
fn the_caret_covers_the_parenthesis() {
    let src = "state m is client Map of Text to (Option of Whole) starting empty\n";
    let err = zdc_parser::parse(src).unwrap_err();
    let span = err.span;
    assert_eq!(
        &src[span.start as usize..span.end as usize],
        "(",
        "the caret belongs under the parenthesis"
    );
}

/// **Every position a type can appear in**, because the fix is in the one
/// function that parses a type and this is what says so. #256 asked
/// whether the same applied elsewhere; it was found on a `state`.
#[test]
fn every_type_position_refuses_parentheses_the_same_way() {
    let sources = [
        ("a state's type", "state m is client (Whole) starting 1\n"),
        ("a record field", "record Book\n    title is (Text)\n"),
        (
            "a route parameter",
            "route Site\n    Home is \"/\" with slug is (Text) in names\n",
        ),
        (
            "a foreign's parameter",
            "foreign send is client\n    from \"./net.js\" as \"send\"\n    takes body is (Text)\n    gives nothing\n",
        ),
    ];

    for (position, src) in sources {
        let err = zdc_parser::parse(src)
            .err()
            .unwrap_or_else(|| panic!("{position}: parentheses were accepted"));
        assert!(
            err.message.contains("not parenthesised"),
            "{position}: refused with the wrong message: {}",
            err.message
        );
    }
}

/// The form the message recommends is the form that parses, so the advice
/// is checked rather than asserted.
#[test]
fn the_recommended_form_parses() {
    let src = "state m is client Map of Text to Option of Whole starting empty\n\
               \nview\n    Column\n        Text \"hi\"\n";
    assert!(
        zdc_parser::parse(src).is_ok(),
        "the message recommends a form that does not parse"
    );
}
