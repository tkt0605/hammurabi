//! # lsp — Hammurabi Language Server Protocol 実装
//!
//! `.hb` ファイルフォーマットの検証・補完・ホバーを提供する。
//!
//! ## `.hb` ファイルフォーマット
//! ```text
//! // コメント
//! agent   openai                   // AI エージェント (openai|anthropic|mock)
//! api_key sk-proj-xxx              // API キー（⚠ .gitignore 推奨）
//! model   gpt-4o                   // モデル名
//! lang    rust                     // 出力言語
//!
//! goal safe_division
//! require NonNull(divisor)
//! require InRange(divisor, 1, 9223372036854775807)
//! ensure  result_is_finite
//! invariant no_memory_aliasing
//! forbid RuntimeNullCheck
//! forbid UnprovenUnwrap
//! ```
//!
//! ファイル先頭の `agent` / `api_key` / `model` / `lang` はファイル全体に適用される。
//! 1 ファイルに複数の `goal` ブロックを記述できる。
//!
//! ## ブロック構文（v2）
//! 最外殻の `{ config, define { context?, goal, settings }, ... }` を同一ファイルに混在可能。
//! `define` 内の `context` は設計・査定の意図（任意）。`[]` や `-` 箇条書きで並列意図を書きやすい。詳細は [`brace`] モジュール。

mod brace;

use crate::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};
use crate::codegen::TargetLang;
use crate::config::AgentKind;

// ---------------------------------------------------------------------------
// Span — LSP のゼロ起算行・列
// ---------------------------------------------------------------------------

/// LSP 座標（行・列ともに 0-indexed）。
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub line:      u32,
    pub col_start: u32,
    pub col_end:   u32,
}

impl Span {
    pub fn new(line: u32, col_start: u32, col_end: u32) -> Self {
        Self { line, col_start, col_end }
    }
    /// 行全体を示すスパン（列 0 から行末まで）
    pub fn whole_line(line: u32, len: u32) -> Self {
        Self { line, col_start: 0, col_end: len }
    }
}

// ---------------------------------------------------------------------------
// ParsedItem — ホバー情報付きの 1 ステートメント
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Precondition,
    Postcondition,
    Invariant,
    Forbidden,
}

#[derive(Debug, Clone)]
pub struct ParsedItem {
    pub span:    Span,
    pub kind:    ItemKind,
    /// Markdown 形式のホバーテキスト
    pub hover:   String,
    /// 述語の表示文字列
    pub display: String,
}

// ---------------------------------------------------------------------------
// ParsedGoal — パース済み 1 ゴール
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ParsedGoal {
    pub goal:      ContractualGoal,
    pub name_span: Span,
    pub items:     Vec<ParsedItem>,
    /// `define: { intent: """...""" }` で与えた設計意図（行ベース goal では常に `None`）
    pub intent:    Option<String>,
    /// `goal:` に指定した自然言語ラベル（`goal: name "日本語ラベル"` または `goal: "ラベルのみ"` で設定）
    pub label:     Option<String>,
    /// `goal: "自然言語"` のみで `settings:` が省略されている場合 `true`。
    /// `hb gen` 実行時に AI が自動的に制約（require/ensure）を生成する。
    pub needs_ai:  bool,
    /// `id:` で指定した一意識別子（依存グラフのノードキー）
    pub id:        Option<String>,
    /// `model:` で指定した AI モデルバージョン固定（再現性の基盤）
    pub model_pin: Option<String>,
    /// `depends_on: [id1, id2]` で指定した依存ゴールの ID リスト
    pub depends_on: Vec<String>,
}

// ---------------------------------------------------------------------------
// ParseError
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub span:     Span,
    pub message:  String,
    pub severity: ErrorSeverity,
}

// ---------------------------------------------------------------------------
// ParseResult
// ---------------------------------------------------------------------------

pub struct ParseResult {
    pub goals:  Vec<ParsedGoal>,
    pub errors: Vec<ParseError>,
    /// `lang <language>` で指定された出力言語（省略時は Rust）
    pub lang:    TargetLang,
    /// `.hb` 内で `lang` が一度でも現れたか（ブロック外または `config:` 内）。false のときは `lang` はデフォルト Rust のみで、config.hb の言語を上書きしない。
    pub lang_specified: bool,
    /// `agent <name>` で指定された AI エージェント（省略時は None）
    pub agent:   Option<AgentKind>,
    /// `api_key <key>` で指定された API キー（省略時は None）
    ///
    /// ⚠ API キーをリポジトリに含めないよう `.gitignore` に `.hb` ファイルを追加してください。
    pub api_key: Option<String>,
    /// `model <name>` で指定されたモデル名（省略時は None）
    pub model:   Option<String>,
}

// ---------------------------------------------------------------------------
// parse_hb — エントリポイント
// ---------------------------------------------------------------------------

/// `.hb` テキストを解析し、`ParseResult` を返す。
/// エラーが含まれていても可能な限りパースを継続する（寛容なパーサ）。
///
/// goal 定義は `{ define: { ... } }` ブロック形式のみサポート。
/// ブロック外に書けるのはファイルレベルの設定行（`agent`, `model`, `lang`, `api_key`）のみ。
pub fn parse_hb(text: &str) -> ParseResult {
    let ranges = brace::find_outer_brace_ranges(text);
    let masked = brace::mask_brace_ranges(text, &ranges);
    let mut result = parse_file_config(&masked);

    for range in ranges {
        match brace::parse_brace_block(text, range) {
            Ok((meta, goals)) => {
                brace::merge_brace_meta_into_result(&mut result, meta);
                result.goals.extend(goals);
            }
            Err(mut errs) => result.errors.append(&mut errs),
        }
    }

    result
}

/// ブロック外のテキストからファイルレベルの設定（agent / model / lang / api_key）のみを読む。
/// goal 定義は `{ define: { ... } }` ブロック形式専用になったため、ここでは扱わない。
fn parse_file_config(text: &str) -> ParseResult {
    let mut errors:       Vec<ParseError>  = Vec::new();
    let mut file_lang:    TargetLang       = TargetLang::Rust;
    let mut lang_specified: bool           = false;
    let mut file_agent:   Option<AgentKind> = None;
    let mut file_api_key: Option<String>   = None;
    let mut file_model:   Option<String>   = None;

    for (line_idx, raw_line) in text.lines().enumerate() {
        let line_no = line_idx as u32;
        let trimmed  = raw_line.trim();

        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }

        let (keyword, rest) = split_keyword(trimmed);

        match keyword.to_lowercase().as_str() {
            "lang" => {
                let val = rest.split_whitespace().next().unwrap_or(rest);
                match val.parse::<TargetLang>() {
                    Ok(l)  => {
                        file_lang = l;
                        lang_specified = true;
                    }
                    Err(e) => errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  format!("lang 指定エラー: {e}"),
                        severity: ErrorSeverity::Error,
                    }),
                }
            }
            "agent" => {
                let val = rest.split_whitespace().next().unwrap_or(rest);
                match val.parse::<AgentKind>() {
                    Ok(a)  => file_agent = Some(a),
                    Err(e) => errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  format!("agent 指定エラー: {e}"),
                        severity: ErrorSeverity::Error,
                    }),
                }
            }
            "api_key" | "apikey" | "api-key" => {
                let val = rest.split_whitespace().next().unwrap_or(rest);
                if val.is_empty() {
                    errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  "api_key の後に値が必要です（例: `api_key $OPENAI_API_KEY`）".into(),
                        severity: ErrorSeverity::Error,
                    });
                } else if let Some(var_name) = val.strip_prefix('$') {
                    match std::env::var(var_name) {
                        Ok(resolved) if !resolved.is_empty() => file_api_key = Some(resolved),
                        _ => errors.push(ParseError {
                            span:     Span::whole_line(line_no, trimmed.len() as u32),
                            message:  format!(
                                "環境変数 `{var_name}` が未設定または空です。\
                                 .env ファイルに `{var_name}=sk-...` を追加してください。"
                            ),
                            severity: ErrorSeverity::Warning,
                        }),
                    }
                } else {
                    file_api_key = Some(val.to_owned());
                    errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  "⚠ api_key を直書きしています。`$ENV_VAR` 形式の環境変数参照を推奨します。".into(),
                        severity: ErrorSeverity::Warning,
                    });
                }
            }
            "model" => {
                let val = rest.split_whitespace().next().unwrap_or(rest);
                if val.is_empty() {
                    errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  "model の後にモデル名が必要です（例: `model gpt-4o`）".into(),
                        severity: ErrorSeverity::Error,
                    });
                } else {
                    file_model = Some(val.to_owned());
                }
            }
            other => {
                // goal などのキーワードはブロック形式のみ有効
                errors.push(ParseError {
                    span:     Span::new(line_no, 0, other.len() as u32),
                    message:  format!(
                        "ブロック外に `{other}` は書けません。\n\
                         goal の定義は `{{ define: {{ ... }} }}` ブロック形式で記述してください。\n\
                         ファイルレベルの設定: `agent`, `model`, `lang`, `api_key`"
                    ),
                    severity: ErrorSeverity::Error,
                });
            }
        }
    }

    ParseResult {
        goals:   Vec::new(),
        errors,
        lang:    file_lang,
        lang_specified,
        agent:   file_agent,
        api_key: file_api_key,
        model:   file_model,
    }
}

// ---------------------------------------------------------------------------
// 述語パーサ
// ---------------------------------------------------------------------------

pub(crate) fn parse_predicate(s: &str) -> Result<Predicate, String> {
    let s = s.trim();

    if s.starts_with("NonNull(") && s.ends_with(')') {
        let inner = &s["NonNull(".len()..s.len() - 1];
        let var = inner.trim();
        if var.is_empty() {
            return Err("NonNull の変数名が空です（例: `NonNull(divisor)`）".into());
        }
        return Ok(Predicate::non_null(var));
    }

    if s.starts_with("InRange(") && s.ends_with(')') {
        let inner  = &s["InRange(".len()..s.len() - 1];
        let parts: Vec<&str> = inner.splitn(3, ',').collect();
        if parts.len() != 3 {
            return Err("InRange は 3 引数が必要です（例: `InRange(divisor, 1, 100)`）".into());
        }
        let var = parts[0].trim();
        let min = parts[1].trim().parse::<i64>()
            .map_err(|_| format!("`{}` は整数ではありません", parts[1].trim()))?;
        let max = parts[2].trim().parse::<i64>()
            .map_err(|_| format!("`{}` は整数ではありません", parts[2].trim()))?;
        if min > max {
            return Err(format!("min({min}) が max({max}) より大きいです"));
        }
        return Ok(Predicate::in_range(var, min, max));
    }

    if s.starts_with("Not(") && s.ends_with(')') {
        let inner = &s["Not(".len()..s.len() - 1];
        return parse_predicate(inner).map(Predicate::not);
    }

    if s.starts_with("And(") && s.ends_with(')') {
        let inner = &s["And(".len()..s.len() - 1];
        if let Some(mid) = find_top_level_comma(inner) {
            let l = parse_predicate(&inner[..mid])?;
            let r = parse_predicate(&inner[mid + 1..])?;
            return Ok(Predicate::and(l, r));
        }
        return Err("And( の引数を2つに分割できませんでした".into());
    }

    if s.starts_with("Or(") && s.ends_with(')') {
        let inner = &s["Or(".len()..s.len() - 1];
        if let Some(mid) = find_top_level_comma(inner) {
            let l = parse_predicate(&inner[..mid])?;
            let r = parse_predicate(&inner[mid + 1..])?;
            return Ok(Predicate::or(l, r));
        }
        return Err("Or( の引数を2つに分割できませんでした".into());
    }
    // Equals(a, b) — 等値制約
    if s.starts_with("Equals(") && s.ends_with(')') {
        let inner = &s["Equals(".len()..s.len() - 1];
        if let Some(mid) = find_top_level_comma(inner) {
            let a = inner[..mid].trim();
            let b = inner[mid + 1..].trim();
            if a.is_empty() || b.is_empty() {
                return Err("Equals の引数が空です（例: `Equals(x, 0)` または `Equals(a, b)`）".into());
            }
            return Ok(Predicate::Equals(a.to_owned(), b.to_owned()));
        }
        return Err("Equals には 2 つの引数が必要です（例: `Equals(divisor, 0)`）".into());
    }

    // Regex(var, "pattern") — 文字列正規表現制約
    if s.starts_with("Regex(") && s.ends_with(')') {
        let inner = &s["Regex(".len()..s.len() - 1];
        // パターン内にカンマが含まれる可能性があるため、最初のカンマだけで分割する
        if let Some(comma_pos) = inner.find(',') {
            let var = inner[..comma_pos].trim();
            let pattern_raw = inner[comma_pos + 1..].trim();
            // ダブルクォートを除去
            let pattern = if pattern_raw.starts_with('"') && pattern_raw.ends_with('"') {
                &pattern_raw[1..pattern_raw.len() - 1]
            } else {
                pattern_raw
            };
            if var.is_empty() {
                return Err("Regex の変数名が空です（例: `Regex(email, \"^[a-z]+$\")`）".into());
            }
            if pattern.is_empty() {
                return Err("Regex のパターンが空です（例: `Regex(email, \"^[a-z]+$\")`）".into());
            }
            // パーサ段階でパターンの構文チェックを行う
            if let Err(e) = regex::Regex::new(pattern) {
                return Err(format!("Regex: 無効な正規表現パターン {pattern:?}: {e}"));
            }
            return Ok(Predicate::regex(var, pattern));
        }
        return Err(
            "Regex には 2 つの引数が必要です（例: `Regex(email, \"^[a-z]+$\")`）".into()
        );
    }

    // When(cond, consequence) — Implies のシンタックスシュガー
    if s.starts_with("When(") && s.ends_with(')') {
        let inner = &s["When(".len()..s.len() - 1];
        if let Some(mid) = find_top_level_comma(inner) {
            let cond = parse_predicate(&inner[..mid])?;
            let cons = parse_predicate(&inner[mid + 1..])?;
            return Ok(Predicate::implies(cond, cons));
        }
        return Err("When( には 2 つの引数が必要です（例: `When(InRange(x, 1, 100), result_ok)`）".into());
    }

    // ベアアトム: 英数字 + アンダースコアのみ
    if !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Ok(Predicate::atom(s));
    }

    Err(format!(
        "述語 `{s}` を解析できません\n\
         使用例: `NonNull(var)`, `InRange(var, min, max)`, `Regex(var, \"pattern\")`,\n\
         `Equals(a, b)`, `Not(pred)`, `And(p1, p2)`, `Or(p1, p2)`, `When(cond, consequence)`"
    ))
}

/// トップレベルのカンマ位置を探す（括弧のネストを考慮）
fn find_top_level_comma(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

pub(crate) fn parse_forbidden(s: &str) -> Result<ForbiddenPattern, String> {
    match s.trim() {
        "NonExhaustiveBranch" => Ok(ForbiddenPattern::NonExhaustiveBranch),
        "RuntimeNullCheck"    => Ok(ForbiddenPattern::RuntimeNullCheck),
        "ImplicitCoercion"    => Ok(ForbiddenPattern::ImplicitCoercion),
        "UnprovenUnwrap"      => Ok(ForbiddenPattern::UnprovenUnwrap),
        "CatchAllSuppression" => Ok(ForbiddenPattern::CatchAllSuppression),
        other => Err(format!(
            "不明な禁止パターン: `{other}`\n\
             使用可能: `NonExhaustiveBranch`, `RuntimeNullCheck`, \
             `ImplicitCoercion`, `UnprovenUnwrap`, `CatchAllSuppression`"
        )),
    }
}

// ---------------------------------------------------------------------------
// ユーティリティ
// ---------------------------------------------------------------------------

/// `key: value`、`key:value`、`key value` の 3 形式からキーワードと値を分割する。
fn split_keyword(s: &str) -> (&str, &str) {
    if let Some((k, v)) = s.split_once(':') {
        return (k.trim(), v.trim());
    }
    let mut iter = s.splitn(2, |c: char| c.is_whitespace());
    let kw   = iter.next().unwrap_or("");
    let rest = iter.next().map(|r| r.trim()).unwrap_or("");
    (kw, rest)
}

/// `goal:` の右辺を `(identifier, label)` に分解する。
///
/// 受け入れる形式:
/// - `my_func`                   → ("my_func",    None)
/// - `my_func "自然言語ラベル"`  → ("my_func",    Some("自然言語ラベル"))
/// - `"日本語の目標説明"`         → (auto-slug,    Some("日本語の目標説明"))
pub(crate) fn parse_goal_rhs(rhs: &str) -> Result<(String, Option<String>), String> {
    let rhs = rhs.trim().trim_end_matches(',').trim();
    if rhs.starts_with('"') {
        let label = extract_quoted(rhs)?;
        let slug = label_to_slug(&label);
        if slug.is_empty() {
            return Err(
                "goal: の引用符ラベルから識別子を生成できませんでした（空文字または記号のみ）".into()
            );
        }
        return Ok((slug, Some(label)));
    }
    let (ident_part, rest) = {
        let mut it = rhs.splitn(2, |c: char| c.is_whitespace());
        let a = it.next().unwrap_or("").to_owned();
        let b = it.next().unwrap_or("").trim().to_owned();
        (a, b)
    };
    let ident_part = ident_part.trim_end_matches(',').to_owned();
    validate_identifier(&ident_part)?;
    let label = if rest.starts_with('"') {
        Some(extract_quoted(&rest)?)
    } else if !rest.is_empty() {
        return Err(format!(
            "goal: `{ident_part}` の後に引用符 `\"` 以外のトークン `{rest}` があります"
        ));
    } else {
        None
    };
    Ok((ident_part, label))
}

fn validate_identifier(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("`goal:` の後に名前が必要です".into());
    }
    if name.contains(|c: char| !(c.is_alphanumeric() || c == '_')) {
        return Err(format!(
            "goal 名 `{name}` に使えない文字があります（英数字と `_` のみ）。\
             自然言語ラベルは引用符で囲んでください（例: `goal: \"ゼロ除算を防ぐ\"`）"
        ));
    }
    Ok(())
}

fn extract_quoted(s: &str) -> Result<String, String> {
    let s = s.trim();
    let body = s.strip_prefix('"').ok_or("引用符 `\"` で始まっていません")?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Ok(out),
            '\\' => match chars.next() {
                Some('"')  => out.push('"'),
                Some('n')  => out.push('\n'),
                Some('t')  => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some(c)    => out.push(c),
                None => return Err("文字列の終端が `\\` で終わっています".into()),
            },
            c => out.push(c),
        }
    }
    Err("文字列の閉じ引用符 `\"` がありません".into())
}

/// 自然言語ラベルを snake_case 識別子に変換する。
///
/// - ASCII 英数字はそのまま小文字化して使う。
/// - 非 ASCII 文字（日本語など）は Unicode スカラー値を `u{XXXX}` で表す（最大 4 語まで）。
/// - 空白・記号は単語区切り `_` として扱う。
// ---------------------------------------------------------------------------
// inputs: / output: パーサ
// ---------------------------------------------------------------------------

/// `a: i32, b: i32` → `Vec<Param>` に変換。
/// 型文字列内の `<>`, `()`, `[]` を考慮してカンマで分割する。
pub(crate) fn parse_inputs(rhs: &str) -> Result<Vec<crate::lang::goal::Param>, String> {
    let rhs = rhs.trim();
    if rhs.is_empty() {
        return Err("`inputs:` の後にパラメータが必要です（例: `inputs: n: i32, m: i32`）".into());
    }
    let items = split_top_level_comma(rhs);
    let mut params = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() { continue; }
        // 最初の `:` でパラメータ名と型に分割（型の中に `:` が来ない前提）
        let colon = item.find(':').ok_or_else(|| {
            format!("`{item}` は `name: Type` 形式ではありません（例: `n: i32`）")
        })?;
        let name     = item[..colon].trim().to_owned();
        let type_str = item[colon + 1..].trim().to_owned();
        if name.is_empty() {
            return Err(format!("`{item}`: パラメータ名が空です"));
        }
        if type_str.is_empty() {
            return Err(format!("`{item}`: 型が空です（例: `{name}: i32`）"));
        }
        params.push(crate::lang::goal::Param::new(name, type_str));
    }
    if params.is_empty() {
        return Err("`inputs:` にパラメータが 1 つもありません".into());
    }
    Ok(params)
}

/// `<>`, `()`, `[]` のネストを考慮してトップレベルの `,` で分割する。
pub(crate) fn split_top_level_comma(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth  = 0i32;
    let mut start  = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

// ---------------------------------------------------------------------------
// examples: パーサ
// ---------------------------------------------------------------------------

/// `examples:` ブロック（`[...]` 内または単行）をパースして `Vec<Example>` を返す。
///
/// 受け付けるフォーマット（`-` 先頭のリスト形式）:
/// ```text
/// - (10, 2)                  => Ok(5)
/// - 10, 2                    => Ok(5)
/// - "ラベル": (10, 2)        => Ok(5)
/// - dividend: 10, divisor: 2 => Ok(5)
/// ```
pub(crate) fn parse_examples(lines: &[(u32, String)]) -> Result<Vec<crate::lang::goal::Example>, String> {
    let mut examples = Vec::new();
    for (_, line) in lines {
        let cleaned = line.trim();
        // `- ` で始まる行のみ処理、空行・コメントはスキップ
        let rest = if let Some(r) = cleaned.strip_prefix('-') {
            r.trim()
        } else if cleaned.is_empty() || cleaned.starts_with("//") || cleaned.starts_with('#') {
            continue;
        } else {
            // `-` なしでも example 行として扱う
            cleaned
        };
        if rest.is_empty() { continue; }
        if let Some(ex) = parse_example_line(rest)? {
            examples.push(ex);
        }
    }
    Ok(examples)
}

/// `[label: ] inputs => output` を 1 行パース。
/// `=>` が見つからない場合は `Ok(None)` を返す（無視）。
pub(crate) fn parse_example_line(s: &str) -> Result<Option<crate::lang::goal::Example>, String> {
    let s = s.trim();
    if s.is_empty() { return Ok(None); }

    // トップレベルの `=>` を探す
    let arrow = find_top_level_arrow(s).ok_or_else(|| {
        format!("example `{s}` に `=>` がありません（例: `(10, 2) => Ok(5)`）")
    })?;

    let lhs = s[..arrow].trim();
    let rhs = s[arrow + 2..].trim();

    if rhs.is_empty() {
        return Err(format!("example `{s}` の `=>` の右辺が空です"));
    }

    // ラベルを検出: `"..."` で始まる場合
    let (label, inputs_raw) = if lhs.starts_with('"') {
        // `"ラベル": inputs`
        let end_quote = lhs[1..].find('"').map(|i| i + 1);
        if let Some(eq) = end_quote {
            let label_str = lhs[1..eq].to_owned();
            let after = lhs[eq + 1..].trim_start().strip_prefix(':')
                .map(|s| s.trim())
                .unwrap_or(lhs[eq + 1..].trim());
            (Some(label_str), after.to_owned())
        } else {
            (None, lhs.to_owned())
        }
    } else {
        (None, lhs.to_owned())
    };

    // 外側の括弧 `(...)` を剥がす
    let inputs_clean = inputs_raw.trim();
    let inputs_final = if inputs_clean.starts_with('(') && inputs_clean.ends_with(')') {
        inputs_clean[1..inputs_clean.len() - 1].trim().to_owned()
    } else {
        inputs_clean.to_owned()
    };

    Ok(Some(crate::lang::goal::Example::new(label, inputs_final, rhs)))
}

/// `=>` のトップレベルインデックスを返す（`<>`, `()`, `[]`, `{}` のネストを無視）。
fn find_top_level_arrow(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' | b'(' | b'[' | b'{' => depth += 1,
            b'>' | b')' | b']' | b'}' => depth -= 1,
            b'"' => {
                // 文字列リテラルをスキップ
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' { i += 1; }
                    i += 1;
                }
            }
            b'=' if depth == 0 && bytes.get(i + 1) == Some(&b'>') => {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

pub(crate) fn label_to_slug(label: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut non_ascii_count = 0u32;

    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            cur.push(ch.to_ascii_lowercase());
        } else if ch.is_alphanumeric() {
            // 非 ASCII 英数字（ひらがな・漢字等）
            if !cur.is_empty() {
                parts.push(cur.clone());
                cur.clear();
            }
            non_ascii_count += 1;
            if non_ascii_count <= 4 {
                parts.push(format!("u{:04x}", ch as u32));
            }
        } else {
            // 空白・記号 → 区切り
            if !cur.is_empty() {
                parts.push(cur.clone());
                cur.clear();
            }
        }
    }
    if !cur.is_empty() { parts.push(cur); }

    let slug = parts.join("_");
    let slug = slug.trim_matches('_').to_owned();
    if slug.len() > 64 { slug[..64].to_owned() } else { slug }
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ブロック形式のサンプル。goal: safe_division は 0-indexed で 3 行目に位置する。
    const SAMPLE: &str = r#"
{
  define: {
    goal: safe_division
    settings: [
      require: NonNull(divisor)
      require: InRange(divisor, 1, 9223372036854775807)
      require: dividend_is_integer
      ensure:  result_is_finite
      ensure:  result_within_i64_range
      invariant: no_memory_aliasing
      forbid: RuntimeNullCheck
      forbid: UnprovenUnwrap
    ]
  }
}
"#;

    #[test]
    fn parses_goal_name() {
        let result = parse_hb(SAMPLE);
        assert_eq!(result.errors.len(), 0, "errors: {:?}", result.errors);
        assert_eq!(result.goals.len(), 1);
        assert_eq!(result.goals[0].goal.name, "safe_division");
    }

    #[test]
    fn parses_preconditions() {
        let result = parse_hb(SAMPLE);
        let pres: Vec<_> = result.goals[0].items.iter()
            .filter(|i| i.kind == ItemKind::Precondition)
            .collect();
        assert_eq!(pres.len(), 3);
    }

    #[test]
    fn parses_postconditions() {
        let result = parse_hb(SAMPLE);
        let posts: Vec<_> = result.goals[0].items.iter()
            .filter(|i| i.kind == ItemKind::Postcondition)
            .collect();
        assert_eq!(posts.len(), 2);
    }

    #[test]
    fn parses_forbidden_patterns() {
        let result = parse_hb(SAMPLE);
        let fbs: Vec<_> = result.goals[0].items.iter()
            .filter(|i| i.kind == ItemKind::Forbidden)
            .collect();
        assert_eq!(fbs.len(), 2);
    }

    #[test]
    fn multiple_goals_in_one_file() {
        let src = r#"
{
  define: {
    goal: foo
    settings: [
      ensure: ok
    ]
  }
  define: {
    goal: bar
    settings: [
      ensure: done
    ]
  }
}
"#;
        let result = parse_hb(src);
        assert_eq!(result.goals.len(), 2);
    }

    #[test]
    fn unknown_keyword_produces_error() {
        // settings ブロック内の不明キーはエラーになる
        let src = "{\n  define: {\n    goal: foo\n    settings: [\n      whatever: blah\n      ensure: ok\n    ]\n  }\n}";
        let result = parse_hb(src);
        assert!(
            result.errors.iter().any(|e| e.message.contains("不明キー")),
            "エラーが見つかりません: {:?}", result.errors
        );
    }

    #[test]
    fn statement_before_goal_produces_error() {
        // ブロック外にキーワードを書くとエラー
        let result = parse_hb("require NonNull(x)\n");
        assert!(
            result.errors.iter().any(|e| e.message.contains("ブロック外")),
            "エラーが見つかりません: {:?}", result.errors
        );
    }

    #[test]
    fn invalid_in_range_min_gt_max() {
        let src = "{\n  define: {\n    goal: foo\n    settings: [\n      require: InRange(x, 100, 1)\n      ensure: ok\n    ]\n  }\n}";
        let result = parse_hb(src);
        assert!(
            result.errors.iter().any(|e| e.message.contains("min")),
            "エラーが見つかりません: {:?}", result.errors
        );
    }

    #[test]
    fn invalid_forbidden_pattern() {
        let src = "{\n  define: {\n    goal: foo\n    settings: [\n      ensure: ok\n      forbid: Bogus\n    ]\n  }\n}";
        let result = parse_hb(src);
        assert!(
            result.errors.iter().any(|e| e.message.contains("不明な禁止パターン")),
            "エラーが見つかりません: {:?}", result.errors
        );
    }

    #[test]
    fn name_span_line_is_correct() {
        let result = parse_hb(SAMPLE);
        // SAMPLE は r#"\n{\n  define: {\n    goal: safe_division\n..." → `goal:` は 0-indexed 3 行目
        assert_eq!(result.goals[0].name_span.line, 3);
    }

    #[test]
    fn complex_predicate_and() {
        let src = "{\n  define: {\n    goal: foo\n    settings: [\n      require: And(NonNull(x), InRange(x, 0, 100))\n      ensure: ok\n    ]\n  }\n}";
        let result = parse_hb(src);
        assert_eq!(result.errors.len(), 0, "{:?}", result.errors);
    }

    #[test]
    fn complex_predicate_not() {
        let src = "{\n  define: {\n    goal: foo\n    settings: [\n      require: Not(NonNull(x))\n      ensure: ok\n    ]\n  }\n}";
        let result = parse_hb(src);
        assert_eq!(result.errors.len(), 0, "{:?}", result.errors);
    }

    #[test]
    fn parse_goal_rhs_identifier_only() {
        let (name, label) = parse_goal_rhs("safe_division").unwrap();
        assert_eq!(name, "safe_division");
        assert!(label.is_none());
    }

    #[test]
    fn parse_goal_rhs_identifier_with_label() {
        let (name, label) = parse_goal_rhs(r#"safe_div "ゼロ除算を防ぐ""#).unwrap();
        assert_eq!(name, "safe_div");
        assert_eq!(label.as_deref(), Some("ゼロ除算を防ぐ"));
    }

    #[test]
    fn parse_goal_rhs_pure_label() {
        let (name, label) = parse_goal_rhs(r#""境界値チェックの実装""#).unwrap();
        assert!(!name.is_empty(), "スラッグが空");
        assert_eq!(label.as_deref(), Some("境界値チェックの実装"));
    }

    #[test]
    fn block_format_goal_accepts_label() {
        let src = r#"
{
  define: {
    goal: safe_div "ゼロ除算を防ぐ"
    settings: [
      ensure: n_ok
    ]
  }
}
"#;
        let result = parse_hb(src);
        assert_eq!(result.errors.len(), 0, "{:?}", result.errors);
        assert_eq!(result.goals[0].goal.name, "safe_div");
        assert_eq!(result.goals[0].label.as_deref(), Some("ゼロ除算を防ぐ"));
    }

    #[test]
    fn block_format_goal_pure_quoted() {
        let src = r#"
{
  define: {
    goal: "自然言語のゴール名"
    settings: [
      ensure: done
    ]
  }
}
"#;
        let result = parse_hb(src);
        assert_eq!(result.errors.len(), 0, "{:?}", result.errors);
        let g = &result.goals[0];
        assert!(!g.goal.name.is_empty());
        assert_eq!(g.label.as_deref(), Some("自然言語のゴール名"));
    }
}
