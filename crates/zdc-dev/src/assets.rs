//! The bundle the dev server holds in memory.
//!
//! `zdc dev` never writes a `dist/`. Serving from memory removes the
//! window in which a browser could load a half-written directory — the
//! same reasoning that makes `zdc build` write only after the whole
//! program compiles — and it removes the question of what to do with the
//! directory when the server stops.

use std::collections::BTreeMap;

/// One served file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// Everything the server can serve, keyed by request path with a leading
/// slash (`/client.js`, `/runtime/dom.js`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Assets {
    files: BTreeMap<String, Asset>,
}

impl Assets {
    pub fn insert(&mut self, path: impl Into<String>, body: impl Into<Vec<u8>>) {
        let path = path.into();
        let content_type = content_type(&path);
        self.files.insert(
            path,
            Asset {
                content_type,
                body: body.into(),
            },
        );
    }

    /// Look up a request target.
    ///
    /// The query string is dropped and `/` is the index, so a browser that
    /// asks for `/?v=2` — or a reload that appends a cache-buster — still
    /// finds the page.
    pub fn get(&self, target: &str) -> Option<&Asset> {
        self.files.get(&normalize(target))
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
}

/// Reduce a request target to a bundle key.
pub fn normalize(target: &str) -> String {
    let path = target
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    if path.is_empty() {
        return "/index.html".to_string();
    }
    path.to_string()
}

/// The `Content-Type` for a bundle path.
///
/// A module served as `text/plain` is refused by the browser outright, so
/// this is load-bearing rather than cosmetic.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        // §14C.3b's generated files. A `static` signal names its own
        // output path, so these extensions arrive from the program rather
        // than from the compiler: `rss.xml` served as an octet stream is
        // downloaded instead of rendered, which is not what ships.
        Some("xml") => "application/xml; charset=utf-8",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        // A genuinely open domain — a file extension is any string, and
        // the set a program can emit is unbounded — so this arm is a
        // default, not a fallthrough. Octet stream is the safe answer: the
        // browser saves the file rather than interpreting it as something
        // it is not.
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_path_resolves_to_the_index() {
        let mut assets = Assets::default();
        assets.insert("/index.html", "<!doctype html>");
        assert!(assets.get("/").is_some());
        assert!(assets.get("").is_some());
        assert!(assets.get("/index.html").is_some());
    }

    #[test]
    fn a_query_string_does_not_hide_the_asset() {
        let mut assets = Assets::default();
        assets.insert("/client.js", "export {}");
        assert!(assets.get("/client.js?t=1730000000").is_some());
        assert!(assets.get("/?reload=1").is_none(), "no index inserted");
    }

    #[test]
    fn a_module_is_served_as_javascript_not_plain_text() {
        // A browser refuses `import` from a module served as text/plain,
        // and the failure looks like a blank page rather than an error.
        let mut assets = Assets::default();
        assets.insert("/client.js", "export {}");
        assets.insert("/styles.css", "body{}");
        assets.insert("/manifest.json", "{}");
        assets.insert("/index.html", "<!doctype html>");

        assert_eq!(
            assets.get("/client.js").unwrap().content_type,
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            assets.get("/styles.css").unwrap().content_type,
            "text/css; charset=utf-8"
        );
        assert_eq!(
            assets.get("/manifest.json").unwrap().content_type,
            "application/json; charset=utf-8"
        );
        assert_eq!(
            assets.get("/index.html").unwrap().content_type,
            "text/html; charset=utf-8"
        );
    }

    /// §14C.3b's generated files are part of the site being developed, so
    /// `zdc dev` has to serve them as what they are. `examples/writing.zd`
    /// emits `rss.xml`, and an octet stream is downloaded rather than
    /// rendered — the thing under development would not be the thing that
    /// ships.
    #[test]
    fn an_emitted_file_is_served_as_its_own_type() {
        let mut assets = Assets::default();
        assets.insert("/rss.xml", "<rss/>");
        assets.insert("/robots.txt", "User-agent: *");
        assert_eq!(
            assets.get("/rss.xml").unwrap().content_type,
            "application/xml; charset=utf-8"
        );
        assert_eq!(
            assets.get("/robots.txt").unwrap().content_type,
            "text/plain; charset=utf-8"
        );
    }

    #[test]
    fn an_unknown_path_is_absent_rather_than_guessed_at() {
        let assets = Assets::default();
        assert!(assets.get("/nope.js").is_none());
    }

    #[test]
    fn a_trailing_slash_does_not_create_a_second_key() {
        assert_eq!(normalize("/runtime/"), "/runtime");
        assert_eq!(normalize("/"), "/index.html");
    }
}
