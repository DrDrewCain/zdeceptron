//! The machine-readable form is machine-readable, and stable.
//!
//! Every assertion here goes through `JSON.parse` in a real JavaScript
//! engine rather than through a substring match on the text this crate
//! wrote. An assertion of the second kind would be a second copy of the
//! escaping rules, agreeing with the first copy by construction — which is
//! precisely the failure mode a program whose string literals may contain
//! quotes, backslashes and U+001B needs ruled out.

use boa_engine::{Context, Source};
use zdc_diagnostics::{json, Diagnostic, Level, Suggestion};
use zdc_graph::GraphError;
use zdc_lexer::Span;

/// Parse `line` as JSON and read one path out of it, as text.
///
/// `JSON.parse` is the whole point: it fails on a trailing comma, on an
/// unescaped control character, and on a quote this crate forgot to
/// escape.
fn read(line: &str, path: &str) -> String {
    let mut context = Context::default();
    let script = format!(
        "(() => {{ const d = JSON.parse({}); const v = {path}; \
         return v === null ? 'null' : String(v); }})()",
        js_literal(line)
    );
    match context.eval(Source::from_bytes(script.as_bytes())) {
        Ok(value) => value.display().to_string().trim_matches('"').to_string(),
        Err(error) => panic!("{path} could not be read from {line:?}: {error}"),
    }
}

/// Whether `expr` is true, evaluated against the parsed record.
///
/// Comparing inside the engine rather than reading a value out of it: the
/// values under test contain quotes and backslashes, and a Rust-side
/// comparison would have to undo whatever the engine's own display does to
/// them — which is one more copy of the escaping this file exists to avoid
/// trusting.
fn holds(line: &str, expr: &str) -> bool {
    let mut context = Context::default();
    let script = format!(
        "(() => {{ const d = JSON.parse({}); return ({expr}) === true; }})()",
        js_literal(line)
    );
    match context.eval(Source::from_bytes(script.as_bytes())) {
        Ok(value) => value.display().to_string() == "true",
        Err(error) => panic!("`{expr}` could not be evaluated against {line:?}: {error}"),
    }
}

/// `d.<path>` is exactly this text.
fn says(line: &str, path: &str, expected: &str) -> bool {
    holds(line, &format!("{path} === {}", js_literal(expected)))
}

/// The line as a JavaScript string literal, escaped so that the engine
/// receives the bytes this crate emitted and not an interpretation of
/// them.
fn js_literal(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

const SOURCE: &str =
    "secret state apiKey is server Text from environment \"K\"\n\nview\n    Text apiKey\n";

/// Spans read out of [`SOURCE`] rather than written as numbers, so the
/// line and column this test asserts are the ones a reader would look at.
fn at(needle: &str, occurrence: Occurrence) -> Span {
    let start = match occurrence {
        Occurrence::First => SOURCE.find(needle),
        Occurrence::Last => SOURCE.rfind(needle),
    }
    .expect("the fixture contains the token") as u32;
    Span::new(start, start + needle.len() as u32)
}

enum Occurrence {
    First,
    Last,
}

fn leak() -> Diagnostic {
    let declared = at("apiKey", Occurrence::First);
    let shown = at("apiKey", Occurrence::Last);
    let notes = vec![
        (declared, "declared secret".to_string()),
        (shown, "read in the browser".to_string()),
    ];
    Diagnostic::from(
        GraphError::new("E-IFC-05", "`apiKey` would reach the view", shown).with_notes(notes),
    )
}

#[test]
fn a_diagnostic_is_one_parseable_json_object_on_one_line() {
    let line = json::line(SOURCE, "leak.zd", &leak());

    assert!(line.ends_with('\n'), "the record must be a whole line");
    assert_eq!(
        line.matches('\n').count(),
        1,
        "a record may not run onto a second line: {line:?}"
    );
    assert_eq!(read(&line, "typeof d"), "object");
}

/// The documented keys, with the documented meanings. A consumer written
/// against this shape is what the format is for, so the shape is asserted
/// field by field rather than as "it parses".
#[test]
fn every_documented_key_is_present_and_carries_what_it_claims() {
    let line = json::line(SOURCE, "leak.zd", &leak());

    assert!(says(&line, "d.level", "error"), "{line}");
    assert!(says(&line, "d.code", "E-IFC-05"), "{line}");
    assert!(
        says(
            &line,
            "d.message",
            "[E-IFC-05] `apiKey` would reach the view"
        ),
        "{line}"
    );
    assert!(says(&line, "d.path", "leak.zd"), "{line}");
    assert!(
        says(&line, "d.help", "run 'zdc explain E-IFC-05' for the rule"),
        "{line}"
    );
    assert_eq!(read(&line, "d.suggestion"), "null");

    // Bytes and a place, because the compiler counts one and every
    // consumer counts the other.
    let shown = at("apiKey", Occurrence::Last);
    assert_eq!(read(&line, "d.span.start"), shown.start.to_string());
    assert_eq!(read(&line, "d.span.end"), shown.end.to_string());
    // `    Text apiKey` is the fourth line, and `apiKey` its tenth
    // character. Both counted from the fixture above by hand, because a
    // test that computed them the way the code does would agree with a
    // wrong answer.
    assert_eq!(read(&line, "d.span.line"), "4");
    assert_eq!(read(&line, "d.span.column"), "10");

    // §7.3's path survives serialisation, in order.
    assert_eq!(read(&line, "d.notes.length"), "2");
    assert!(
        says(&line, "d.notes[0].message", "declared secret"),
        "{line}"
    );
    assert!(
        says(&line, "d.notes[1].message", "read in the browser"),
        "{line}"
    );
    assert_eq!(read(&line, "d.notes[0].span.line"), "1");
}

/// Absent is `null`, not missing. A consumer reading `d.code` on a
/// diagnostic with no code should get `null` rather than an exception on
/// the next property access.
#[test]
fn a_key_the_compiler_has_nothing_for_is_null_rather_than_absent() {
    let line = json::line(
        "",
        "nope.zd",
        &Diagnostic::file_error("Could not read nope.zd"),
    );

    for key in ["code", "span", "label", "help", "suggestion"] {
        assert_eq!(
            read(&line, &format!("d.{key}")),
            "null",
            "d.{key} must be present and null"
        );
        assert!(
            holds(&line, &format!("'{key}' in d")),
            "d.{key} must be present at all"
        );
    }
    assert_eq!(
        read(&line, "d.notes.length"),
        "0",
        "notes is [] and not null"
    );
    assert_eq!(read(&line, "d.level"), "error");
}

#[test]
fn a_warning_serialises_as_a_warning() {
    let warning = Diagnostic::from(GraphError::warning(
        "W0331",
        "`unread` is never read",
        Span::new(6, 12),
    ));

    assert_eq!(warning.level, Level::Warning);
    let line = json::line("state unread is client Text\n", "a.zd", &warning);
    assert!(says(&line, "d.level", "warning"), "{line}");
}

#[test]
fn a_suggestion_carries_its_replacement_and_the_range_it_replaces() {
    let src = "state votes is Map of Id to Int starting empty\n";
    let at = src.find("Map").expect("the type is in the source") as u32;
    let diagnostic = Diagnostic {
        message: "no placement.".to_string(),
        span: Some(Span::new(at, at + 3)),
        label: None,
        notes: Vec::new(),
        help: None,
        suggestion: Some(Suggestion {
            span: Span::new(at, at),
            replacement: "client ".to_string(),
        }),
        code: None,
        level: Level::Error,
    };

    let line = json::line(src, "a.zd", &diagnostic);

    assert!(says(&line, "d.suggestion.replacement", "client "), "{line}");
    assert_eq!(read(&line, "d.suggestion.span.start"), "15");
    assert_eq!(read(&line, "d.suggestion.span.end"), "15");
}

/// A message quotes the program, and a `.zd` string literal may contain a
/// quote, a backslash or a newline. The engine decides whether the
/// escaping worked, which is the only judge that is not this crate.
#[test]
fn a_message_containing_json_metacharacters_still_parses_to_itself() {
    let hostile = "a \"quoted\" \\ backslash, a\ttab and a\nnewline";
    let diagnostic = Diagnostic {
        message: hostile.to_string(),
        span: None,
        label: None,
        notes: Vec::new(),
        help: None,
        suggestion: None,
        code: None,
        level: Level::Error,
    };

    let line = json::line("", "a.zd", &diagnostic);

    assert_eq!(
        line.matches('\n').count(),
        1,
        "an embedded newline must not split the record: {line:?}"
    );
    assert!(says(&line, "d.message", hostile), "{line:?}");
}

/// The terminal-injection defence is not lost by going machine-readable.
/// JSON output is read by machines and `cat`ed by people, and a record
/// carrying a live U+001B would clear the screen of whoever looked at it.
#[test]
fn an_escape_sequence_from_the_program_does_not_survive_into_the_record() {
    let diagnostic = Diagnostic {
        message: "`\u{1b}[2J` is not defined.".to_string(),
        span: None,
        label: None,
        notes: Vec::new(),
        help: Some("Try \u{1b}]0;pwned\u{7}.".to_string()),
        suggestion: None,
        code: None,
        level: Level::Error,
    };

    let line = json::line("", "a.zd", &diagnostic);

    assert!(
        !line.contains('\u{1b}'),
        "a raw escape reached the record: {line:?}"
    );
    // Not merely escaped as `\u001b` either: a consumer that decodes the
    // JSON and prints the message would put it straight back.
    assert!(
        !line.contains("\\u001b"),
        "an encoded escape reached the record: {line:?}"
    );
    assert!(
        says(&line, "d.message", "`?[2J` is not defined."),
        "{line:?}"
    );
}

/// The renderer honours the format, so every existing call site emits
/// JSON without knowing the option exists. Asserted through `render`
/// rather than through `json::line`, because that dispatch is the thing
/// that could be wired up wrongly.
#[test]
fn render_emits_the_json_form_when_the_process_is_set_to_it() {
    zdc_diagnostics::set_format(zdc_diagnostics::Format::Json);
    assert_eq!(zdc_diagnostics::format(), zdc_diagnostics::Format::Json);

    let out = zdc_diagnostics::render(SOURCE, "leak.zd", &leak());

    assert!(
        !out.contains('\u{256d}'),
        "the human report was drawn instead:\n{out}"
    );
    assert!(says(&out, "d.code", "E-IFC-05"), "{out}");
}
