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

use std::collections::{HashMap, HashSet};
use std::error::Error;

use lsp_server::{Connection, ExtractError, Message, Request, RequestId, Response};
use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall,
    CallHierarchyServerCapability, CodeAction, CodeActionKind, CodeActionOrCommand,
    CodeActionProviderCapability, CompletionItem, CompletionItemKind, CompletionOptions,
    CompletionResponse, Diagnostic, DiagnosticSeverity, DocumentHighlight, DocumentHighlightKind,
    DocumentSymbol, DocumentSymbolResponse, FoldingRange, FoldingRangeKind,
    FoldingRangeProviderCapability, GotoDefinitionResponse, Hover, HoverContents,
    HoverProviderCapability, InlayHint, InlayHintKind, InlayHintLabel, Location, MarkupContent,
    MarkupKind, OneOf, ParameterInformation, ParameterLabel, Position, PrepareRenameResponse,
    PublishDiagnosticsParams, Range, RenameOptions, SemanticToken, SemanticTokenModifier,
    SemanticTokenType, SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensOptions, SemanticTokensRangeResult, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, SignatureHelp, SignatureHelpOptions,
    SignatureInformation, SymbolInformation, SymbolKind, TextDocumentSyncCapability,
    TextDocumentSyncKind, TypeDefinitionProviderCapability, Uri,
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
        //
        // Written out rather than given as a bare `Kind`, because a bare
        // kind asks for changes and nothing else, and a save is the event
        // after which a file this document imports may have become a
        // different file.
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            lsp_types::TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                save: Some(lsp_types::TextDocumentSyncSaveOptions::SaveOptions(
                    lsp_types::SaveOptions {
                        // The buffer is already current, because a save
                        // follows the change that prompted it, so asking
                        // for the text again would send the file twice.
                        include_text: Some(false),
                    },
                )),
                ..Default::default()
            },
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_highlight_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        // `prepare` is advertised so that an editor refuses a rename it
        // cannot complete *before* the programmer types a new name,
        // rather than after, which is the difference between a feature
        // that is unavailable and one that appears to have failed.
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: Default::default(),
        })),
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
                // The range form, so a client colouring what is on screen
                // does not have to ask for the whole document to do it.
                range: Some(true),
                ..Default::default()
            },
        )),
        type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
        folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        signature_help_provider: Some(SignatureHelpOptions {
            // A call is written `f with a, b`, so the list becomes worth
            // showing at the space after `with` and again after each
            // comma. There is no bracket to trigger on.
            trigger_characters: Some(vec![" ".to_string(), ",".to_string()]),
            retrigger_characters: Some(vec![",".to_string()]),
            work_done_progress_options: Default::default(),
        }),
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
                for published in accept(&mut documents, notification) {
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

/// Take a document change, returning the diagnostics to publish.
///
/// More than one document's worth, because a save is not local: a module
/// is read from disk by every file that imports it, so saving one file
/// changes what its importers compile against. A server that answered
/// only for the document that changed would leave a stale error showing
/// in a neighbouring window, and a stale error is the kind of wrong that
/// makes a programmer stop reading the ones that are right.
fn accept(
    documents: &mut Documents,
    notification: lsp_server::Notification,
) -> Vec<PublishDiagnosticsParams> {
    use lsp_types::notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    };

    match notification.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let Ok(params) =
                serde_json::from_value::<lsp_types::DidOpenTextDocumentParams>(notification.params)
            else {
                return Vec::new();
            };
            let uri = params.text_document.uri;
            let analysis =
                Analysis::of_document(file_path(&uri).as_deref(), &params.text_document.text);
            let published = publish(&uri, &analysis);
            documents.open.insert(uri, analysis);
            vec![published]
        }
        DidChangeTextDocument::METHOD => {
            let Ok(params) = serde_json::from_value::<lsp_types::DidChangeTextDocumentParams>(
                notification.params,
            ) else {
                return Vec::new();
            };
            // Full sync, so the last change carries the whole document.
            let Some(change) = params.content_changes.into_iter().next_back() else {
                return Vec::new();
            };
            let uri = params.text_document.uri;
            let analysis = Analysis::of_document(file_path(&uri).as_deref(), &change.text);
            let published = publish(&uri, &analysis);
            documents.open.insert(uri, analysis);
            vec![published]
        }
        DidSaveTextDocument::METHOD => {
            let Ok(params) =
                serde_json::from_value::<lsp_types::DidSaveTextDocumentParams>(notification.params)
            else {
                return Vec::new();
            };
            // The saved document's own text is already in hand: a save
            // follows the change that made it worth saving, and the
            // server asks not to be sent the text again. What has changed
            // is the *disk*, which is what every other open document is
            // compiled against.
            let _ = params;
            reanalyse(documents)
        }
        DidCloseTextDocument::METHOD => {
            let Ok(params) = serde_json::from_value::<lsp_types::DidCloseTextDocumentParams>(
                notification.params,
            ) else {
                return Vec::new();
            };
            documents.open.remove(&params.text_document.uri);
            // An empty list clears whatever was showing for the file.
            vec![PublishDiagnosticsParams {
                uri: params.text_document.uri,
                diagnostics: Vec::new(),
                version: None,
            }]
        }
        _ => Vec::new(),
    }
}

/// Re-run the compiler over every open document and publish for each.
///
/// What a save changes on disk is invisible to an analysis that already
/// ran, and the file that changed is not the only one affected: a module
/// is read by whatever imports it. Re-running everything open is the only
/// answer that cannot be stale, and the cost is one pass per open window
/// on an event that happens at human speed rather than per keystroke.
///
/// Ordered by URI so that the notifications go out in the same order
/// every time, whatever order the map happens to iterate in.
fn reanalyse(documents: &mut Documents) -> Vec<PublishDiagnosticsParams> {
    let mut texts: Vec<(Uri, String)> = documents
        .open
        .iter()
        .map(|(uri, analysis)| (uri.clone(), analysis.text().to_string()))
        .collect();
    texts.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));

    let mut published = Vec::with_capacity(texts.len());
    for (uri, text) in texts {
        let analysis = Analysis::of_document(file_path(&uri).as_deref(), &text);
        published.push(publish(&uri, &analysis));
        documents.open.insert(uri, analysis);
    }
    published
}

fn answer(documents: &Documents, request: Request) -> Response {
    use lsp_types::request::{
        CallHierarchyIncomingCalls, CallHierarchyOutgoingCalls, CallHierarchyPrepare,
        CodeActionRequest, Completion, DocumentHighlightRequest, DocumentSymbolRequest,
        FoldingRangeRequest, GotoDefinition, GotoTypeDefinition, HoverRequest, InlayHintRequest,
        PrepareRenameRequest, References, Rename, SemanticTokensFullRequest,
        SemanticTokensRangeRequest, SignatureHelpRequest, WorkspaceSymbolRequest,
    };

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

        References::METHOD => reply(id, request, |request: lsp_types::ReferenceParams| {
            let uri = request.text_document_position.text_document.uri;
            let (analysis, offset) =
                locate(documents, &uri, request.text_document_position.position)?;
            let declaration = crate::refs::target(analysis, offset)
                .and_then(|target| crate::refs::declaration(analysis, target));
            let found: Vec<Location> = crate::refs::references(analysis, offset)
                .into_iter()
                // The protocol lets a client ask for the uses alone, and
                // an editor that shows the definition separately does.
                .filter(|span| request.context.include_declaration || Some(*span) != declaration)
                .filter_map(|span| location(analysis, &uri, span))
                .collect();
            Some(found)
        }),

        DocumentSymbolRequest::METHOD => {
            reply(id, request, |request: lsp_types::DocumentSymbolParams| {
                let analysis = documents.open.get(&request.text_document.uri)?;
                let found: Vec<DocumentSymbol> = crate::outline::document_declarations(analysis)
                    .into_iter()
                    .map(|declaration| {
                        let (start, end) =
                            analysis.lines().range(analysis.text(), declaration.span);
                        let (name_start, name_end) = analysis
                            .lines()
                            .range(analysis.text(), declaration.selection);
                        #[allow(deprecated)]
                        DocumentSymbol {
                            name: declaration.name,
                            detail: Some(detail(declaration.kind).to_string()),
                            kind: symbol_kind(declaration.kind),
                            tags: None,
                            // `deprecated` is deprecated in the protocol
                            // and replaced by `tags`, but the struct still
                            // carries the field, so it has to be written.
                            deprecated: None,
                            range: Range {
                                start: point(start),
                                end: point(end),
                            },
                            selection_range: Range {
                                start: point(name_start),
                                end: point(name_end),
                            },
                            // Flat. A declaration's parts, a signal's
                            // type or a component's parameters, are not
                            // declarations, and inventing children for
                            // them would put things in an outline that
                            // nothing else in this server treats as
                            // symbols.
                            children: None,
                        }
                    })
                    .collect();
                Some(DocumentSymbolResponse::Nested(found))
            })
        }

        WorkspaceSymbolRequest::METHOD => {
            reply(id, request, |request: lsp_types::WorkspaceSymbolParams| {
                // The workspace is whatever the open documents reach. A
                // server with no folder handed to it has no other honest
                // definition of one, and every file a program imports is
                // reachable from the program that imports it.
                let mut found: Vec<SymbolInformation> = Vec::new();
                let mut seen: HashSet<(String, String, u32, u32)> = HashSet::new();
                for (uri, analysis) in &documents.open {
                    for declaration in crate::outline::declarations(analysis) {
                        if !matches(&declaration.name, &request.query) {
                            continue;
                        }
                        let Some(at) = location(analysis, uri, declaration.selection) else {
                            continue;
                        };
                        // Two open documents importing one module would
                        // otherwise report that module's declarations
                        // twice, which is a fact about the editor's tabs
                        // rather than about the program.
                        let key = (
                            declaration.name.clone(),
                            at.uri.as_str().to_string(),
                            at.range.start.line,
                            at.range.start.character,
                        );
                        if !seen.insert(key) {
                            continue;
                        }
                        #[allow(deprecated)]
                        found.push(SymbolInformation {
                            name: declaration.name,
                            kind: symbol_kind(declaration.kind),
                            tags: None,
                            deprecated: None,
                            location: at,
                            container_name: None,
                        });
                    }
                }
                // By file and position after the name, because the map
                // the documents came out of has no order and two files
                // may declare the same word.
                found.sort_by(|left, right| {
                    (
                        &left.name,
                        left.location.uri.as_str(),
                        left.location.range.start.line,
                    )
                        .cmp(&(
                            &right.name,
                            right.location.uri.as_str(),
                            right.location.range.start.line,
                        ))
                });
                Some(found)
            })
        }

        DocumentHighlightRequest::METHOD => {
            reply(
                id,
                request,
                |request: lsp_types::DocumentHighlightParams| {
                    let (analysis, offset) = locate(
                        documents,
                        &request.text_document_position_params.text_document.uri,
                        request.text_document_position_params.position,
                    )?;
                    let declaration = crate::refs::target(analysis, offset)
                        .and_then(|target| crate::refs::declaration(analysis, target));
                    let found: Vec<DocumentHighlight> = crate::refs::references(analysis, offset)
                        .into_iter()
                        // One file's worth, since a highlight is drawn in the
                        // window the cursor is in.
                        .filter(|span| analysis.in_document(*span))
                        .map(|span| {
                            let (start, end) = analysis.lines().range(analysis.text(), span);
                            DocumentHighlight {
                                range: Range {
                                    start: point(start),
                                    end: point(end),
                                },
                                // The declaration is a write. Everything else
                                // is left as plain text rather than claimed to
                                // be a read: a mutation target and a value
                                // read are one kind in this index, and calling
                                // both of them reads would be a claim it
                                // cannot support.
                                kind: Some(if Some(span) == declaration {
                                    DocumentHighlightKind::WRITE
                                } else {
                                    DocumentHighlightKind::TEXT
                                }),
                            }
                        })
                        .collect();
                    Some(found)
                },
            )
        }

        PrepareRenameRequest::METHOD => {
            reply(
                id,
                request,
                |request: lsp_types::TextDocumentPositionParams| {
                    let (analysis, offset) =
                        locate(documents, &request.text_document.uri, request.position)?;
                    // The symbol's own span, so the editor pre-fills the box
                    // with the name being changed. Refusing here is what stops
                    // a rename of something whose occurrences are not all
                    // findable from being started at all.
                    crate::refs::target(analysis, offset)?;
                    let symbol = analysis.symbols().at(offset)?;
                    let (start, end) = analysis.lines().range(analysis.text(), symbol.span);
                    Some(PrepareRenameResponse::RangeWithPlaceholder {
                        range: Range {
                            start: point(start),
                            end: point(end),
                        },
                        placeholder: symbol.name.clone(),
                    })
                },
            )
        }

        Rename::METHOD => reply(id, request, |request: lsp_types::RenameParams| {
            let uri = request.text_document_position.text_document.uri;
            let (analysis, offset) =
                locate(documents, &uri, request.text_document_position.position)?;
            let spans = crate::refs::rename(analysis, offset, &request.new_name)?;

            // Grouped by file, because a rename crosses module boundaries
            // and the protocol's unit of edit is a document.
            //
            // `Uri` is the key `WorkspaceEdit` is defined with, so there
            // is no other type to use. Its one interior-mutable field is
            // `fluent_uri`'s cached offset of the authority, which is
            // parse bookkeeping: equality and hashing are of the text,
            // and nothing here mutates a key after inserting it.
            #[allow(clippy::mutable_key_type)]
            let mut changes: HashMap<Uri, Vec<lsp_types::TextEdit>> = HashMap::new();
            for span in spans {
                let Some(at) = location(analysis, &uri, span) else {
                    // A span whose file cannot be named is a rename that
                    // would be applied to some of its occurrences and not
                    // others, so the whole edit is abandoned.
                    return None;
                };
                changes
                    .entry(at.uri)
                    .or_default()
                    .push(lsp_types::TextEdit {
                        range: at.range,
                        new_text: request.new_name.clone(),
                    });
            }
            Some(lsp_types::WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            })
        }),

        GotoTypeDefinition::METHOD => {
            reply(id, request, |request: lsp_types::GotoDefinitionParams| {
                let uri = request.text_document_position_params.text_document.uri;
                let (analysis, offset) = locate(
                    documents,
                    &uri,
                    request.text_document_position_params.position,
                )?;
                let span = crate::typedef::type_definition(analysis, offset)?;
                Some(GotoDefinitionResponse::Scalar(location(
                    analysis, &uri, span,
                )?))
            })
        }

        FoldingRangeRequest::METHOD => {
            reply(id, request, |request: lsp_types::FoldingRangeParams| {
                let analysis = documents.open.get(&request.text_document.uri)?;
                let found: Vec<FoldingRange> = crate::folds::folds(analysis)
                    .into_iter()
                    .map(|fold| FoldingRange {
                        start_line: fold.start_line,
                        end_line: fold.end_line,
                        // Line-based: a block here begins after a line
                        // break by construction, so naming columns would
                        // add precision the layout pass does not have.
                        start_character: None,
                        end_character: None,
                        kind: Some(FoldingRangeKind::Region),
                        collapsed_text: None,
                    })
                    .collect();
                Some(found)
            })
        }

        InlayHintRequest::METHOD => {
            reply(id, request, |request: lsp_types::InlayHintParams| {
                let analysis = documents.open.get(&request.text_document.uri)?;
                let found: Vec<InlayHint> =
                    crate::hints::hints(analysis, request.range.start.line, request.range.end.line)
                        .into_iter()
                        .map(|hint| {
                            let at = analysis.lines().position(analysis.text(), hint.at);
                            InlayHint {
                                position: point(at),
                                label: InlayHintLabel::String(hint.label),
                                kind: Some(InlayHintKind::TYPE),
                                text_edits: None,
                                tooltip: None,
                                // A space before, none after: the hint sits
                                // between a name and whatever follows it, and the
                                // language puts a space there.
                                padding_left: Some(true),
                                padding_right: Some(false),
                                data: None,
                            }
                        })
                        .collect();
                Some(found)
            })
        }

        SignatureHelpRequest::METHOD => {
            reply(id, request, |request: lsp_types::SignatureHelpParams| {
                let (analysis, offset) = locate(
                    documents,
                    &request.text_document_position_params.text_document.uri,
                    request.text_document_position_params.position,
                )?;
                let found = crate::signature::signature(analysis, offset)?;
                let parameters = found
                    .parameters
                    .iter()
                    .map(|parameter| ParameterInformation {
                        // By label rather than by offset: the offsets the
                        // protocol wants are in UTF-16 units of the
                        // signature string, and a label that appears once
                        // is unambiguous without counting them.
                        label: ParameterLabel::Simple(parameter.clone()),
                        documentation: None,
                    })
                    .collect();
                Some(SignatureHelp {
                    signatures: vec![SignatureInformation {
                        label: found.label,
                        documentation: None,
                        parameters: Some(parameters),
                        active_parameter: Some(found.active),
                    }],
                    active_signature: Some(0),
                    active_parameter: Some(found.active),
                })
            })
        }

        CodeActionRequest::METHOD => reply(id, request, |request: lsp_types::CodeActionParams| {
            let uri = request.text_document.uri;
            let analysis = documents.open.get(&uri)?;
            let span = zdc_lexer::Span::new(
                offset_of(analysis, request.range.start),
                offset_of(analysis, request.range.end),
            );
            let found: Vec<CodeActionOrCommand> = crate::actions::actions(analysis, span)
                .into_iter()
                .map(|action| {
                    let (start, end) = analysis.lines().range(analysis.text(), action.at);
                    let edit = lsp_types::TextEdit {
                        range: Range {
                            start: point(start),
                            end: point(end),
                        },
                        new_text: action.insert,
                    };
                    CodeActionOrCommand::CodeAction(CodeAction {
                        title: action.title,
                        kind: Some(CodeActionKind::QUICKFIX),
                        diagnostics: Some(
                            publish(&uri, analysis)
                                .diagnostics
                                .into_iter()
                                .filter(|diagnostic| {
                                    diagnostic.message == rendered(&action.diagnostic)
                                })
                                .collect(),
                        ),
                        edit: Some(lsp_types::WorkspaceEdit {
                            changes: Some(HashMap::from([(uri.clone(), vec![edit])])),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                })
                .collect();
            Some(found)
        }),

        CallHierarchyPrepare::METHOD => reply(
            id,
            request,
            |request: lsp_types::CallHierarchyPrepareParams| {
                let uri = request.text_document_position_params.text_document.uri;
                let (analysis, offset) = locate(
                    documents,
                    &uri,
                    request.text_document_position_params.position,
                )?;
                Some(vec![hierarchy_item(
                    analysis,
                    &uri,
                    crate::calls::callable_at(analysis, offset)?,
                )?])
            },
        ),

        CallHierarchyIncomingCalls::METHOD => reply(
            id,
            request,
            |request: lsp_types::CallHierarchyIncomingCallsParams| {
                let (uri, analysis, def) = anchor(documents, &request.item)?;
                let found: Vec<CallHierarchyIncomingCall> = crate::calls::incoming(analysis, def)
                    .into_iter()
                    .filter_map(|edge| {
                        Some(CallHierarchyIncomingCall {
                            from_ranges: ranges(analysis, &edge.sites),
                            from: hierarchy_item(analysis, uri, edge.callable)?,
                        })
                    })
                    .collect();
                Some(found)
            },
        ),

        CallHierarchyOutgoingCalls::METHOD => reply(
            id,
            request,
            |request: lsp_types::CallHierarchyOutgoingCallsParams| {
                let (uri, analysis, def) = anchor(documents, &request.item)?;
                let found: Vec<CallHierarchyOutgoingCall> = crate::calls::outgoing(analysis, def)
                    .into_iter()
                    .filter_map(|edge| {
                        Some(CallHierarchyOutgoingCall {
                            from_ranges: ranges(analysis, &edge.sites),
                            to: hierarchy_item(analysis, uri, edge.callable)?,
                        })
                    })
                    .collect();
                Some(found)
            },
        ),

        SemanticTokensFullRequest::METHOD => {
            reply(id, request, |request: lsp_types::SemanticTokensParams| {
                let analysis = documents.open.get(&request.text_document.uri)?;
                Some(SemanticTokensResult::Tokens(SemanticTokens {
                    result_id: None,
                    data: semantic_tokens(analysis, 0, u32::MAX),
                }))
            })
        }

        SemanticTokensRangeRequest::METHOD => reply(
            id,
            request,
            |request: lsp_types::SemanticTokensRangeParams| {
                let analysis = documents.open.get(&request.text_document.uri)?;
                Some(SemanticTokensRangeResult::Tokens(SemanticTokens {
                    result_id: None,
                    data: semantic_tokens(
                        analysis,
                        request.range.start.line,
                        request.range.end.line,
                    ),
                }))
            },
        ),

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

/// A byte offset in an open document, from a protocol position.
fn offset_of(analysis: &Analysis, position: Position) -> u32 {
    analysis.lines().offset(
        analysis.text(),
        crate::lines::Position {
            line: position.line,
            character: position.character,
        },
    )
}

/// The message a compiler diagnostic is published under, so a code action
/// can be attached to the published copy of the one it repairs.
fn rendered(diagnostic: &zdc_diagnostics::Diagnostic) -> String {
    match &diagnostic.help {
        Some(help) => format!("{}\n\nhelp: {help}", diagnostic.message),
        None => diagnostic.message.clone(),
    }
}

/// A callable, as the item a call hierarchy is navigated by.
fn hierarchy_item(
    analysis: &Analysis,
    here: &Uri,
    callable: crate::calls::Callable,
) -> Option<CallHierarchyItem> {
    let whole = location(analysis, here, callable.span)?;
    let name = location(analysis, here, callable.selection)?;
    Some(CallHierarchyItem {
        name: callable.name,
        kind: SymbolKind::FUNCTION,
        tags: None,
        detail: None,
        uri: whole.uri,
        range: whole.range,
        selection_range: name.range,
        // The document the hierarchy was asked from, which is not always
        // the document the item lives in: a function declared in an
        // imported file has no window of its own, and the analysis that
        // can answer about it is the importing one. The protocol carries
        // this back untouched on the next request, which is what `data`
        // is for.
        data: Some(serde_json::Value::String(here.as_str().to_string())),
    })
}

/// The analysis and the callable an editor is asking the hierarchy of.
///
/// The item is looked up again rather than trusted: an editor may hold
/// one across an edit, and a stale handle should answer about whatever is
/// at that place now, or about nothing.
fn anchor<'a>(
    documents: &'a Documents,
    item: &CallHierarchyItem,
) -> Option<(&'a Uri, &'a Analysis, zdc_hir::DefId)> {
    let serde_json::Value::String(document) = item.data.as_ref()? else {
        return None;
    };
    let (document, analysis) = documents
        .open
        .iter()
        .find(|(uri, _)| uri.as_str() == document)?;

    // Matched on where the name is rather than on what it is called, so
    // two declarations sharing a name in two files stay distinct. Each
    // candidate is rendered against the originating document, exactly as
    // it was when the item was handed out. Passing the item's own URI
    // here instead would make every in-document candidate match, because
    // that is the URI an in-document span is answered with.
    let found = crate::outline::declarations(analysis)
        .into_iter()
        .find(|d| {
            location(analysis, document, d.selection).is_some_and(|at| {
                at.uri == item.uri && at.range.start == item.selection_range.start
            })
        })?;
    let callable = crate::calls::callable_at(analysis, found.selection.start)?;
    Some((document, analysis, callable.def))
}

/// Call sites as ranges in the document they were found in.
fn ranges(analysis: &Analysis, sites: &[zdc_lexer::Span]) -> Vec<Range> {
    sites
        .iter()
        .map(|span| {
            let found = analysis.locate(*span);
            let lines = LineIndex::new(found.text);
            let (start, end) = lines.range(found.text, found.span);
            Range {
                start: point(start),
                end: point(end),
            }
        })
        .collect()
}

/// The highlights on lines `from` to `to`, in the protocol's encoding.
///
/// One function for both the whole-document form and the range form, so
/// the two cannot come to colour the same line differently. The delta
/// encoding is relative to the first token *returned*, which is what the
/// protocol asks for in both cases.
fn semantic_tokens(analysis: &Analysis, from: u32, to: u32) -> Vec<SemanticToken> {
    crate::tokens::encode(&crate::tokens::highlights_within(analysis, from, to))
        .chunks_exact(5)
        .map(|five| SemanticToken {
            delta_line: five[0],
            delta_start: five[1],
            length: five[2],
            token_type: five[3],
            token_modifiers_bitset: five[4],
        })
        .collect()
}

/// The protocol's nearest name for one of this language's declaration
/// forms.
///
/// Every arm written out, so a declaration form added to the language has
/// to be given a protocol kind here rather than inheriting one.
fn symbol_kind(kind: crate::outline::DeclarationKind) -> SymbolKind {
    use crate::outline::DeclarationKind;

    match kind {
        // A signal is a variable whose placement is the interesting part,
        // and the placement is in the detail line rather than in the kind:
        // the protocol has no vocabulary for it, and picking a different
        // icon per placement would say something the icons do not mean.
        DeclarationKind::Signal(_) => SymbolKind::VARIABLE,
        DeclarationKind::Function => SymbolKind::FUNCTION,
        DeclarationKind::Release => SymbolKind::FUNCTION,
        DeclarationKind::Foreign => SymbolKind::FUNCTION,
        DeclarationKind::View => SymbolKind::MODULE,
        DeclarationKind::Record => SymbolKind::STRUCT,
        // A route is a choice plus a bijection onto URLs (§14G.2), so it
        // takes a choice's kind.
        DeclarationKind::Choice | DeclarationKind::Route => SymbolKind::ENUM,
        DeclarationKind::Component => SymbolKind::CLASS,
    }
}

/// The word the language itself uses for a declaration form, which is
/// what an outline should show beside the name.
fn detail(kind: crate::outline::DeclarationKind) -> &'static str {
    use crate::outline::DeclarationKind;
    use zdc_ast::Placement;

    match kind {
        DeclarationKind::Signal(Placement::Client) => "client state",
        DeclarationKind::Signal(Placement::Static) => "static state",
        DeclarationKind::Signal(Placement::Server) => "server state",
        DeclarationKind::Signal(Placement::Durable) => "durable state",
        DeclarationKind::Function => "function",
        DeclarationKind::View => "view",
        DeclarationKind::Record => "record",
        DeclarationKind::Choice => "choice",
        DeclarationKind::Component => "component",
        DeclarationKind::Foreign => "foreign",
        DeclarationKind::Release => "release",
        DeclarationKind::Route => "route",
    }
}

/// Whether a declaration's name answers a workspace query.
///
/// Case-insensitive substring, which is what the protocol says a query is
/// and what every client's own filter then narrows further. An empty
/// query asks for everything.
fn matches(name: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(&query.to_lowercase())
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

    /// The one publish a notification about one document produces.
    ///
    /// Asserting the count rather than taking the first is the point: a
    /// change to one file must not start publishing for others, and the
    /// only event that publishes for more than one is a save.
    fn one(published: Vec<PublishDiagnosticsParams>, why: &str) -> PublishDiagnosticsParams {
        assert_eq!(published.len(), 1, "{why}: {published:?}");
        published.into_iter().next().expect("just counted")
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
            one(
                accept(documents, opened),
                "the open notification is understood",
            );
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

    /// Find-references over a module boundary has to reach three kinds of
    /// place at once: the declaration in the imported file, the name on
    /// the `use` line that borrowed it, and the call in the entry file.
    /// The `use` line is the one nothing else would have found, being a
    /// name in no syntax tree and in no index, and it is also the one a
    /// rename must not miss.
    #[test]
    fn find_references_across_a_use_reaches_both_files_and_the_use_line() {
        let project = Project::new("refs-across-use");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let response = answer(
            &documents,
            request(
                lsp_types::request::References::METHOD,
                lsp_types::ReferenceParams {
                    text_document_position: document_position(&app, APP, "double with 2"),
                    context: lsp_types::ReferenceContext {
                        include_declaration: true,
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let found: Vec<Location> =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a list of locations");

        assert_eq!(found.len(), 3, "{found:?}");
        let model = project.uri("model.zd");
        assert_eq!(
            found.iter().filter(|at| at.uri == model).count(),
            1,
            "the declaration, in the file that declares it: {found:?}"
        );
        assert_eq!(
            found.iter().filter(|at| at.uri == app).count(),
            2,
            "the `use` line and the call: {found:?}"
        );

        let mut in_app: Vec<Position> = found
            .iter()
            .filter(|at| at.uri == app)
            .map(|at| at.range.start)
            .collect();
        in_app.sort_by_key(|at| (at.line, at.character));
        assert_eq!(in_app[0], position(APP, "double\n"), "the `use` line");
        assert_eq!(in_app[1], position(APP, "double with 2"), "the call");
    }

    /// The declaration can be left out, and then it is the only thing
    /// left out.
    #[test]
    fn find_references_can_omit_the_declaration_it_would_otherwise_carry() {
        let project = Project::new("refs-no-declaration");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let response = answer(
            &documents,
            request(
                lsp_types::request::References::METHOD,
                lsp_types::ReferenceParams {
                    text_document_position: document_position(&app, APP, "double with 2"),
                    context: lsp_types::ReferenceContext {
                        include_declaration: false,
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let found: Vec<Location> =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a list of locations");

        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found.iter().all(|at| at.uri == app),
            "only the importing file's own occurrences remain: {found:?}"
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
        let published = one(
            accept(&mut documents, opened),
            "diagnostics for the open document",
        );
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
        let published = one(
            accept(&mut documents, changed),
            "diagnostics after the change",
        );
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
        let published = one(accept(&mut documents, closed), "a clearing publish");
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
        assert!(accept(&mut documents, nonsense).is_empty());

        let unknown = lsp_server::Notification::new(
            "textDocument/didSomethingElse".to_string(),
            serde_json::json!({}),
        );
        assert!(accept(&mut documents, unknown).is_empty());
    }

    /// This named `textDocument/rename` until the server started
    /// answering it. The method here has to be one nothing answers, or
    /// the test stops checking the fall-through it was written for.
    #[test]
    fn an_unknown_request_is_answered_with_an_error_rather_than_a_crash() {
        let documents = Documents::default();
        let response = answer(
            &documents,
            Request::new(
                1.into(),
                "textDocument/linkedEditingRange".to_string(),
                serde_json::json!({}),
            ),
        );
        assert!(response.response_result.is_err());
    }

    /// A rename over a module boundary must rewrite the declaration in
    /// the file that owns it, the `use` line that borrowed the name, and
    /// the call. Leaving any one of the three is a file that no longer
    /// compiles, which is why this is the feature that had to wait for
    /// the go-to-definition defect to be fixed first.
    // See the note at the map's construction: the key is the protocol's
    // own, and its interior mutability is `fluent_uri`'s parse cache.
    #[allow(clippy::mutable_key_type)]
    #[test]
    fn rename_across_a_use_rewrites_every_file_the_name_appears_in() {
        let project = Project::new("rename-across-use");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let response = answer(
            &documents,
            request(
                lsp_types::request::Rename::METHOD,
                lsp_types::RenameParams {
                    text_document_position: document_position(&app, APP, "double with 2"),
                    new_name: "twice".to_string(),
                    work_done_progress_params: Default::default(),
                },
            ),
        );
        let edit: lsp_types::WorkspaceEdit =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a workspace edit");
        let changes = edit.changes.expect("edits grouped by file");

        assert_eq!(changes.len(), 2, "both files are edited: {changes:?}");
        let model = project.uri("model.zd");
        assert_eq!(
            changes.get(&model).map(|edits| edits.len()),
            Some(1),
            "the declaration in the imported file"
        );
        assert_eq!(
            changes.get(&app).map(|edits| edits.len()),
            Some(2),
            "the `use` line and the call"
        );
        assert!(
            changes
                .values()
                .flatten()
                .all(|edit| edit.new_text == "twice"),
            "{changes:?}"
        );

        // The edits describe a program that still resolves. Applying them
        // is what checks that, rather than counting them.
        assert_eq!(
            apply(MODEL, changes.get(&model).expect("model edits")),
            "function twice with n\n    give n + n\n"
        );
        assert_eq!(
            apply(APP, changes.get(&app).expect("app edits")),
            "use \"./model\" for twice\n\
             state total is client Whole from twice with 2\n\
             view\n    Text total\n"
        );
    }

    /// A rename the server cannot complete must be refused before the
    /// programmer types a new name, not after.
    #[test]
    fn preparing_a_rename_of_a_built_in_element_refuses_it() {
        let mut documents = Documents::default();
        let src = "state count is client Whole starting 0\nview\n    Text count\n";
        documents.open.insert(uri(), Analysis::of(src));

        let refused = answer(
            &documents,
            request(
                lsp_types::request::PrepareRenameRequest::METHOD,
                document_position(&uri(), src, "Text count"),
            ),
        );
        assert_eq!(
            refused.response_result.ok(),
            Some(serde_json::Value::Null),
            "a built-in element is not renameable"
        );

        let allowed = answer(
            &documents,
            request(
                lsp_types::request::PrepareRenameRequest::METHOD,
                document_position(&uri(), src, "count\n"),
            ),
        );
        let prepared: PrepareRenameResponse =
            serde_json::from_value(allowed.response_result.expect("a result"))
                .expect("a prepare response");
        let PrepareRenameResponse::RangeWithPlaceholder { range, placeholder } = prepared else {
            panic!("expected the name and its range");
        };
        assert_eq!(placeholder, "count");
        assert_eq!(range.start, position(src, "count\n"));
    }

    /// Highlight is find-references narrowed to the window the cursor is
    /// in, so the imported file's declaration must not appear in it.
    #[test]
    fn document_highlight_stays_inside_the_file_that_was_asked_about() {
        let project = Project::new("highlight-one-file");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let response = answer(
            &documents,
            request(
                lsp_types::request::DocumentHighlightRequest::METHOD,
                lsp_types::DocumentHighlightParams {
                    text_document_position_params: document_position(&app, APP, "double with 2"),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let found: Vec<DocumentHighlight> =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a list of highlights");

        assert_eq!(
            found.len(),
            2,
            "the `use` line and the call, and not the declaration a file away: {found:?}"
        );
        assert!(
            found.iter().all(|at| at.range.start.line < 2),
            "both are in `app.zd`'s own first two lines: {found:?}"
        );
    }

    /// The outline is of the file that was asked about. An importing
    /// file's outline must not list the declarations it borrowed, or the
    /// same declaration appears in two files' outlines and only one of
    /// them can be jumped to.
    #[test]
    fn the_document_outline_lists_this_file_and_not_the_one_it_imports() {
        let project = Project::new("outline-one-file");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let response = answer(
            &documents,
            request(
                lsp_types::request::DocumentSymbolRequest::METHOD,
                lsp_types::DocumentSymbolParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri: app },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let result: DocumentSymbolResponse =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("an outline");
        let DocumentSymbolResponse::Nested(found) = result else {
            panic!("expected the nested form");
        };

        let names: Vec<&str> = found.iter().map(|symbol| symbol.name.as_str()).collect();
        assert_eq!(names, ["total", "view"], "{found:?}");
        assert_eq!(found[0].detail.as_deref(), Some("client state"));
        assert_eq!(found[0].kind, SymbolKind::VARIABLE);
        // The name is inside the declaration, so a breadcrumb contains
        // what it names.
        assert!(found[0].range.start <= found[0].selection_range.start);
        assert!(found[0].selection_range.end <= found[0].range.end);
    }

    /// A workspace search reaches the imported file, which is the whole
    /// difference between it and the outline above.
    #[test]
    fn a_workspace_search_reaches_declarations_in_an_imported_file() {
        let project = Project::new("workspace-symbols");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let response = answer(
            &documents,
            request(
                lsp_types::request::WorkspaceSymbolRequest::METHOD,
                lsp_types::WorkspaceSymbolParams {
                    query: "doub".to_string(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let found: Vec<SymbolInformation> =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a list of symbols");

        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].name, "double");
        assert_eq!(found[0].kind, SymbolKind::FUNCTION);
        assert_eq!(
            found[0].location.uri,
            project.uri("model.zd"),
            "the file that declares it, not the one that imports it"
        );
        assert_eq!(found[0].location.range.start, position(MODEL, "double"));

        // A query that matches nothing is empty rather than everything,
        // which is what makes the filter above load-bearing.
        let empty = answer(
            &documents,
            request(
                lsp_types::request::WorkspaceSymbolRequest::METHOD,
                lsp_types::WorkspaceSymbolParams {
                    query: "nothing-is-called-this".to_string(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let none: Vec<SymbolInformation> =
            serde_json::from_value(empty.response_result.expect("a result"))
                .expect("a list of symbols");
        assert!(none.is_empty(), "{none:?}");
        let _ = app;
    }

    /// A `record` in an imported file, and a value of it in the entry
    /// file. The jump has to land in the file that declares the type.
    #[test]
    fn type_definition_reaches_a_record_declared_in_an_imported_file() {
        let project = Project::new("typedef-across-use");
        let model = "record Item\n    id is Text\n";
        project.write("model.zd", model);
        let app = "use \"./model\" for Item\n\
                   state items is client List of Item starting empty\n\
                   view\n    Text \"hi\"\n";
        let mut documents = Documents::default();
        let uri = project.open(&mut documents, "app.zd", app);

        let response = answer(
            &documents,
            request(
                lsp_types::request::GotoTypeDefinition::METHOD,
                lsp_types::GotoDefinitionParams {
                    text_document_position_params: document_position(&uri, app, "items"),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let result: GotoDefinitionResponse =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a type definition response");
        let GotoDefinitionResponse::Scalar(at) = result else {
            panic!("expected a single location");
        };
        assert_eq!(at.uri, project.uri("model.zd"));
        assert_eq!(at.range.start, position(model, "Item"));
    }

    /// The range form must colour what is on screen and no more, and the
    /// tokens it returns must be the same ones the whole-document form
    /// would have returned for those lines.
    #[test]
    fn semantic_tokens_by_range_are_the_whole_document_s_answer_for_those_lines() {
        let project = Project::new("tokens-by-range");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let whole = answer(
            &documents,
            request(
                lsp_types::request::SemanticTokensFullRequest::METHOD,
                lsp_types::SemanticTokensParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri: app.clone() },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let SemanticTokensResult::Tokens(all) =
            serde_json::from_value(whole.response_result.expect("a result")).expect("tokens")
        else {
            panic!("expected a full token set");
        };

        // `app.zd`'s third line, which is the `view` keyword alone.
        let ranged = answer(
            &documents,
            request(
                lsp_types::request::SemanticTokensRangeRequest::METHOD,
                lsp_types::SemanticTokensRangeParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri: app },
                    range: Range {
                        start: Position {
                            line: 2,
                            character: 0,
                        },
                        end: Position {
                            line: 2,
                            character: 4,
                        },
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let SemanticTokensRangeResult::Tokens(some) =
            serde_json::from_value(ranged.response_result.expect("a result")).expect("tokens")
        else {
            panic!("expected a token set");
        };

        assert!(!all.data.is_empty(), "the fixture must colour something");
        assert_eq!(some.data.len(), 1, "the `view` keyword alone: {some:?}");
        assert!(
            some.data.len() < all.data.len(),
            "the range form must return less than the whole document"
        );
        // Delta-encoded from the start of the document in both forms, so
        // the one token's line delta is its absolute line.
        assert_eq!(some.data[0].delta_line, 2);
        assert_eq!(some.data[0].length, 4);
        assert_eq!(
            some.data[0].token_type,
            all.data
                .iter()
                .scan(0, |line, token| {
                    *line += token.delta_line;
                    Some((*line, token))
                })
                .find(|(line, _)| *line == 2)
                .expect("the same token in the whole-document answer")
                .1
                .token_type,
            "the two forms classify the same token the same way"
        );
    }

    /// Folding is of the document that was asked about, and it is the
    /// block structure the layout pass produced.
    #[test]
    fn folding_ranges_follow_the_blocks_of_the_open_document() {
        let project = Project::new("folding");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let response = answer(
            &documents,
            request(
                lsp_types::request::FoldingRangeRequest::METHOD,
                lsp_types::FoldingRangeParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri: app },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let found: Vec<FoldingRange> =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a list of folding ranges");

        // `app.zd` has exactly one block: the `view` on line 2, whose
        // single node is on line 3. `model.zd`'s block is in another file
        // and must not appear.
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].start_line, 2);
        assert_eq!(found[0].end_line, 3);
        assert_eq!(found[0].kind, Some(FoldingRangeKind::Region));
    }

    /// A save republishes for every open document, not only the one that
    /// was saved. A module read from disk by its importers is a different
    /// file after a save, and the window showing the importer is where
    /// that shows up.
    #[test]
    fn saving_one_file_republishes_for_the_file_that_imports_it() {
        let project = Project::new("save-republishes");
        let mut documents = Documents::default();
        // `model.zd` on disk declares nothing, so `app.zd` cannot resolve
        // the name it imports.
        let model = project.open(&mut documents, "model.zd", "record Other\n    id is Text\n");
        let app = project.open(&mut documents, "app.zd", APP);
        assert!(
            !publish(&app, documents.open.get(&app).expect("open"))
                .diagnostics
                .is_empty(),
            "the importing file starts broken, or this test proves nothing"
        );

        // The programmer fixes `model.zd` and saves it. Nothing at all is
        // sent about `app.zd`.
        let fixed = lsp_server::Notification::new(
            lsp_types::notification::DidChangeTextDocument::METHOD.to_string(),
            lsp_types::DidChangeTextDocumentParams {
                text_document: lsp_types::VersionedTextDocumentIdentifier {
                    uri: model.clone(),
                    version: 2,
                },
                content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: MODEL.to_string(),
                }],
            },
        );
        one(
            accept(&mut documents, fixed),
            "a change publishes for one file",
        );
        project.write("model.zd", MODEL);

        let saved = lsp_server::Notification::new(
            lsp_types::notification::DidSaveTextDocument::METHOD.to_string(),
            lsp_types::DidSaveTextDocumentParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: model.clone() },
                text: None,
            },
        );
        let published = accept(&mut documents, saved);

        assert_eq!(published.len(), 2, "one for each open document");
        let for_app = published
            .iter()
            .find(|params| params.uri == app)
            .expect("the importing file is republished");
        assert!(
            for_app.diagnostics.is_empty(),
            "its error is gone now that the file it imports declares the name: {:?}",
            for_app.diagnostics
        );
        let for_model = published
            .iter()
            .find(|params| params.uri == model)
            .expect("the saved file is republished too");
        assert!(
            for_model.diagnostics.is_empty(),
            "{:?}",
            for_model.diagnostics
        );
    }

    /// The hint is drawn at the binder and says the type the checker
    /// inferred, in the language's own `name is Type` spelling.
    #[test]
    fn inlay_hints_annotate_the_binders_of_the_open_document() {
        let project = Project::new("inlay-hints");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = "use \"./model\" for double\n\
                   state totals is client List of Whole starting empty\n\
                   view\n\
                   \x20   each total in totals\n\
                   \x20       Text (double with total)\n";
        let uri = project.open(&mut documents, "app.zd", app);

        let response = answer(
            &documents,
            request(
                lsp_types::request::InlayHintRequest::METHOD,
                lsp_types::InlayHintParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri },
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 4,
                            character: 0,
                        },
                    },
                    work_done_progress_params: Default::default(),
                },
            ),
        );
        let found: Vec<InlayHint> =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a list of hints");

        assert_eq!(found.len(), 1, "the `each` binder alone: {found:?}");
        let InlayHintLabel::String(label) = &found[0].label else {
            panic!("expected a plain label");
        };
        assert_eq!(label, "is Whole");
        assert_eq!(found[0].kind, Some(InlayHintKind::TYPE));
        // Drawn just after the binder's name, on the `each` line.
        assert_eq!(
            found[0].position,
            Position {
                line: 3,
                character: position(app, "total in").character + 5,
            }
        );
        // `double`'s own parameter lives in the imported file, and its
        // hint belongs in that file's window rather than in this one.
        assert!(
            found.iter().all(|hint| hint.position.line == 3),
            "{found:?}"
        );
    }

    /// Signature help for a function declared in another file, which is
    /// where the parameter names are least likely to be remembered.
    #[test]
    fn signature_help_names_the_parameters_of_an_imported_function() {
        let project = Project::new("signature-help");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = project.open(&mut documents, "app.zd", APP);

        let response = answer(
            &documents,
            request(
                lsp_types::request::SignatureHelpRequest::METHOD,
                lsp_types::SignatureHelpParams {
                    text_document_position_params: lsp_types::TextDocumentPositionParams {
                        text_document: lsp_types::TextDocumentIdentifier { uri: app },
                        position: Position {
                            line: 1,
                            character: position(APP, "double with 2").character
                                + "double with ".len() as u32,
                        },
                    },
                    context: None,
                    work_done_progress_params: Default::default(),
                },
            ),
        );
        let found: SignatureHelp =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("signature help");

        assert_eq!(found.signatures.len(), 1, "{found:?}");
        assert_eq!(found.signatures[0].label, "double with n is Whole");
        assert_eq!(found.active_parameter, Some(0));
    }

    /// The quick fix an editor can apply: a name declared in a file this
    /// one already reaches, but not among the names the `use` line
    /// borrowed. The repair is a fact about the module graph.
    #[test]
    fn a_name_a_reachable_file_declares_is_offered_as_an_import() {
        let project = Project::new("code-actions");
        project.write(
            "model.zd",
            "function double with n\n    give n + n\n\
             function triple with n\n    give n + n + n\n",
        );
        let mut documents = Documents::default();
        let app = "use \"./model\" for double\n\
                   state total is client Whole from triple with 2\n\
                   view\n    Text total\n";
        let uri = project.open(&mut documents, "app.zd", app);

        let analysis = documents.open.get(&uri).expect("open");
        assert_eq!(
            analysis.diagnostics().len(),
            1,
            "`triple` is declared but not imported: {:?}",
            analysis.diagnostics()
        );

        let at = position(app, "triple with 2");
        let response = answer(
            &documents,
            request(
                lsp_types::request::CodeActionRequest::METHOD,
                lsp_types::CodeActionParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                    range: Range { start: at, end: at },
                    context: Default::default(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let found: Vec<CodeActionOrCommand> =
            serde_json::from_value(response.response_result.expect("a result"))
                .expect("a list of code actions");

        assert_eq!(found.len(), 1, "{found:?}");
        let CodeActionOrCommand::CodeAction(action) = &found[0] else {
            panic!("expected an action rather than a command");
        };
        assert_eq!(action.title, "Import `triple` from \"./model\"");
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(
            action.diagnostics.as_ref().map(|found| found.len()),
            Some(1),
            "the fix is attached to the diagnostic it repairs"
        );

        // Applying it produces a file that resolves, which is the only
        // claim a quick fix makes.
        let edits = action
            .edit
            .as_ref()
            .and_then(|edit| edit.changes.as_ref())
            .and_then(|changes| changes.get(&uri))
            .expect("edits for this document");
        let fixed = apply(app, edits);
        assert_eq!(
            fixed,
            "use \"./model\" for double, triple\n\
             state total is client Whole from triple with 2\n\
             view\n    Text total\n"
        );
        assert!(
            Analysis::of_document(file_path(&uri).as_deref(), &fixed)
                .diagnostics()
                .is_empty(),
            "the repaired file must compile"
        );
    }

    /// The call hierarchy of a function declared in another file: its
    /// caller is in the entry document, and the item itself belongs to
    /// the file that declares it.
    #[test]
    fn a_call_hierarchy_crosses_the_module_boundary_in_both_directions() {
        let project = Project::new("call-hierarchy");
        project.write("model.zd", MODEL);
        let mut documents = Documents::default();
        let app = "use \"./model\" for double\n\
                   function quadruple with n\n    give double with (double with n)\n\
                   state four is client Whole from quadruple with 1\n\
                   view\n    Text four\n";
        let uri = project.open(&mut documents, "app.zd", app);

        let prepared = answer(
            &documents,
            request(
                lsp_types::request::CallHierarchyPrepare::METHOD,
                lsp_types::CallHierarchyPrepareParams {
                    text_document_position_params: document_position(&uri, app, "double with ("),
                    work_done_progress_params: Default::default(),
                },
            ),
        );
        let items: Vec<CallHierarchyItem> =
            serde_json::from_value(prepared.response_result.expect("a result"))
                .expect("hierarchy items");
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].name, "double");
        assert_eq!(
            items[0].uri,
            project.uri("model.zd"),
            "the item is the declaration, which is in the imported file"
        );

        let incoming = answer(
            &documents,
            request(
                lsp_types::request::CallHierarchyIncomingCalls::METHOD,
                lsp_types::CallHierarchyIncomingCallsParams {
                    item: items[0].clone(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let callers: Vec<CallHierarchyIncomingCall> =
            serde_json::from_value(incoming.response_result.expect("a result"))
                .expect("incoming calls");
        assert_eq!(callers.len(), 1, "{callers:?}");
        assert_eq!(callers[0].from.name, "quadruple");
        assert_eq!(
            callers[0].from_ranges.len(),
            2,
            "`quadruple` names `double` twice"
        );

        let outgoing = answer(
            &documents,
            request(
                lsp_types::request::CallHierarchyOutgoingCalls::METHOD,
                lsp_types::CallHierarchyOutgoingCallsParams {
                    item: callers[0].from.clone(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                },
            ),
        );
        let callees: Vec<CallHierarchyOutgoingCall> =
            serde_json::from_value(outgoing.response_result.expect("a result"))
                .expect("outgoing calls");
        assert_eq!(callees.len(), 1, "{callees:?}");
        assert_eq!(callees[0].to.name, "double");
        assert_eq!(callees[0].to.uri, project.uri("model.zd"));
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

    /// Apply a set of edits to a test file, so a test can assert on what
    /// the programmer would be left holding rather than on a count.
    ///
    /// Applied last first, so an earlier edit's range is still valid when
    /// it is reached.
    fn apply(text: &str, edits: &[lsp_types::TextEdit]) -> String {
        let lines = LineIndex::new(text);
        let mut ordered: Vec<&lsp_types::TextEdit> = edits.iter().collect();
        ordered.sort_by_key(|edit| (edit.range.start.line, edit.range.start.character));

        let mut out = text.to_string();
        for edit in ordered.into_iter().rev() {
            let at = |position: Position| {
                lines.offset(
                    text,
                    crate::lines::Position {
                        line: position.line,
                        character: position.character,
                    },
                ) as usize
            };
            out.replace_range(at(edit.range.start)..at(edit.range.end), &edit.new_text);
        }
        out
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
