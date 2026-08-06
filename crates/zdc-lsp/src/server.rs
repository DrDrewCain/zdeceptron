//! The protocol loop: JSON-RPC over stdin and stdout.
//!
//! Every request is answered from a fresh [`Analysis`] of the document's
//! current text, held in a map keyed by URI. Documents are synchronised in
//! full rather than incrementally — see [`crate::analysis`] for why that is
//! a deliberate choice rather than an unfinished one.
//!
//! The loop must outlive every bad message it is sent. A request whose
//! parameters do not deserialize is answered with an error, an unknown
//! method is answered with an error, and a notification that makes no sense
//! is ignored; none of the three ends the loop, because an editor that
//! sends one has not stopped needing diagnostics for everything else.

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, ExtractError, Message, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionResponse, Diagnostic,
    DiagnosticSeverity, GotoDefinitionResponse, Hover, HoverContents, HoverProviderCapability,
    Location, MarkupContent, MarkupKind, OneOf, Position, PublishDiagnosticsParams, Range,
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};

use crate::analysis::Analysis;
use crate::complete::CompletionKind;
use crate::lines::LineIndex;
use crate::tokens::{TOKEN_MODIFIERS, TOKEN_TYPES};

/// Serve one editor over stdin and stdout until it disconnects.
pub fn run() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, threads) = Connection::stdio();
    let capabilities = serde_json::to_value(capabilities())?;
    let _ = connection.initialize(capabilities)?;
    serve(&connection)?;

    // The writer thread runs until the last sender is dropped, and the
    // connection holds one. Joining before dropping it waits for a thread
    // that is waiting for this one, so the process would linger after the
    // editor asked it to exit — invisibly, since it has already stopped
    // answering. `crates/zdc-cli/tests/lsp.rs` is what catches that.
    drop(connection);
    threads.join()?;
    Ok(())
}

fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        // Full sync: the pipeline re-runs on the whole file anyway, so
        // reassembling it from edits would be work with no result.
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: TOKEN_TYPES
                        .iter()
                        .map(|name| SemanticTokenType::new(name))
                        .collect(),
                    token_modifiers: TOKEN_MODIFIERS
                        .iter()
                        .map(|name| SemanticTokenModifier::new(name))
                        .collect(),
                },
                full: Some(SemanticTokensFullOptions::Bool(true)),
                ..Default::default()
            },
        )),
        completion_provider: Some(CompletionOptions {
            // A completion is asked for after a space as often as after a
            // letter, because the grammar's keywords are separate words.
            trigger_characters: Some(vec![" ".to_string(), ".".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The open documents, and the analysis of each.
#[derive(Default)]
struct Documents {
    open: HashMap<Uri, Analysis>,
}

fn serve(connection: &Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
    let mut documents = Documents::default();

    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                let response = answer(&documents, request);
                connection.sender.send(Message::Response(response))?;
            }
            Message::Notification(notification) => {
                if let Some(published) = accept(&mut documents, notification) {
                    connection.sender.send(Message::Notification(
                        lsp_server::Notification::new(
                            lsp_types::notification::PublishDiagnostics::METHOD.to_string(),
                            published,
                        ),
                    ))?;
                }
            }
            // A response to a request this server never sent. Ignoring it
            // is the whole of the correct handling.
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// Take a document change, returning the diagnostics to publish for it.
fn accept(
    documents: &mut Documents,
    notification: lsp_server::Notification,
) -> Option<PublishDiagnosticsParams> {
    use lsp_types::notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument,
    };

    match notification.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: lsp_types::DidOpenTextDocumentParams =
                serde_json::from_value(notification.params).ok()?;
            let uri = params.text_document.uri;
            let analysis =
                Analysis::of_document(file_path(&uri).as_deref(), &params.text_document.text);
            let published = publish(&uri, &analysis);
            documents.open.insert(uri, analysis);
            Some(published)
        }
        DidChangeTextDocument::METHOD => {
            let params: lsp_types::DidChangeTextDocumentParams =
                serde_json::from_value(notification.params).ok()?;
            // Full sync, so the last change carries the whole document.
            let text = params.content_changes.into_iter().next_back()?.text;
            let uri = params.text_document.uri;
            let analysis = Analysis::of_document(file_path(&uri).as_deref(), &text);
            let published = publish(&uri, &analysis);
            documents.open.insert(uri, analysis);
            Some(published)
        }
        DidCloseTextDocument::METHOD => {
            let params: lsp_types::DidCloseTextDocumentParams =
                serde_json::from_value(notification.params).ok()?;
            documents.open.remove(&params.text_document.uri);
            // An empty list clears whatever was showing for the file.
            Some(PublishDiagnosticsParams {
                uri: params.text_document.uri,
                diagnostics: Vec::new(),
                version: None,
            })
        }
        _ => None,
    }
}

fn answer(documents: &Documents, request: Request) -> Response {
    use lsp_types::request::{Completion, GotoDefinition, HoverRequest, SemanticTokensFullRequest};

    let id = request.id.clone();
    match request.method.as_str() {
        HoverRequest::METHOD => reply(id, request, |request: lsp_types::HoverParams| {
            let (analysis, offset) = locate(
                documents,
                &request.text_document_position_params.text_document.uri,
                request.text_document_position_params.position,
            )?;
            let (span, markdown) = crate::hover::hover(analysis, offset)?;
            let (start, end) = analysis.lines().range(analysis.text(), span);
            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: markdown,
                }),
                range: Some(Range {
                    start: point(start),
                    end: point(end),
                }),
            })
        }),

        GotoDefinition::METHOD => reply(id, request, |request: lsp_types::GotoDefinitionParams| {
            let uri = request.text_document_position_params.text_document.uri;
            let (analysis, offset) = locate(
                documents,
                &uri,
                request.text_document_position_params.position,
            )?;
            let span = crate::goto::definition(analysis, offset)?;
            Some(GotoDefinitionResponse::Scalar(location(
                analysis, &uri, span,
            )?))
        }),

        SemanticTokensFullRequest::METHOD => {
            reply(id, request, |request: lsp_types::SemanticTokensParams| {
                let analysis = documents.open.get(&request.text_document.uri)?;
                let data = crate::tokens::encode(&crate::tokens::highlights(analysis))
                    .chunks_exact(5)
                    .map(|five| SemanticToken {
                        delta_line: five[0],
                        delta_start: five[1],
                        length: five[2],
                        token_type: five[3],
                        token_modifiers_bitset: five[4],
                    })
                    .collect();
                Some(SemanticTokensResult::Tokens(SemanticTokens {
                    result_id: None,
                    data,
                }))
            })
        }

        Completion::METHOD => reply(id, request, |request: lsp_types::CompletionParams| {
            let (analysis, offset) = locate(
                documents,
                &request.text_document_position.text_document.uri,
                request.text_document_position.position,
            )?;
            let items = crate::complete::complete(analysis, offset)
                .into_iter()
                .map(|item| CompletionItem {
                    kind: Some(match item.kind {
                        CompletionKind::Keyword => CompletionItemKind::KEYWORD,
                        CompletionKind::Placement => CompletionItemKind::ENUM_MEMBER,
                        CompletionKind::Type => CompletionItemKind::CLASS,
                        CompletionKind::Element => CompletionItemKind::STRUCT,
                        CompletionKind::Variant => CompletionItemKind::ENUM_MEMBER,
                        CompletionKind::Signal => CompletionItemKind::VARIABLE,
                        CompletionKind::Function => CompletionItemKind::FUNCTION,
                    }),
                    detail: Some(item.detail),
                    label: item.label,
                    ..Default::default()
                })
                .collect();
            Some(CompletionResponse::Array(items))
        }),

        other => Response::new_err(
            request.id,
            lsp_server::ErrorCode::MethodNotFound as i32,
            format!("This server does not answer {other}."),
        ),
    }
}

/// Run a handler over a request's parameters, turning any failure into a
/// response rather than into an end of the loop.
///
/// A handler that finds nothing answers `null`, which is what the protocol
/// asks for and what every one of these requests may legitimately return.
fn reply<P, R>(id: RequestId, request: Request, handler: impl FnOnce(P) -> Option<R>) -> Response
where
    P: serde::de::DeserializeOwned,
    R: serde::Serialize,
{
    let method = request.method.clone();
    let params = match request.extract::<P>(&method) {
        Ok((_, params)) => params,
        Err(ExtractError::JsonError { method, error }) => {
            return Response::new_err(
                id,
                lsp_server::ErrorCode::InvalidParams as i32,
                format!("The parameters of {method} could not be read: {error}"),
            )
        }
        Err(ExtractError::MethodMismatch(_)) => {
            return Response::new_err(
                id,
                lsp_server::ErrorCode::InternalError as i32,
                "A request was dispatched to the wrong handler.".to_string(),
            )
        }
    };

    match serde_json::to_value(handler(params)) {
        Ok(value) => Response {
            id,
            response_result: Ok(value),
        },
        Err(error) => Response::new_err(
            id,
            lsp_server::ErrorCode::InternalError as i32,
            format!("The answer could not be encoded: {error}"),
        ),
    }
}

/// A span the compiler produced, as a place an editor can open.
///
/// A span indexes the linker's combined buffer, so a name declared in an
/// imported module belongs to *that* file and to an offset within it.
/// Rendering it against the document the request named would point at the
/// right offset of the wrong file, which is worse than answering nothing:
/// the editor would silently move the cursor somewhere plausible. Every
/// answer carrying a location is built here rather than at its call site,
/// so no one feature can be fixed while another stays wrong.
///
/// A span inside the open document answers with `here`, the URI the
/// editor itself sent, rather than one rebuilt from the path. The two
/// should agree, but only one of them is certain to: an editor is free to
/// escape a path more or less than this server would, and a URI that
/// differs by one `%20` is a second file as far as a client is concerned.
fn location(analysis: &Analysis, here: &Uri, span: zdc_lexer::Span) -> Option<Location> {
    if analysis.in_document(span) {
        let (start, end) = analysis.lines().range(analysis.text(), span);
        return Some(Location {
            uri: here.clone(),
            range: Range {
                start: point(start),
                end: point(end),
            },
        });
    }

    // An imported file, which the editor may never have named, so its URI
    // has to be built. Its text is not this document's, so neither is the
    // line index the span is rendered against.
    let found = analysis.locate(span);
    let lines = LineIndex::new(found.text);
    let (start, end) = lines.range(found.text, found.span);
    Some(Location {
        uri: path_uri(found.path?)?,
        range: Range {
            start: point(start),
            end: point(end),
        },
    })
}

/// A filesystem path as a `file://` URI, the inverse of [`file_path`].
///
/// Only a path that is absolute and valid UTF-8 can become one. A module
/// is always reached from the entry file's own path, so a relative path
/// here would mean the entry document had none, in which case there was
/// nothing to link and no imported file to name.
fn path_uri(path: &std::path::Path) -> Option<Uri> {
    use std::str::FromStr as _;

    let path = normalized(path);
    let text = path.to_str()?;
    if !text.starts_with('/') {
        return None;
    }
    Uri::from_str(&format!("file://{}", percent_encoded(text))).ok()
}

/// A path with `.` components dropped and `..` folded into whatever
/// preceded it.
///
/// A module's path is the importing file's directory joined with the
/// specifier as written, so `use "./model"` yields `…/./model.zd`. An
/// editor compares URIs as strings, and two spellings of one path are two
/// files to it: it would open a second, identical tab rather than move
/// the cursor in the one already showing.
///
/// Folded lexically rather than by `Path::canonicalize`, which reads the
/// filesystem and resolves symlinks. That would answer with a path the
/// editor has never seen, which is the same failure in a new place.
fn normalized(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;

    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Only foldable against a named directory. Above the root,
                // or after a `..` that could not be folded, it has to stay
                // as written or the path stops naming the same file.
                if out
                    .components()
                    .next_back()
                    .is_some_and(|last| matches!(last, Component::Normal(_)))
                {
                    out.pop();
                } else {
                    out.push(component.as_os_str());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                out.push(component.as_os_str())
            }
        }
    }
    out
}

/// Percent-encode the bytes a URI path may not carry literally.
///
/// RFC 3986's `pchar` set, plus `/`, which is the path separator and must
/// stay one. Everything else, including every byte of a multi-byte
/// character, is escaped, which is what makes this the inverse of
/// [`percent_decoded`]. The set is the permissive one on purpose: an
/// editor sends the same paths back and compares URIs textually, so
/// escaping a byte it leaves alone splits one file into two.
fn percent_encoded(text: &str) -> String {
    const UNESCAPED: &[u8] = b"-._~!$&'()*+,;=:@/";

    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || UNESCAPED.contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The analysis of an open document, and the byte offset of a position in
/// it. `None` when the editor asks about a document it never opened.
fn locate<'a>(
    documents: &'a Documents,
    uri: &Uri,
    position: Position,
) -> Option<(&'a Analysis, u32)> {
    let analysis = documents.open.get(uri)?;
    let offset = analysis.lines().offset(
        analysis.text(),
        crate::lines::Position {
            line: position.line,
            character: position.character,
        },
    );
    Some((analysis, offset))
}

/// Every diagnostic for a document, in the protocol's shape.
///
/// The compiler's `help` is folded into the message rather than attached
/// as related information: §7.3 makes the help the part that names the
/// single valid phrasing, so hiding it behind a second click would hide
/// the answer.
/// The filesystem path a document URI names, if it names one.
///
/// A `use` line is relative to the importing file (§14D.2), so a document
/// with no path on disk — an untitled buffer, or one served over a scheme
/// this compiler cannot read — is analysed on its own rather than guessed
/// at.
fn file_path(uri: &Uri) -> Option<std::path::PathBuf> {
    let text = uri.as_str();
    let rest = text.strip_prefix("file://")?;
    // `file:///a/b` on Unix leaves a leading slash, which is the path.
    // A URI with an authority is not this machine's filesystem.
    if !rest.starts_with('/') {
        return None;
    }
    Some(std::path::PathBuf::from(percent_decoded(rest)))
}

/// Undo the percent-encoding an editor applies to a path.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn publish(uri: &Uri, analysis: &Analysis) -> PublishDiagnosticsParams {
    let text = analysis.text();
    let lines: &LineIndex = analysis.lines();

    let diagnostics = analysis
        .diagnostics()
        .iter()
        .map(|diagnostic| {
            let span = diagnostic.span.unwrap_or(zdc_lexer::Span::new(0, 0));
            let (start, end) = lines.range(text, span);
            let message = match &diagnostic.help {
                Some(help) => format!("{}\n\nhelp: {help}", diagnostic.message),
                None => diagnostic.message.clone(),
            };
            Diagnostic {
                range: Range {
                    start: point(start),
                    end: point(end),
                },
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("zdc".to_string()),
                message,
                ..Default::default()
            }
        })
        .collect();

    PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics,
        version: None,
    }
}

fn point(position: crate::lines::Position) -> Position {
    Position {
        line: position.line,
        character: position.character,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr as _;

    fn uri() -> Uri {
        Uri::from_str("file:///example.zd").expect("a valid uri")
    }

    /// A throwaway directory of `.zd` files, so a request can be driven
    /// over a program that really does `use` a second file.
    ///
    /// Every editor feature here has to be exercised across a module
    /// boundary rather than only within one file, because that is the
    /// case where a span stops meaning what it appears to mean: it
    /// indexes the linker's combined buffer and not the document on
    /// screen. A single-file test cannot tell the two apart.
    struct Project {
        root: std::path::PathBuf,
    }

    impl Project {
        fn new(name: &str) -> Project {
            // Tests in one binary run in parallel, so the directory has to
            // be unique per test as well as per process.
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir()
                .join(format!("zdc-lsp-{name}-{}-{serial}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("a temporary directory");
            Project { root }
        }

        fn write(&self, name: &str, source: &str) -> std::path::PathBuf {
            let path = self.root.join(name);
            std::fs::write(&path, source).expect("writing a test module");
            path
        }

        fn uri(&self, name: &str) -> Uri {
            let path = self.root.join(name);
            Uri::from_str(&format!("file://{}", path.display())).expect("a valid file uri")
        }

        /// Open a document exactly as an editor would, so the analysis
        /// under test is the one the notification handler builds.
        fn open(&self, documents: &mut Documents, name: &str, source: &str) -> Uri {
            self.write(name, source);
            let uri = self.uri(name);
            let opened = lsp_server::Notification::new(
                lsp_types::notification::DidOpenTextDocument::METHOD.to_string(),
                lsp_types::DidOpenTextDocumentParams {
                    text_document: lsp_types::TextDocumentItem {
                        uri: uri.clone(),
                        language_id: "zdeceptron".to_string(),
                        version: 1,
                        text: source.to_string(),
                    },
                },
            );
            accept(documents, opened).expect("the open notification is understood");
            uri
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// The two-file program most of the tests below are driven over:
    /// `model.zd` declares a function, `app.zd` imports and calls it.
    const MODEL: &str = "function double with n\n    give n + n\n";
    const APP: &str = "use \"./model\" for double\n\
                       state total is client Whole from double with 2\n\
                       view\n    Text total\n";

    fn request<P: serde::Serialize>(method: &str, params: P) -> Request {
        Request::new(
            1.into(),
            method.to_string(),
            serde_json::to_value(params).expect("serializable parameters"),
        )
    }

    /// The position of the first byte of `needle` in `text`.
    fn position(text: &str, needle: &str) -> Position {
        let at = text.find(needle).expect("the needle is in the source") as u32;
        let lines = LineIndex::new(text);
        let position = lines.position(text, at);
        Position {
            line: position.line,
            character: position.character,
        }
    }

    fn document_position(
        uri: &Uri,
        text: &str,
        needle: &str,
    ) -> lsp_types::TextDocumentPositionParams {
        lsp_types::TextDocumentPositionParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
            position: position(text, needle),
        }
    }

    /// A `Location` an imported file owns: the `uri` names that file and
    /// the range is an offset within it, not within the entry document.
    #[test]
    fn go_to_definition_across_a_use_lands_in_the_imported_file() {
        let project = Project::new("goto-across-use");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let response = answer(
            &documents,
            request(
                lsp_types::request::GotoDefinition::METHOD,
                lsp_types::GotoDefinitionParams {
                    text_document_position_params: document_position(&app, APP, "double with 2"),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );

        let result: GotoDefinitionResponse =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a definition response");
        let GotoDefinitionResponse::Scalar(location) = result else {
            panic!("expected a single location");
        };
        assert_eq!(
            location.uri,
            project.uri("model.zd"),
            "the definition belongs to the imported file"
        );
        // `double` is declared on the first line of `model.zd`, at the
        // character after `function `. Against `app.zd` the same combined
        // offset would land on line 1, which is what this pins down.
        assert_eq!(location.range.start, position(MODEL, "double"));
        assert_eq!(
            location.range.end,
            Position {
                line: 0,
                character: position(MODEL, "double").character + 6,
            }
        );
    }

    /// The single-file case must keep answering against the document the
    /// request named, which is the only file there is.
    #[test]
    fn go_to_definition_within_one_file_still_answers_that_file() {
        let mut documents = Documents::default();
        let src = "state count is client Whole starting 0\nview\n    Text count\n";
        documents.open.insert(uri(), Analysis::of(src));

        let response = answer(
            &documents,
            request(
                lsp_types::request::GotoDefinition::METHOD,
                lsp_types::GotoDefinitionParams {
                    text_document_position_params: document_position(&uri(), src, "count\n"),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let result: GotoDefinitionResponse =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a definition response");
        let GotoDefinitionResponse::Scalar(location) = result else {
            panic!("expected a single location");
        };
        assert_eq!(location.uri, uri());
        assert_eq!(location.range.start, position(src, "count"));
    }

    #[test]
    fn the_legend_indices_match_the_declared_order() {
        let ServerCapabilities {
            semantic_tokens_provider:
                Some(SemanticTokensServerCapabilities::SemanticTokensOptions(options)),
            ..
        } = capabilities()
        else {
            panic!("the server must advertise semantic tokens");
        };
        assert_eq!(options.legend.token_types.len(), TOKEN_TYPES.len());
        assert_eq!(options.legend.token_modifiers.len(), TOKEN_MODIFIERS.len());
    }

    #[test]
    fn a_diagnostic_carries_its_range_and_its_help() {
        let analysis = Analysis::of("state a is client Whole starting \"text\"\n");
        let published = publish(&uri(), &analysis);
        assert_eq!(published.diagnostics.len(), 1);
        let diagnostic = &published.diagnostics[0];
        assert_eq!(diagnostic.source.as_deref(), Some("zdc"));
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
        assert!(diagnostic.range.end.character > diagnostic.range.start.character);
    }

    /// A file-level diagnostic has no span. It must still be publishable,
    /// pointing at the start of the file rather than at nothing.
    ///
    /// This ran `Analysis::of("")` and looped over the diagnostics it
    /// produced. An empty file is *valid*, so there were none: the loop
    /// body never executed and the test passed however `publish` treated a
    /// spanless diagnostic — which is the one code path that runs on the
    /// worst day, when the compiler has panicked. The analysis is built in
    /// the spanless state directly now, and the count is asserted first, so
    /// an empty one fails here instead of passing quietly.
    #[test]
    fn a_spanless_diagnostic_still_becomes_a_range() {
        let analysis = Analysis::spanless("state a is client Whole starting 1\n", "boom");
        let published = publish(&uri(), &analysis);

        assert_eq!(
            published.diagnostics.len(),
            1,
            "the fixture must carry the spanless diagnostic under test"
        );
        let diagnostic = &published.diagnostics[0];
        assert_eq!(diagnostic.message, "boom");
        assert_eq!(
            (
                diagnostic.range.start.line,
                diagnostic.range.start.character
            ),
            (0, 0),
            "a spanless diagnostic points at the start of the file"
        );
        assert_eq!(
            (diagnostic.range.end.line, diagnostic.range.end.character),
            (0, 0),
            "and it claims no width, so no editor underlines a stray character"
        );
        assert_eq!(diagnostic.source.as_deref(), Some("zdc"));
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn opening_and_changing_a_document_publishes_for_it() {
        let mut documents = Documents::default();
        let opened = lsp_server::Notification::new(
            lsp_types::notification::DidOpenTextDocument::METHOD.to_string(),
            lsp_types::DidOpenTextDocumentParams {
                text_document: lsp_types::TextDocumentItem {
                    uri: uri(),
                    language_id: "zdeceptron".to_string(),
                    version: 1,
                    text: "state a is client Whole from missing\n".to_string(),
                },
            },
        );
        let published = accept(&mut documents, opened).expect("diagnostics for the open document");
        assert_eq!(published.diagnostics.len(), 1);
        assert!(documents.open.contains_key(&uri()));

        let changed = lsp_server::Notification::new(
            lsp_types::notification::DidChangeTextDocument::METHOD.to_string(),
            lsp_types::DidChangeTextDocumentParams {
                text_document: lsp_types::VersionedTextDocumentIdentifier {
                    uri: uri(),
                    version: 2,
                },
                content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: "state a is client Whole starting 0\n".to_string(),
                }],
            },
        );
        let published = accept(&mut documents, changed).expect("diagnostics after the change");
        assert!(published.diagnostics.is_empty(), "{published:?}");
    }

    #[test]
    fn closing_a_document_clears_its_diagnostics() {
        let mut documents = Documents::default();
        documents.open.insert(
            uri(),
            Analysis::of("state a is client Whole from missing\n"),
        );

        let closed = lsp_server::Notification::new(
            lsp_types::notification::DidCloseTextDocument::METHOD.to_string(),
            lsp_types::DidCloseTextDocumentParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: uri() },
            },
        );
        let published = accept(&mut documents, closed).expect("a clearing publish");
        assert!(published.diagnostics.is_empty());
        assert!(documents.open.is_empty());
    }

    /// A notification the server does not understand, or one whose
    /// parameters are nonsense, must be dropped rather than propagated.
    #[test]
    fn a_malformed_notification_is_ignored() {
        let mut documents = Documents::default();
        let nonsense = lsp_server::Notification::new(
            lsp_types::notification::DidOpenTextDocument::METHOD.to_string(),
            serde_json::json!({ "not": "the right shape" }),
        );
        assert!(accept(&mut documents, nonsense).is_none());

        let unknown = lsp_server::Notification::new(
            "textDocument/didSomethingElse".to_string(),
            serde_json::json!({}),
        );
        assert!(accept(&mut documents, unknown).is_none());
    }

    #[test]
    fn an_unknown_request_is_answered_with_an_error_rather_than_a_crash() {
        let documents = Documents::default();
        let response = answer(
            &documents,
            Request::new(
                1.into(),
                "textDocument/rename".to_string(),
                serde_json::json!({}),
            ),
        );
        assert!(response.response_result.is_err());
    }

    /// An imported file's URI is built rather than received, so it has to
    /// be one the editor would have sent for the same file. These are the
    /// paths where the two spellings could come apart.
    #[test]
    fn a_path_becomes_a_uri_that_decodes_back_to_it() {
        let paths = [
            "/tmp/app.zd",
            "/tmp/my project/app.zd",
            "/tmp/a(1)/app.zd",
            "/tmp/\u{4e2d}\u{6587}/app.zd",
            "/tmp/100%/app.zd",
            "/tmp/a b#c/app.zd",
        ];
        for path in paths {
            let uri = path_uri(std::path::Path::new(path)).expect("an absolute utf-8 path");
            assert_eq!(
                file_path(&uri).as_deref(),
                Some(std::path::Path::new(path)),
                "for {path}"
            );
        }
        // A relative path names no file on this machine and must not
        // become a URI that claims to.
        assert_eq!(path_uri(std::path::Path::new("app.zd")), None);
    }

    /// A module's path is the importing file's directory joined with the
    /// specifier as written, so it arrives with the `.` in it.
    #[test]
    fn a_path_with_dot_segments_names_the_same_file_as_one_without() {
        for (written, plain) in [
            ("/tmp/project/./model.zd", "/tmp/project/model.zd"),
            ("/tmp/project/views/../model.zd", "/tmp/project/model.zd"),
            ("/tmp/./a/./b/../model.zd", "/tmp/a/model.zd"),
        ] {
            assert_eq!(
                path_uri(std::path::Path::new(written)),
                path_uri(std::path::Path::new(plain)),
                "for {written}"
            );
        }
    }

    #[test]
    fn a_request_with_bad_parameters_is_answered_with_an_error() {
        let documents = Documents::default();
        let response = answer(
            &documents,
            Request::new(
                1.into(),
                lsp_types::request::HoverRequest::METHOD.to_string(),
                serde_json::json!({ "nope": true }),
            ),
        );
        assert!(response.response_result.is_err());
    }

    /// A request about a file the editor never opened is a `null` answer,
    /// not an error and not a panic.
    #[test]
    fn a_request_for_an_unopened_document_answers_nothing() {
        let documents = Documents::default();
        let response = answer(
            &documents,
            Request::new(
                1.into(),
                lsp_types::request::HoverRequest::METHOD.to_string(),
                serde_json::to_value(lsp_types::HoverParams {
                    text_document_position_params: lsp_types::TextDocumentPositionParams {
                        text_document: lsp_types::TextDocumentIdentifier { uri: uri() },
                        position: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                    work_done_progress_params: Default::default(),
                })
                .expect("serializable"),
            ),
        );
        assert_eq!(response.response_result.ok(), Some(serde_json::Value::Null));
    }

    #[test]
    fn hover_answers_through_the_protocol() {
        let mut documents = Documents::default();
        let src = "state count is client Whole starting 0\nview\n    Text count\n";
        documents.open.insert(uri(), Analysis::of(src));

        let response = answer(
            &documents,
            Request::new(
                1.into(),
                lsp_types::request::HoverRequest::METHOD.to_string(),
                serde_json::to_value(lsp_types::HoverParams {
                    text_document_position_params: lsp_types::TextDocumentPositionParams {
                        text_document: lsp_types::TextDocumentIdentifier { uri: uri() },
                        position: Position {
                            line: 2,
                            character: 9,
                        },
                    },
                    work_done_progress_params: Default::default(),
                })
                .expect("serializable"),
            ),
        );
        let encoded = response.response_result.expect("a result").to_string();
        assert!(encoded.contains("browser memory"), "{encoded}");
    }

    #[test]
    fn semantic_tokens_answer_through_the_protocol() {
        let mut documents = Documents::default();
        documents.open.insert(
            uri(),
            Analysis::of("state count is client Whole starting 0\n"),
        );

        let response = answer(
            &documents,
            Request::new(
                1.into(),
                lsp_types::request::SemanticTokensFullRequest::METHOD.to_string(),
                serde_json::to_value(lsp_types::SemanticTokensParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri: uri() },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                })
                .expect("serializable"),
            ),
        );
        let result: SemanticTokensResult =
            serde_json::from_value(response.response_result.expect("a result")).expect("tokens");
        let SemanticTokensResult::Tokens(tokens) = result else {
            panic!("expected a full token set");
        };
        assert!(!tokens.data.is_empty());
    }
}
