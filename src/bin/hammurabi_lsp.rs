//! # hammurabi-lsp — Hammurabi Language Server
//!
//! `.hb` ファイルに対して以下の LSP 機能を提供する。
//!
//! | 機能              | 内容 |
//! |-------------------|------|
//! | **Diagnostics**   | 構文エラー・検証エラーをリアルタイムで表示 |
//! | **Hover**         | 述語・禁止パターンの説明を表示 |
//! | **Completion**    | キーワード・禁止パターンのオートコンプリート |
//!
//! ## 起動方法
//! ```bash
//! cargo build --bin hammurabi-lsp
//! # VSCode 等のエディタから stdio で呼び出す
//! ```

use std::collections::HashMap;

use hammurabi::compiler::verifier::{MockVerifier, Verifier};
use hammurabi::lsp::{ErrorSeverity, ItemKind, ParsedGoal, ParseResult, parse_hb};
use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionOptions,
    CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity,
    Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams,
    MarkupContent, MarkupKind,
    Position, PublishDiagnosticsParams, Range,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind,
    Uri,
    notification::{DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _},
    request::{Completion as CompletionRequest, HoverRequest, Request as _},
};

// ---------------------------------------------------------------------------
// エントリポイント
// ---------------------------------------------------------------------------

fn main() -> anyhow::Result<()> {
    eprintln!("[hammurabi-lsp] starting...");

    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![" ".into(), "\n".into()]),
            resolve_provider: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };

    let _init: InitializeParams = serde_json::from_value(
        connection.initialize(serde_json::to_value(server_capabilities)?)?
    )?;

    eprintln!("[hammurabi-lsp] initialized");
    main_loop(connection)?;
    io_threads.join()?;
    eprintln!("[hammurabi-lsp] shutdown");
    Ok(())
}

// ---------------------------------------------------------------------------
// メインループ
// ---------------------------------------------------------------------------

fn main_loop(conn: Connection) -> anyhow::Result<()> {
    // ファイル URI → 最新テキスト
    let mut docs: HashMap<Uri, String> = HashMap::new();

    for msg in &conn.receiver {
        match msg {
            // ── リクエスト ────────────────────────────────────────────────
            Message::Request(req) => {
                if conn.handle_shutdown(&req)? {
                    break;
                }
                if let Some(resp) = handle_request(req, &docs) {
                    conn.sender.send(Message::Response(resp))?;
                }
            }

            // ── 通知 ──────────────────────────────────────────────────────
            Message::Notification(notif) => {
                if let Some((uri, diags)) = handle_notification(notif, &mut docs) {
                    let params = PublishDiagnosticsParams {
                        uri,
                        diagnostics: diags,
                        version: None,
                    };
                    conn.sender.send(Message::Notification(Notification::new(
                        "textDocument/publishDiagnostics".into(),
                        params,
                    )))?;
                }
            }

            Message::Response(_) => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// リクエストハンドラ
// ---------------------------------------------------------------------------

fn handle_request(req: Request, docs: &HashMap<Uri, String>) -> Option<Response> {
    // Hover
    if req.method == HoverRequest::METHOD {
        let (id, params) = match req.extract::<HoverParams>(HoverRequest::METHOD) {
            Ok(v) => v,
            Err(_) => return None,
        };
        let uri  = &params.text_document_position_params.text_document.uri;
        let pos  = params.text_document_position_params.position;
        let text = docs.get(uri).map(String::as_str).unwrap_or("");

        let result = compute_hover(text, pos);
        return Some(Response::new_ok(id, result));
    }

    // Completion
    if req.method == CompletionRequest::METHOD {
        let (id, params) = match req.extract::<CompletionParams>(CompletionRequest::METHOD) {
            Ok(v) => v,
            Err(_) => return None,
        };
        let uri  = &params.text_document_position.text_document.uri;
        let pos  = params.text_document_position.position;
        let text = docs.get(uri).map(String::as_str).unwrap_or("");

        let result = compute_completion(text, pos);
        return Some(Response::new_ok(id, Some(result)));
    }

    None
}

// ---------------------------------------------------------------------------
// 通知ハンドラ（診断を返す）
// ---------------------------------------------------------------------------

fn handle_notification(
    notif: Notification,
    docs: &mut HashMap<Uri, String>,
) -> Option<(Uri, Vec<Diagnostic>)> {
    if notif.method == DidOpenTextDocument::METHOD {
        let params = serde_json::from_value::<
            lsp_types::DidOpenTextDocumentParams
        >(notif.params).ok()?;
        let uri  = params.text_document.uri;
        let text = params.text_document.text;
        let diag = validate(&text);
        docs.insert(uri.clone(), text);
        return Some((uri, diag));
    }

    if notif.method == DidChangeTextDocument::METHOD {
        let params = serde_json::from_value::<
            lsp_types::DidChangeTextDocumentParams
        >(notif.params).ok()?;
        let uri  = params.text_document.uri;
        let text = params.content_changes.into_iter().last()?.text;
        let diag = validate(&text);
        docs.insert(uri.clone(), text);
        return Some((uri, diag));
    }

    if notif.method == DidCloseTextDocument::METHOD {
        let params = serde_json::from_value::<
            lsp_types::DidCloseTextDocumentParams
        >(notif.params).ok()?;
        let uri = params.text_document.uri;
        docs.remove(&uri);
        // 閉じたファイルの診断を空にする
        return Some((uri, vec![]));
    }

    None
}

// ---------------------------------------------------------------------------
// 診断生成
// ---------------------------------------------------------------------------

fn validate(text: &str) -> Vec<Diagnostic> {
    let ParseResult { goals, errors, .. } = parse_hb(text);
    let mut diags: Vec<Diagnostic> = Vec::new();

    // パースエラー → Error
    for e in errors {
        diags.push(Diagnostic {
            range:    span_to_range(&hammurabi::lsp::Span::new(
                e.span.line, e.span.col_start, e.span.col_end,
            )),
            severity: Some(match e.severity {
                ErrorSeverity::Error   => DiagnosticSeverity::ERROR,
                ErrorSeverity::Warning => DiagnosticSeverity::WARNING,
            }),
            source:   Some("hammurabi".into()),
            message:  e.message,
            ..Default::default()
        });
    }

    // 各ゴールを MockVerifier で検証
    let verifier = MockVerifier::default();
    for ParsedGoal { goal, name_span, .. } in &goals {
        // well-formedness
        if !goal.is_well_formed() {
            diags.push(Diagnostic {
                range: span_to_range(name_span),
                severity: Some(DiagnosticSeverity::ERROR),
                source:   Some("hammurabi".into()),
                message:  format!(
                    "`{}`: 事後条件（ensure）が1つも定義されていません。\
                     ContractualGoal には最低1つの事後条件が必要です。",
                    goal.name
                ),
                ..Default::default()
            });
            continue;
        }

        match verifier.verify_goal(goal) {
            Ok(report) => {
                for violation in &report.violations {
                    diags.push(Diagnostic {
                        range:    span_to_range(name_span),
                        severity: Some(DiagnosticSeverity::WARNING),
                        source:   Some("hammurabi".into()),
                        message:  violation.clone(),
                        ..Default::default()
                    });
                }
            }
            Err(e) => {
                diags.push(Diagnostic {
                    range:    span_to_range(name_span),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source:   Some("hammurabi".into()),
                    message:  e.to_string(),
                    ..Default::default()
                });
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// ホバー
// ---------------------------------------------------------------------------

fn compute_hover(text: &str, pos: Position) -> Option<Hover> {
    let ParseResult { goals, .. } = parse_hb(text);

    for ParsedGoal {
        goal,
        name_span,
        items,
        context,
    } in &goals
    {
        // ゴール名をホバー → ContractualGoal のサマリを表示
        if span_contains(name_span, pos) {
            let mut md = format!(
                "## ContractualGoal: `{}`\n\n\
                 | 種別 | 数 |\n|------|----|\n\
                 | Preconditions  | {} |\n\
                 | Postconditions | {} |\n\
                 | Invariants     | {} |\n\
                 | Forbidden      | {} |\n",
                goal.name,
                goal.preconditions.len(),
                goal.postconditions.len(),
                goal.invariants.len(),
                goal.forbidden.len(),
            );
            if let Some(ctx) = context.as_ref().filter(|s| !s.trim().is_empty()) {
                md.push_str("\n### 設計コンテキスト（define）\n\n");
                md.push_str(ctx);
                md.push('\n');
            }
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind:  MarkupKind::Markdown,
                    value: md,
                }),
                range: Some(span_to_range(name_span)),
            });
        }

        // ステートメントをホバー
        for item in items {
            if span_contains(&item.span, pos) {
                let badge = match item.kind {
                    ItemKind::Precondition  => "🔵 **Precondition**",
                    ItemKind::Postcondition => "🟢 **Postcondition**",
                    ItemKind::Invariant     => "🟡 **Invariant**",
                    ItemKind::Forbidden     => "🔴 **Forbidden Pattern**",
                };
                let md = format!("{badge}\n\n`{}`", item.display);
                return Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind:  MarkupKind::Markdown,
                        value: md,
                    }),
                    range: Some(span_to_range(&item.span)),
                });
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// 補完
// ---------------------------------------------------------------------------

fn compute_completion(text: &str, pos: Position) -> CompletionResponse {
    let current_line = text.lines()
        .nth(pos.line as usize)
        .unwrap_or("")
        .trim_start();

    let items: Vec<CompletionItem> = if current_line.is_empty()
        || !current_line.contains(' ')
    {
        // 行頭 → キーワード補完
        keyword_completions()
    } else if current_line.starts_with("forbid") {
        // forbid の後 → 禁止パターン補完
        forbidden_completions()
    } else if current_line.starts_with("require")
        || current_line.starts_with("ensure")
        || current_line.starts_with("invariant")
    {
        // 述語補完
        predicate_completions()
    } else {
        vec![]
    };

    CompletionResponse::List(CompletionList {
        is_incomplete: false,
        items,
    })
}

fn keyword_completions() -> Vec<CompletionItem> {
    [
        ("goal",      "goal <function_name>",      "ContractualGoal の定義を開始する"),
        ("require",   "require <predicate>",        "事前条件（Precondition）を追加する"),
        ("ensure",    "ensure <predicate>",         "事後条件（Postcondition）を追加する"),
        ("invariant", "invariant <predicate>",      "不変条件（Invariant）を追加する"),
        ("forbid",    "forbid <ForbiddenPattern>",  "禁止パターンを追加する"),
    ]
    .into_iter()
    .map(|(label, detail, doc)| CompletionItem {
        label:            label.into(),
        kind:             Some(CompletionItemKind::KEYWORD),
        detail:           Some(detail.into()),
        documentation:    Some(lsp_types::Documentation::String(doc.into())),
        insert_text:      Some(label.into()),
        ..Default::default()
    })
    .collect()
}

fn forbidden_completions() -> Vec<CompletionItem> {
    [
        ("NonExhaustiveBranch",  "全分岐を型レベルで網羅することを要求"),
        ("RuntimeNullCheck",     "NonNull を型システムで証明することを要求"),
        ("ImplicitCoercion",     "暗黙の型強制を禁止"),
        ("UnprovenUnwrap",       "証明なしの unwrap/expect を禁止"),
        ("CatchAllSuppression",  "`_` パターンによるロジック隠蔽を禁止"),
    ]
    .into_iter()
    .map(|(label, doc)| CompletionItem {
        label:         label.into(),
        kind:          Some(CompletionItemKind::ENUM_MEMBER),
        documentation: Some(lsp_types::Documentation::String(doc.into())),
        insert_text:   Some(label.into()),
        ..Default::default()
    })
    .collect()
}

fn predicate_completions() -> Vec<CompletionItem> {
    [
        ("NonNull(${1:var})",              "NonNull — 参照が Null でないこと"),
        ("InRange(${1:var}, ${2:min}, ${3:max})", "InRange — 整数が範囲内にあること"),
        ("Not(${1:predicate})",            "Not — 述語の否定"),
        ("And(${1:pred1}, ${2:pred2})",    "And — 論理積"),
        ("Or(${1:pred1}, ${2:pred2})",     "Or  — 論理和"),
    ]
    .into_iter()
    .map(|(label, doc)| CompletionItem {
        label:             label.split('(').next().unwrap_or(label).into(),
        kind:              Some(CompletionItemKind::FUNCTION),
        documentation:     Some(lsp_types::Documentation::String(doc.into())),
        insert_text:       Some(label.into()),
        insert_text_format: Some(lsp_types::InsertTextFormat::SNIPPET),
        ..Default::default()
    })
    .collect()
}

// ---------------------------------------------------------------------------
// ユーティリティ
// ---------------------------------------------------------------------------

fn span_to_range(span: &hammurabi::lsp::Span) -> Range {
    Range {
        start: Position { line: span.line, character: span.col_start },
        end:   Position { line: span.line, character: span.col_end },
    }
}

fn span_contains(span: &hammurabi::lsp::Span, pos: Position) -> bool {
    pos.line == span.line
        && pos.character >= span.col_start
        && pos.character <= span.col_end
}
