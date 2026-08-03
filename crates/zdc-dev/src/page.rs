//! The two pages `zdc dev` adds to a build: the live-reload client that
//! is injected into every served page, and the error page that replaces
//! the app when the program does not compile.

use crate::ansi;
use crate::sse;

/// The live-reload client, as a `<script>` element.
///
/// It is injected into the app page *and* the error page, because the
/// error page is the one that most needs it: the developer is looking at
/// a diagnostic, and the fix should make it disappear without a manual
/// refresh.
///
/// `ready` carries the generation the server is currently on. A client
/// that reconnects and sees a different generation than the one it loaded
/// under reloads, which is what happens when the server was restarted or
/// the machine was asleep while the source changed.
pub fn live_script() -> String {
    format!(
        "<script>\n\
         (function () {{\n\
         \x20 var seen = null;\n\
         \x20 var source = new EventSource({path:?});\n\
         \x20 source.addEventListener({ready:?}, function (e) {{\n\
         \x20   if (seen !== null && seen !== e.data) location.reload();\n\
         \x20   seen = e.data;\n\
         \x20 }});\n\
         \x20 source.addEventListener({reload:?}, function () {{ location.reload(); }});\n\
         }})();\n\
         </script>\n",
        path = sse::LIVE_PATH,
        ready = sse::READY,
        reload = sse::RELOAD,
    )
}

/// Append the live-reload client to a generated page.
///
/// Appending rather than splicing into `</body>`: the page `zdc-codegen`
/// emits has no `</body>`, and a rewrite that silently found no anchor
/// would produce a page that simply never reloads — a bug with no symptom
/// except the one the developer would blame on the watcher.
pub fn with_live_reload(html: &str) -> String {
    let mut page = html.to_string();
    if !page.ends_with('\n') {
        page.push('\n');
    }
    page.push_str(&live_script());
    page
}

/// The page shown when the program does not compile.
///
/// `report` is the terminal output of `zdc-diagnostics`, unchanged — the
/// browser is shown the same bytes as the terminal (spec §7.3), with the
/// escape sequences translated rather than discarded.
pub fn error_page(source_path: &str, report: &str) -> String {
    format!(
        "<!doctype html>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} — zdc dev</title>\n\
         <style>{css}</style>\n\
         <main>\n\
         \x20 <h1>This program does not compile.</h1>\n\
         \x20 <p class=\"path\">{path}</p>\n\
         \x20 <pre>{body}</pre>\n\
         \x20 <p class=\"hint\">Fix it and save. This page reloads itself.</p>\n\
         </main>\n\
         {live}",
        title = ansi::escape(source_path),
        path = ansi::escape(source_path),
        body = ansi::to_html(report),
        css = ERROR_CSS,
        live = live_script(),
    )
}

/// Deliberately terminal-like. The developer is reading compiler output;
/// dressing it up as a web page would make the two renderings look like
/// two different diagnostics.
const ERROR_CSS: &str = "\
:root { color-scheme: dark; }
body { margin: 0; background: #21252b; color: #abb2bf;
       font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
main { max-width: 60rem; margin: 0 auto; padding: 2.5rem 1.5rem; }
h1 { margin: 0 0 .25rem; font-size: 1.1rem; font-weight: 700; color: #e06c75; }
.path { margin: 0 0 1.5rem; color: #5c6370; }
pre { margin: 0; padding: 1.25rem; overflow-x: auto;
      background: #1b1f24; border: 1px solid #333842; border-radius: 6px;
      white-space: pre; tab-size: 4; }
.hint { margin: 1.5rem 0 0; color: #5c6370; }
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_script_subscribes_to_the_stream_the_server_publishes() {
        let script = live_script();
        assert!(
            script.contains(sse::LIVE_PATH),
            "the client and the server must agree on the path:\n{script}"
        );
        assert!(script.contains("EventSource"), "no subscription:\n{script}");
        assert!(script.contains("location.reload"), "no reload:\n{script}");
    }

    #[test]
    fn the_live_script_listens_for_both_event_names_the_server_sends() {
        let script = live_script();
        assert!(
            script.contains("\"reload\""),
            "no reload listener:\n{script}"
        );
        assert!(script.contains("\"ready\""), "no ready listener:\n{script}");
    }

    #[test]
    fn injection_appends_the_client_without_disturbing_the_page() {
        let generated = "<!doctype html>\n<div id=\"app\"></div>\n";
        let page = with_live_reload(generated);
        assert!(page.starts_with(generated), "page was rewritten:\n{page}");
        assert!(page.contains("EventSource"), "client not injected:\n{page}");
    }

    #[test]
    fn injection_does_not_run_two_lines_together() {
        let page = with_live_reload("<div id=\"app\"></div>");
        assert!(
            page.contains("</div>\n<script>"),
            "the injected script must start on its own line:\n{page}"
        );
    }

    #[test]
    fn the_error_page_shows_the_diagnostic_and_the_path() {
        let report = "\u{1b}[31mError\u{1b}[0m: Expected a line break.";
        let page = error_page("examples/counter.zd", report);
        assert!(page.contains("examples/counter.zd"), "no path:\n{page}");
        assert!(
            page.contains("Expected a line break."),
            "no message:\n{page}"
        );
    }

    #[test]
    fn the_error_page_keeps_the_colours_the_terminal_shows() {
        let page = error_page("a.zd", "\u{1b}[31mError\u{1b}[0m: bad");
        assert!(
            page.contains("<span style=\"color:"),
            "colour lost:\n{page}"
        );
        assert!(!page.contains('\u{1b}'), "raw escapes leaked:\n{page}");
    }

    #[test]
    fn the_error_page_reloads_itself_when_the_program_is_fixed() {
        // Without this the developer fixes the error, saves, and stares at
        // a stale error page wondering why the fix did not take.
        let page = error_page("a.zd", "Error: bad");
        assert!(page.contains("EventSource"), "no live reload:\n{page}");
    }

    #[test]
    fn a_diagnostic_quoting_markup_is_shown_not_executed() {
        // The report quotes the developer's source. A `.zd` file that
        // contains a tag must not inject it into the error page.
        let page = error_page("a.zd", "Error: near <img src=x onerror=alert(1)>");
        assert!(!page.contains("<img"), "markup injected:\n{page}");
        assert!(page.contains("&lt;img"), "markup not escaped:\n{page}");
    }

    #[test]
    fn a_path_containing_markup_is_escaped_in_the_title_and_the_body() {
        let page = error_page("</title><script>alert(1)</script>", "Error");
        assert!(!page.contains("<script>alert(1)"), "path injected:\n{page}");
    }
}
