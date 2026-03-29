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

#[derive(Debug)]
pub struct ParsedGoal {
    pub goal:      ContractualGoal,
    pub name_span: Span,
    pub items:     Vec<ParsedItem>,
    /// `define: { context: ... }` で与えた設計意図・査定用の文脈（行ベース goal では常に `None`）
    pub context:   Option<String>,
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
/// 行ベース構文に加え、ファイル内の最外殻 `{ ... }` ブロックを
/// [`brace::parse_brace_block`] で解釈し、得られた `goal` 群（複数 `define` 可）と `config` をマージする。
pub fn parse_hb(text: &str) -> ParseResult {
    let ranges = brace::find_outer_brace_ranges(text);
    let masked = brace::mask_brace_ranges(text, &ranges);
    let mut result = parse_hb_line_based(&masked);

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

fn parse_hb_line_based(text: &str) -> ParseResult {
    let mut goals:      Vec<ParsedGoal>    = Vec::new();
    let mut errors:     Vec<ParseError>    = Vec::new();
    let mut current:    Option<ParsedGoal> = None;
    let mut file_lang:  TargetLang         = TargetLang::Rust;
    let mut file_agent: Option<AgentKind>  = None;
    let mut file_api_key: Option<String>   = None;
    let mut file_model: Option<String>     = None;

    for (line_idx, raw_line) in text.lines().enumerate() {
        let line_no = line_idx as u32;
        let trimmed  = raw_line.trim();

        // 空行・コメント行はスキップ
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('#')
        {
            continue;
        }

        // キーワードと残りに分割
        let (keyword, rest) = split_keyword(trimmed);
        let kw_lower = keyword.to_lowercase();

        match kw_lower.as_str() {
            "lang" => {
                // ファイルレベルの言語指定: `lang python`
                let val = rest.split_whitespace().next().unwrap_or(rest);
                match val.parse::<TargetLang>() {
                    Ok(l)  => file_lang = l,
                    Err(e) => errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  format!("lang 指定エラー: {e}"),
                        severity: ErrorSeverity::Error,
                    }),
                }
            }

            "agent" => {
                // AI エージェント指定: `agent openai`
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
                // API キー指定: 3 通りの書き方をサポート
                //   api_key sk-proj-xxx          → 直書き（⚠ .gitignore 推奨）
                //   api_key $OPENAI_API_KEY      → 環境変数参照（.env も可）
                //   api_key $MY_CUSTOM_KEY       → 任意の環境変数名
                let val = rest.split_whitespace().next().unwrap_or(rest);
                if val.is_empty() {
                    errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  "api_key の後に値が必要です。\
                                   直書き: `api_key sk-proj-xxx`\
                                   変数参照: `api_key $OPENAI_API_KEY`".into(),
                        severity: ErrorSeverity::Error,
                    });
                } else if let Some(var_name) = val.strip_prefix('$') {
                    // ─── $ENV_VAR 形式 ─────────────────────────────────
                    // .env がロード済みであれば dotenvy 経由の値も std::env::var で取得できる
                    match std::env::var(var_name) {
                        Ok(resolved) if !resolved.is_empty() => {
                            file_api_key = Some(resolved);
                            // 解決成功: info 扱い（警告なし）
                        }
                        Ok(_) | Err(_) => {
                            errors.push(ParseError {
                                span:     Span::whole_line(line_no, trimmed.len() as u32),
                                message:  format!(
                                    "環境変数 `{var_name}` が未設定または空です。\
                                     .env ファイルに `{var_name}=sk-...` を追加するか、\
                                     `export {var_name}=sk-...` を実行してください。"
                                ),
                                severity: ErrorSeverity::Warning,
                            });
                        }
                    }
                } else {
                    // ─── 直書きキー ─────────────────────────────────────
                    file_api_key = Some(val.to_owned());
                    errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  "⚠ api_key を .hb ファイルに直書きしています。\
                                   `api_key $OPENAI_API_KEY` のように環境変数を参照することを推奨します。".into(),
                        severity: ErrorSeverity::Warning,
                    });
                }
            }

            "model" => {
                // モデル名指定: `model gpt-4o`
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

            "goal" => {
                // 前のゴールを確定
                if let Some(g) = current.take() { goals.push(g); }

                if rest.is_empty() {
                    errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  "`goal` の後に関数名が必要です（例: `goal safe_division`）".into(),
                        severity: ErrorSeverity::Error,
                    });
                } else {
                    let name = rest.split_whitespace().next().unwrap_or(rest);
                    let col   = col_of(raw_line, name);
                    current = Some(ParsedGoal {
                        goal:      ContractualGoal::new(name),
                        name_span: Span::new(line_no, col, col + name.len() as u32),
                        items:     Vec::new(),
                        context:   None,
                    });
                }
            }

            kw @ ("require" | "ensure" | "invariant" | "forbid") => {
                let Some(ref mut entry) = current else {
                    errors.push(ParseError {
                        span:     Span::whole_line(line_no, keyword.len() as u32),
                        message:  format!("`{kw}` の前に `goal <name>` が必要です"),
                        severity: ErrorSeverity::Error,
                    });
                    continue;
                };

                if rest.is_empty() {
                    errors.push(ParseError {
                        span:     Span::whole_line(line_no, trimmed.len() as u32),
                        message:  format!("`{kw}` の後に述語が必要です"),
                        severity: ErrorSeverity::Error,
                    });
                    continue;
                }

                let col   = col_of(raw_line, rest);
                let span  = Span::new(line_no, col, col + rest.len() as u32);

                match kw {
                    "require" => match parse_predicate(rest) {
                        Ok(pred) => {
                            let display = pred.to_string();
                            entry.items.push(ParsedItem {
                                span,
                                kind:    ItemKind::Precondition,
                                hover:   format!("**Precondition**\n\n`{display}`\n\n呼び出し元が保証する入力の性質。"),
                                display: display.clone(),
                            });
                            entry.goal = entry.goal.clone().require(pred);
                        }
                        Err(msg) => errors.push(ParseError { span, message: msg, severity: ErrorSeverity::Error }),
                    },

                    "ensure" => match parse_predicate(rest) {
                        Ok(pred) => {
                            let display = pred.to_string();
                            entry.items.push(ParsedItem {
                                span,
                                kind:    ItemKind::Postcondition,
                                hover:   format!("**Postcondition**\n\n`{display}`\n\nこの関数が保証しなければならない出力の性質。"),
                                display: display.clone(),
                            });
                            entry.goal = entry.goal.clone().ensure(pred);
                        }
                        Err(msg) => errors.push(ParseError { span, message: msg, severity: ErrorSeverity::Error }),
                    },

                    "invariant" => match parse_predicate(rest) {
                        Ok(pred) => {
                            let display = pred.to_string();
                            entry.items.push(ParsedItem {
                                span,
                                kind:    ItemKind::Invariant,
                                hover:   format!("**Invariant**\n\n`{display}`\n\n実行中ずっと成立しなければならない不変条件。"),
                                display: display.clone(),
                            });
                            entry.goal = entry.goal.clone().invariant(pred);
                        }
                        Err(msg) => errors.push(ParseError { span, message: msg, severity: ErrorSeverity::Error }),
                    },

                    "forbid" => match parse_forbidden(rest) {
                        Ok(fp) => {
                            let display = fp.to_string();
                            entry.items.push(ParsedItem {
                                span,
                                kind:    ItemKind::Forbidden,
                                hover:   format!("**Forbidden Pattern**\n\n`{display}`\n\n{}", forbidden_doc(&fp)),
                                display: display.clone(),
                            });
                            entry.goal = entry.goal.clone().forbid(fp);
                        }
                        Err(msg) => errors.push(ParseError { span, message: msg, severity: ErrorSeverity::Error }),
                    },

                    _ => unreachable!(),
                }
            }

            _ => {
                errors.push(ParseError {
                    span:    Span::new(line_no, 0, keyword.len() as u32),
                    message: format!(
                        "不明なキーワード: `{keyword}`\n\
                         ファイル設定: `agent`, `api_key`, `model`, `lang`\n\
                         ゴール定義: `goal`, `require`, `ensure`, `invariant`, `forbid`"
                    ),
                    severity: ErrorSeverity::Error,
                });
            }
        }
    }

    if let Some(g) = current { goals.push(g); }
    ParseResult {
        goals,
        errors,
        lang:    file_lang,
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
         使用例: `NonNull(var)`, `InRange(var, min, max)`, `atom_name`,\n\
         `Not(pred)`, `And(p1, p2)`, `Or(p1, p2)`, `When(cond, consequence)`"
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

fn forbidden_doc(fp: &ForbiddenPattern) -> &'static str {
    match fp {
        ForbiddenPattern::NonExhaustiveBranch => "全分岐を型レベルで網羅することを要求。",
        ForbiddenPattern::RuntimeNullCheck    => "NonNull を型システムで証明することを要求。",
        ForbiddenPattern::ImplicitCoercion    => "暗黙の型強制を禁止。",
        ForbiddenPattern::UnprovenUnwrap      => "証明なしの unwrap/expect を禁止。",
        ForbiddenPattern::CatchAllSuppression => "`_` パターンによるロジック隠蔽を禁止。",
    }
}

// ---------------------------------------------------------------------------
// ユーティリティ
// ---------------------------------------------------------------------------

/// 先頭のキーワードと残りに分割する。
/// キーワードと残りの値に分割する。
/// `key: value`、`key:value`、`key value` の 3 形式すべてを受け付ける。
fn split_keyword(s: &str) -> (&str, &str) {
    // `key: value` または `key:value` 形式（コロン区切り）を優先
    if let Some((k, v)) = s.split_once(':') {
        let kw   = k.trim();
        let rest = v.trim();
        // コロン後が空の場合（`goal:` のみの行）は空文字を返す
        return (kw, rest);
    }
    // 旧来の `key value` 形式（スペース区切り）にフォールバック
    let mut iter = s.splitn(2, |c: char| c.is_whitespace());
    let kw   = iter.next().unwrap_or("");
    let rest = iter.next().map(|r| r.trim()).unwrap_or("");
    (kw, rest)
}

/// `needle` が `line` 中で最初に登場する列（UTF-8 考慮）を返す。
fn col_of(line: &str, needle: &str) -> u32 {
    line.find(needle).unwrap_or(0) as u32
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
// safe_division の ContractualGoal

goal safe_division
require NonNull(divisor)
require InRange(divisor, 1, 9223372036854775807)
require dividend_is_integer
ensure  result_is_finite
ensure  result_within_i64_range
invariant no_memory_aliasing
forbid RuntimeNullCheck
forbid UnprovenUnwrap
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
        let src = "goal foo\nensure ok\n\ngoal bar\nensure done\n";
        let result = parse_hb(src);
        assert_eq!(result.goals.len(), 2);
    }

    #[test]
    fn unknown_keyword_produces_error() {
        let result = parse_hb("goal foo\nwhatever blah\nensure ok\n");
        assert!(result.errors.iter().any(|e| e.message.contains("不明なキーワード")));
    }

    #[test]
    fn statement_before_goal_produces_error() {
        let result = parse_hb("require NonNull(x)\n");
        assert!(result.errors.iter().any(|e| e.message.contains("goal <name>")));
    }

    #[test]
    fn invalid_in_range_min_gt_max() {
        let result = parse_hb("goal foo\nrequire InRange(x, 100, 1)\nensure ok\n");
        assert!(result.errors.iter().any(|e| e.message.contains("min")));
    }

    #[test]
    fn invalid_forbidden_pattern() {
        let result = parse_hb("goal foo\nensure ok\nforbid Bogus\n");
        assert!(result.errors.iter().any(|e| e.message.contains("不明な禁止パターン")));
    }

    #[test]
    fn name_span_line_is_correct() {
        let result = parse_hb(SAMPLE);
        // "goal safe_division" はサンプル中の 4 行目（0-indexed: 3）
        assert_eq!(result.goals[0].name_span.line, 3);
    }

    #[test]
    fn complex_predicate_and() {
        let result = parse_hb(
            "goal foo\nrequire And(NonNull(x), InRange(x, 0, 100))\nensure ok\n"
        );
        assert_eq!(result.errors.len(), 0, "{:?}", result.errors);
    }

    #[test]
    fn complex_predicate_not() {
        let result = parse_hb("goal foo\nrequire Not(NonNull(x))\nensure ok\n");
        assert_eq!(result.errors.len(), 0, "{:?}", result.errors);
    }
}
