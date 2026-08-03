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
            let analysis = Analysis::of(&params.text_document.text);
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
            let analysis = Analysis::of(&text);
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
            let (start, end) = analysis.lines().range(analysis.text(), span);
            Some(GotoDefinitionResponse::Scalar(Location {
                uri,
                range: Range {
                    start: point(start),
                    end: point(end),
                },
            }))
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
    #[test]
    fn a_spanless_diagnostic_still_becomes_a_range() {
        let analysis = Analysis::of("");
        let published = publish(&uri(), &analysis);
        for diagnostic in &published.diagnostics {
            assert_eq!(diagnostic.range.start.line, diagnostic.range.end.line);
        }
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
