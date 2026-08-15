//! The route back from a generated line to the `.zd` line (#6).
//!
//! **Everything here decodes.** A source map is base64 VLQ over relative
//! numbers on four axes, and every way of getting it wrong produces a
//! document that parses: a sign bit read as a continuation, a source index
//! reset per line instead of carried, a column counted in bytes. None of
//! those is visible in the string, and all of them point the reader at the
//! wrong line — which costs more than no map at all, because they only
//! find out after making the trip.
//!
//! So the assertions are made through a decoder written here, against the
//! two texts the map claims to relate. `assert_maps` takes a fragment of
//! emitted JavaScript and the `.zd` line it should resolve to, finds the
//! generated line holding that fragment, and asks the map. If the encoder
//! and the decoder were wrong in the same direction the answer would still
//! have to be a real line of the real source.

mod support;

use zdc_codegen::sourcemap::{self, Content, SourceFile};
use zdc_codegen::Bundle;

/// The alphabet, restated rather than imported: this file is checking the
/// encoder, and sharing its table would let one typo agree with itself.
const DIGITS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// One decoded segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    generated_line: usize,
    generated_column: usize,
    source: usize,
    source_line: usize,
    source_column: usize,
}

fn digits(segment: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let mut chars = segment.chars().peekable();
    while chars.peek().is_some() {
        let mut value = 0i64;
        let mut shift = 0;
        loop {
            let c = chars.next().expect("a digit");
            let digit = DIGITS
                .find(c)
                .unwrap_or_else(|| panic!("`{c}` is not a base64 digit"))
                as i64;
            value |= (digit & 0b1_1111) << shift;
            shift += 5;
            if digit & 0b10_0000 == 0 {
                break;
            }
        }
        out.push(if value & 1 == 1 {
            -(value >> 1)
        } else {
            value >> 1
        });
    }
    out
}

/// Every segment of a `mappings` field, absolute.
///
/// The deltas are what the encoder gets to be wrong about, so the decoder
/// undoes all four of them: the generated column resets at each line and
/// the other three carry across the whole document.
fn decode(mappings: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let (mut source, mut source_line, mut source_column) = (0i64, 0i64, 0i64);
    for (generated_line, line) in mappings.split(';').enumerate() {
        let mut generated_column = 0i64;
        if line.is_empty() {
            continue;
        }
        for segment in line.split(',') {
            let values = digits(segment);
            assert_eq!(
                values.len(),
                4,
                "this map writes four-field segments and only four-field segments; \
                 a one-field segment says `nothing here maps` and a five-field one \
                 names an original identifier, and neither is something the emitter \
                 has any business producing"
            );
            generated_column += values[0];
            source += values[1];
            source_line += values[2];
            source_column += values[3];
            assert!(
                generated_column >= 0 && source >= 0 && source_line >= 0 && source_column >= 0,
                "a delta took a field negative, which means the encoder's baseline \
                 disagrees with the decoder's: {segment}"
            );
            out.push(Segment {
                generated_line,
                generated_column: generated_column as usize,
                source: source as usize,
                source_line: source_line as usize,
                source_column: source_column as usize,
            });
        }
    }
    out
}

/// The map a single-file program produces, and its own two texts.
struct Mapped {
    generated: String,
    source: String,
    document: serde_free::Map,
}

/// The tiny amount of JSON reading these tests need.
///
/// A dependency would be the honest thing if this grew; it has not, and
/// `serde_json` is not in the workspace. The fields read are the four the
/// encoder writes, and each is read by finding its key — so a map that
/// reordered them, or grew a field, still parses here.
mod serde_free {
    #[derive(Debug, Clone)]
    pub struct Map {
        pub text: String,
    }

    impl Map {
        /// The value of `"key":"..."`, unescaped enough for the escapes
        /// this encoder can produce.
        pub fn string(&self, key: &str) -> String {
            let at = self
                .text
                .find(&format!("\"{key}\":\""))
                .unwrap_or_else(|| panic!("no `{key}` in {}", self.text));
            let rest = &self.text[at + key.len() + 4..];
            let mut out = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => return out,
                    '\\' => match chars.next().expect("an escape") {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'u' => {
                            let hex: String = chars.by_ref().take(4).collect();
                            let code = u32::from_str_radix(&hex, 16).expect("four hex digits");
                            out.push(char::from_u32(code).expect("a scalar value"));
                        }
                        other => panic!("unexpected escape `\\{other}`"),
                    },
                    _ => out.push(c),
                }
            }
            panic!("unterminated string for `{key}`")
        }

        pub fn has(&self, key: &str) -> bool {
            self.text.contains(&format!("\"{key}\":"))
        }

        /// The first entry of a `["..."]` array field.
        pub fn first_of(&self, key: &str) -> String {
            let at = self
                .text
                .find(&format!("\"{key}\":["))
                .unwrap_or_else(|| panic!("no `{key}` in {}", self.text));
            let rest = &self.text[at + key.len() + 4..];
            Map {
                text: format!("\"x\":{rest}"),
            }
            .string("x")
        }
    }
}

fn map_of(source: &str, content: Content) -> Mapped {
    let bundle: Bundle = support::compile_source_named(source, "app.zd");
    let sources = vec![SourceFile {
        name: "app.zd".to_string(),
        text: source.to_string(),
        offset: 0,
    }];
    let document = sourcemap::render("client.js", &bundle.mappings, &sources, content);
    Mapped {
        generated: bundle.client_js,
        source: source.to_string(),
        document: serde_free::Map { text: document },
    }
}

impl Mapped {
    fn segments(&self) -> Vec<Segment> {
        decode(&self.document.string("mappings"))
    }

    /// The generated line, zero-based, that contains `fragment`.
    fn generated_line_of(&self, fragment: &str) -> usize {
        let found: Vec<usize> = self
            .generated
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(fragment))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(
            found.len(),
            1,
            "`{fragment}` should name one emitted line and named {}:\n{}",
            found.len(),
            self.generated
        );
        found[0]
    }

    /// What the map says about the generated line holding `fragment` —
    /// which is what a stack trace naming that line would be resolved to.
    ///
    /// The *last* segment at or before the position, which is the lookup a
    /// browser performs: a segment claims every position after it until the
    /// next one.
    fn resolve(&self, fragment: &str) -> Segment {
        let line = self.generated_line_of(fragment);
        *self
            .segments()
            .iter()
            .rfind(|segment| segment.generated_line <= line)
            .unwrap_or_else(|| panic!("nothing maps at or before generated line {}", line + 1))
    }

    /// The `.zd` line the map resolves `fragment` to, as text.
    fn zd_line(&self, fragment: &str) -> String {
        let segment = self.resolve(fragment);
        self.source
            .lines()
            .nth(segment.source_line)
            .unwrap_or_else(|| {
                panic!(
                    "the map points at line {} of a {} line file",
                    segment.source_line + 1,
                    self.source.lines().count()
                )
            })
            .to_string()
    }
}

/// A program with a function whose body is several statements, so there is
/// something for the map to distinguish between — and which calls two
/// prelude functions, so the bundle contains emitted code the map must
/// leave alone.
const COUNTING: &str = "\
state total is client Whole starting 0

function tally of values
    with lowest is (first of values)
    give valueOr with maybe is lowest, fallback is 0

function doubled of n
    give n * 2

view
    Column
        Text total
        Text (tally of [1, 2])
        Text (doubled of 3)
";

/// The claim #6 exists for, in the form the issue puts it: a position in
/// generated JavaScript resolves to the line of `.zd` that produced it.
#[test]
fn a_generated_line_resolves_to_the_zd_line_that_produced_it() {
    let mapped = map_of(COUNTING, Content::Omit);

    assert_eq!(
        mapped.zd_line("function tally("),
        "function tally of values"
    );
    assert_eq!(mapped.zd_line("function doubled("), "function doubled of n");
    assert_eq!(mapped.zd_line("return n * 2;"), "    give n * 2");
    assert_eq!(
        mapped.zd_line("const lowest ="),
        "    with lowest is (first of values)"
    );
}

/// A frame anywhere inside a statement resolves to that statement, not to
/// the one after it.
///
/// This is the property that makes a statement-granular map useful rather
/// than merely present: a browser reports the column the failure happened
/// at, which is somewhere in the middle of an expression the map says
/// nothing about, and the answer has to be the statement it is inside.
#[test]
fn a_column_inside_a_statement_still_resolves_to_that_statement() {
    let mapped = map_of(COUNTING, Content::Omit);
    let line = mapped.generated_line_of("return n * 2;");
    let segment = mapped.resolve("return n * 2;");
    assert_eq!(segment.generated_line, line, "the statement is mapped");

    // Every column of that line resolves to the same segment, because the
    // next segment starts on a later line.
    let after: Vec<Segment> = mapped
        .segments()
        .into_iter()
        .filter(|s| s.generated_line == line && s.generated_column > 0)
        .collect();
    assert!(
        after.is_empty(),
        "this map claims one position per statement, so a second segment on \
         the same line would be a column claim it cannot support: {after:?}"
    );
}

/// The map's own arithmetic, checked against the source it names.
///
/// Not "the numbers decode" — that only says the string is well formed.
/// Every segment is resolved back to a line of `app.zd` and a line of
/// `client.js`, and both have to exist. An encoder whose source index or
/// line delta was reset per line passes the first check and fails this
/// one from the second line onwards.
#[test]
fn every_segment_points_into_both_files() {
    let mapped = map_of(COUNTING, Content::Omit);
    let generated = mapped.generated.lines().count();
    let source = mapped.source.lines().count();
    let segments = mapped.segments();

    assert!(!segments.is_empty(), "the program has statements to map");
    for segment in &segments {
        assert_eq!(segment.source, 0, "one file, so one source index");
        assert!(
            segment.generated_line < generated,
            "segment at generated line {} of a {generated} line file: {segment:?}",
            segment.generated_line + 1
        );
        assert!(
            segment.source_line < source,
            "segment at source line {} of a {source} line file: {segment:?}",
            segment.source_line + 1
        );
        let line = mapped
            .source
            .lines()
            .nth(segment.source_line)
            .expect("a line");
        assert!(
            segment.source_column <= line.chars().count(),
            "column {} of a {} character line: {segment:?}",
            segment.source_column + 1,
            line.chars().count()
        );
    }
}

/// Segments arrive in the order the format requires.
///
/// A decoder is allowed to assume it, and several do: within a line the
/// generated columns increase, and the lines themselves are written in
/// order because `;` is how the encoder counts them.
#[test]
fn segments_are_ordered_the_way_the_format_requires() {
    let mapped = map_of(COUNTING, Content::Omit);
    let segments = mapped.segments();
    // The comparison below is over adjacent pairs, so an empty map would
    // satisfy it without comparing anything. This program has two functions,
    // four statements between them and two declarations.
    assert!(
        segments.len() >= 8,
        "expected at least eight mapped positions, got {}: {segments:?}",
        segments.len()
    );
    for pair in segments.windows(2) {
        let (before, after) = (pair[0], pair[1]);
        assert!(
            (before.generated_line, before.generated_column)
                < (after.generated_line, after.generated_column),
            "{before:?} is not before {after:?}"
        );
    }
}

/// Nothing from §17.4.1's prelude reaches the map.
///
/// The library is resolved into the same arenas as the program and its
/// spans index the library's *own* sources, so a mark from one would land
/// on whatever byte of `app.zd` sat at that offset. `sum` and `valueOr`
/// are prelude functions and this program calls both, so the emitted
/// bundle contains their bodies.
#[test]
fn the_prelude_contributes_no_mappings() {
    let mapped = map_of(COUNTING, Content::Omit);
    let source_lines = mapped.source.lines().count();
    for segment in mapped.segments() {
        assert!(
            segment.source_line < source_lines,
            "a mapping points past the end of `app.zd`, which is what a prelude \
             span looks like once it is read as an offset into the user's file: \
             {segment:?}"
        );
    }
    // And positively: the prelude's own functions are emitted and none of
    // the generated lines holding one is mapped.
    let unmapped: Vec<&str> = mapped
        .generated
        .lines()
        .enumerate()
        .filter(|(index, line)| {
            line.starts_with("function ")
                && !line.contains("tally")
                && !line.contains("doubled")
                && !mapped
                    .segments()
                    .iter()
                    .any(|segment| segment.generated_line == *index)
        })
        .map(|(_, line)| line)
        .collect();
    assert!(
        !unmapped.is_empty(),
        "this program calls prelude functions, so their emitted bodies should be \
         present and unmapped:\n{}",
        mapped.generated
    );
}

/// The bundle names its map, and names the file that is written beside it.
#[test]
fn the_bundle_names_the_map_that_sits_beside_it() {
    let bundle = support::compile_source_named(COUNTING, "app.zd");
    assert!(
        bundle
            .client_js
            .ends_with("//# sourceMappingURL=client.js.map\n"),
        "the comment must be the last line, or a bundler appending to the file \
         moves it into the middle where nothing reads it:\n{}",
        &bundle.client_js[bundle.client_js.len().saturating_sub(200)..]
    );
    assert_eq!(zdc_codegen::CLIENT_MAP, "client.js.map");
}

/// `zdc build` names the sources and does not carry them; `zdc dev` does.
#[test]
fn only_a_development_map_carries_the_program() {
    let released = map_of(COUNTING, Content::Omit);
    assert!(
        !released.document.has("sourcesContent"),
        "a released map sits at a guessable public URL; carrying the program's \
         text there publishes it: {}",
        released.document.text
    );
    assert_eq!(released.document.first_of("sources"), "app.zd");

    let development = map_of(COUNTING, Content::Embed);
    assert!(development.document.has("sourcesContent"));
    assert_eq!(
        development.document.first_of("sourcesContent"),
        COUNTING,
        "the embedded text must be the source byte for byte, or devtools \
         highlights the wrong span of it"
    );
}

/// The version and the shape a decoder checks before it reads anything.
#[test]
fn the_document_declares_the_version_it_is() {
    let mapped = map_of(COUNTING, Content::Omit);
    assert!(mapped.document.text.starts_with("{\"version\":3,"));
    assert_eq!(mapped.document.string("file"), "client.js");
    assert!(
        mapped.document.text.contains("\"names\":[]"),
        "this map renames nothing, so there is nothing for a name index to \
         point at and the array is empty rather than absent"
    );
}

/// A program with no statements outside its view still gets a map.
///
/// An empty `mappings` field is a different fact from a missing file: the
/// browser is told that nothing here maps, rather than fetching a name the
/// bundle gave it and getting a 404 on every load.
#[test]
fn a_program_with_nothing_to_map_still_produces_a_document() {
    let mapped = map_of(
        "state count is client Whole starting 0\n\nview\n    Text count\n",
        Content::Omit,
    );
    assert!(mapped.document.text.starts_with("{\"version\":3,"));
    // The `state` itself is a declaration and is mapped, so this is not the
    // empty case — assert what it does say rather than that it says
    // nothing.
    assert_eq!(
        mapped.zd_line("signal("),
        "state count is client Whole starting 0"
    );
}

/// A statement inside a nested block maps to its own line, not to the
/// block's.
#[test]
fn a_statement_inside_a_nested_block_maps_to_its_own_line() {
    const NESTED: &str = "\
function classify of n
    if n > 10
        give \"big\"
    give \"small\"

view
    Column
        Text (classify of 3)
";
    let mapped = map_of(NESTED, Content::Omit);
    assert_eq!(mapped.zd_line("if (n > 10)"), "    if n > 10");
    assert_eq!(mapped.zd_line("return 'big';"), "        give \"big\"");
    assert_eq!(mapped.zd_line("return 'small';"), "    give \"small\"");
}
