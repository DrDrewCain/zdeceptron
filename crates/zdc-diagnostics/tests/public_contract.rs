use zdc_diagnostics::{render, Diagnostic};
use zdc_lexer::Span;

fn strip_ansi(input: &str) -> String {
    let mut plain = String::new();
    let mut chars = input.chars();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            for escaped in chars.by_ref() {
                if escaped == 'm' {
                    break;
                }
            }
        } else {
            plain.push(character);
        }
    }
    plain
}

#[test]
fn parse_error_conversion_preserves_message_and_span() {
    let error = zdc_parser::ParseError {
        message: "Expected a value.".into(),
        span: Span::new(4, 9),
    };

    let diagnostic = Diagnostic::from(error);

    assert_eq!(diagnostic.message, "Expected a value.");
    assert_eq!(diagnostic.span, Some(Span::new(4, 9)));
    assert_eq!(diagnostic.help, None);
}

#[test]
fn file_error_factory_creates_a_location_free_diagnostic() {
    let diagnostic = Diagnostic::file_error("Could not read source.zd");

    assert_eq!(diagnostic.message, "Could not read source.zd");
    assert_eq!(diagnostic.span, None);
    assert_eq!(diagnostic.help, None);
}

#[test]
fn location_free_rendering_includes_optional_help_without_a_caret() {
    let diagnostic = Diagnostic {
        message: "Could not decode source.zd".into(),
        span: None,
        help: Some("Save the file as UTF-8.".into()),
    };

    let output = render("ignored", "source.zd", &diagnostic);

    assert!(output.contains("Error: Could not decode source.zd"));
    assert!(output.contains("--> source.zd"));
    assert!(output.contains("help: Save the file as UTF-8."));
    assert!(!output.contains('│'), "unexpected source gutter:\n{output}");
    assert!(!output.contains('┬'), "unexpected caret:\n{output}");
}

#[test]
fn parser_diagnostics_render_the_path_message_and_source() {
    let src = "state votes is Map of Id to Int starting empty";
    let error = zdc_parser::parse(src).unwrap_err();
    let diagnostic = Diagnostic::from(error);

    let output = render(src, "example.zd", &diagnostic);
    let plain = strip_ansi(&output);

    assert!(plain.contains("example.zd"));
    assert!(
        plain.contains("client"),
        "missing expected syntax:\n{output}"
    );
    assert!(plain.contains("Map"), "missing source text:\n{output}");
}
