//! Turns the terminal output of `zdc-diagnostics` into HTML.
//!
//! Spec §7.3 makes diagnostics a primary deliverable, and a developer
//! looking at the page must see the *same* diagnostic the terminal shows —
//! not a second, plainer rendering that can drift from it. So the browser
//! is given `ariadne`'s real output, escapes and all, and this module
//! translates the SGR sequences into spans rather than stripping them.
//!
//! Only the sequences `ariadne` actually emits are interpreted: basic
//! foreground colours, 256-colour foregrounds, and reset. Anything else is
//! consumed and ignored, so an unrecognised escape degrades to unstyled
//! text instead of leaking `\u{1b}[…m` into the page.

use std::fmt::Write as _;

/// Convert a string containing SGR escape sequences into HTML.
///
/// The result is a fragment, not a document: it is meant to sit inside a
/// `<pre>`. Runs of identically styled characters are coalesced, because
/// `ariadne` colours the highlighted source line one character at a time
/// and a span per character would be roughly twenty times the bytes for
/// exactly the same pixels.
pub fn to_html(ansi: &str) -> String {
    let mut out = String::new();
    let mut style = Style::default();
    let mut open: Option<Style> = None;
    let mut run = String::new();
    let mut chars = ansi.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            run.push(c);
            continue;
        }
        // A CSI sequence: ESC '[' parameters final-byte. Anything that is
        // not one is dropped along with its escape, never printed.
        if chars.peek() != Some(&'[') {
            continue;
        }
        chars.next();
        let mut params = String::new();
        let mut final_byte = None;
        for c in chars.by_ref() {
            if c.is_ascii_digit() || c == ';' {
                params.push(c);
            } else {
                final_byte = Some(c);
                break;
            }
        }
        if final_byte != Some('m') {
            continue;
        }
        let next = style.applying(&params);
        if next != style {
            flush(&mut out, &mut run, &mut open, style);
            style = next;
        }
    }
    flush(&mut out, &mut run, &mut open, style);
    if open.is_some() {
        out.push_str("</span>");
    }
    out
}

/// Emit the pending run under the style it was collected with, opening or
/// closing a span only when the style actually differs from the open one.
fn flush(out: &mut String, run: &mut String, open: &mut Option<Style>, style: Style) {
    if run.is_empty() {
        return;
    }
    if *open != Some(style) {
        if open.is_some() {
            out.push_str("</span>");
        }
        match style.css() {
            Some(css) => {
                let _ = write!(out, "<span style=\"{css}\">");
                *open = Some(style);
            }
            None => *open = None,
        }
    }
    out.push_str(&escape(run));
    run.clear();
}

/// The subset of SGR state this renderer models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Style {
    color: Option<Rgb>,
    bold: bool,
}

type Rgb = (u8, u8, u8);

impl Style {
    /// Apply one SGR parameter list. An empty list means `ESC[m`, which is
    /// a reset.
    fn applying(self, params: &str) -> Style {
        if params.is_empty() {
            return Style::default();
        }
        let codes: Vec<&str> = params.split(';').collect();
        let mut style = self;
        let mut i = 0;
        while i < codes.len() {
            let code: u16 = codes[i].parse().unwrap_or(0);
            match code {
                0 => style = Style::default(),
                1 => style.bold = true,
                22 => style.bold = false,
                30..=37 => style.color = Some(basic(code - 30)),
                39 => style.color = None,
                90..=97 => style.color = Some(basic(code - 90 + 8)),
                // `38;5;n` is the 256-colour form `ariadne` uses for its
                // gutters; `38;2;r;g;b` is truecolour. Both consume the
                // parameters they name, so the loop must skip them or it
                // would read `5` as "blink" and `246` as nothing.
                38 => {
                    match codes.get(i + 1).copied() {
                        Some("5") => {
                            style.color = codes.get(i + 2).and_then(|n| n.parse().ok()).map(xterm);
                            i += 2;
                        }
                        Some("2") => {
                            let component = |k: usize| -> u8 {
                                codes.get(i + k).and_then(|n| n.parse().ok()).unwrap_or(0)
                            };
                            style.color = Some((component(2), component(3), component(4)));
                            i += 4;
                        }
                        _ => {}
                    };
                }
                _ => {}
            }
            i += 1;
        }
        style
    }

    fn css(self) -> Option<String> {
        let mut css = String::new();
        if let Some((r, g, b)) = self.color {
            let _ = write!(css, "color:#{r:02x}{g:02x}{b:02x}");
        }
        if self.bold {
            if !css.is_empty() {
                css.push(';');
            }
            css.push_str("font-weight:700");
        }
        (!css.is_empty()).then_some(css)
    }
}

/// The sixteen basic colours, in a palette chosen to stay legible on the
/// dark page the diagnostics are shown on.
fn basic(index: u16) -> Rgb {
    const PALETTE: [Rgb; 16] = [
        (0x28, 0x2c, 0x34), // black
        (0xe0, 0x6c, 0x75), // red
        (0x98, 0xc3, 0x79), // green
        (0xe5, 0xc0, 0x7b), // yellow
        (0x61, 0xaf, 0xef), // blue
        (0xc6, 0x78, 0xdd), // magenta
        (0x56, 0xb6, 0xc2), // cyan
        (0xab, 0xb2, 0xbf), // white
        (0x5c, 0x63, 0x70), // bright black
        (0xef, 0x8b, 0x93), // bright red
        (0xb2, 0xd6, 0x99), // bright green
        (0xf0, 0xd3, 0x99), // bright yellow
        (0x84, 0xc2, 0xf5), // bright blue
        (0xd7, 0x9a, 0xe8), // bright magenta
        (0x7d, 0xcb, 0xd4), // bright cyan
        (0xe6, 0xe9, 0xef), // bright white
    ];
    PALETTE[(index as usize).min(15)]
}

/// The xterm 256-colour cube, which `ariadne` draws its gutter greys from.
fn xterm(index: u8) -> Rgb {
    match index {
        0..=15 => basic(index as u16),
        16..=231 => {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let n = index as usize - 16;
            (LEVELS[n / 36], LEVELS[(n / 6) % 6], LEVELS[n % 6])
        }
        _ => {
            let v = 8 + (index as u16 - 232) * 10;
            let v = v as u8;
            (v, v, v)
        }
    }
}

/// Escape the five characters that would otherwise be read as markup.
///
/// This runs on compiler output, which quotes the developer's own source —
/// a `.zd` file containing `<script>` must appear on the error page as
/// text, not execute in it.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_through_unchanged() {
        assert_eq!(to_html("Expected a line break."), "Expected a line break.");
    }

    #[test]
    fn no_escape_sequence_survives_into_the_output() {
        // The failure this guards against is visible garbage on the page:
        // a literal `[31m` in front of every message.
        let html = to_html("\u{1b}[31mError\u{1b}[0m: bad\u{1b}[38;5;246m│\u{1b}[0m");
        assert!(!html.contains('\u{1b}'), "raw escape survived: {html:?}");
        assert!(!html.contains("[31m"), "escape body survived: {html:?}");
        assert!(
            !html.contains("[38;5;246m"),
            "escape body survived: {html:?}"
        );
    }

    #[test]
    fn a_basic_colour_becomes_a_span() {
        let html = to_html("\u{1b}[31mError\u{1b}[0m: bad");
        assert_eq!(html, "<span style=\"color:#e06c75\">Error</span>: bad");
    }

    #[test]
    fn a_256_colour_gutter_becomes_a_span_of_that_grey() {
        // `ariadne` draws every gutter with `38;5;n`. Read as three
        // separate codes it would produce the wrong colour and swallow no
        // parameters, so this is the case most likely to be mis-parsed.
        let html = to_html("\u{1b}[38;5;246m│\u{1b}[0m");
        assert_eq!(html, "<span style=\"color:#949494\">│</span>");
    }

    #[test]
    fn a_truecolour_sequence_becomes_that_exact_colour() {
        let html = to_html("\u{1b}[38;2;18;52;86mx\u{1b}[0m");
        assert_eq!(html, "<span style=\"color:#123456\">x</span>");
    }

    #[test]
    fn bold_is_carried_alongside_colour() {
        let html = to_html("\u{1b}[1;31mError\u{1b}[0m");
        assert_eq!(
            html,
            "<span style=\"color:#e06c75;font-weight:700\">Error</span>"
        );
    }

    #[test]
    fn adjacent_characters_of_one_colour_share_a_single_span() {
        // `ariadne` colours the source line character by character, so a
        // span per character is the natural — and very fat — output.
        let per_char = "\u{1b}[31mv\u{1b}[0m\u{1b}[31mi\u{1b}[0m\u{1b}[31me\u{1b}[0m";
        let html = to_html(per_char);
        assert_eq!(html, "<span style=\"color:#e06c75\">vie</span>");
        assert_eq!(html.matches("<span").count(), 1, "spans not coalesced");
    }

    #[test]
    fn every_opened_span_is_closed() {
        let html = to_html("\u{1b}[31mError: unterminated colour");
        assert_eq!(
            html.matches("<span").count(),
            html.matches("</span>").count()
        );
    }

    #[test]
    fn markup_in_the_quoted_source_is_escaped_not_executed() {
        // The diagnostic quotes the developer's file. If that file
        // contains a tag, the error page must show it, not run it.
        let html = to_html("\u{1b}[31m<script>alert(1)</script>\u{1b}[0m");
        assert!(!html.contains("<script>"), "unescaped markup: {html}");
        assert!(html.contains("&lt;script&gt;"), "not escaped: {html}");
    }

    #[test]
    fn an_unknown_escape_degrades_to_unstyled_text() {
        // A sequence this renderer does not model must not print itself.
        let html = to_html("a\u{1b}[2Kb");
        assert_eq!(html, "ab");
    }

    #[test]
    fn a_lone_escape_at_the_end_of_input_does_not_panic_or_print() {
        assert_eq!(to_html("done\u{1b}"), "done");
        assert_eq!(to_html("done\u{1b}["), "done");
        assert_eq!(to_html("done\u{1b}[38;5;"), "done");
    }

    #[test]
    fn escape_covers_the_characters_that_would_break_out_of_text() {
        assert_eq!(escape("<&>\"'"), "&lt;&amp;&gt;&quot;&#39;");
    }
}
