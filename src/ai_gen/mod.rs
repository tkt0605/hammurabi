//! # ai_gen — AI エージェントによる ContractualGoal 自動生成
//!
//! 自然言語の関数説明から `ContractualGoal` を自動生成する。
//!
//! ## バックエンド
//! | 型 | 必要なもの | 説明 |
//! |----|-----------|------|
//! | `MockAiGenerator`      | なし           | キーワード解析、API 不要（開発・テスト用） |
//! | `OpenAiGenerator`      | `ai` feature + API キー | GPT-4o による生成 |
//! | `AnthropicGenerator`   | `ai` feature + API キー | Claude による生成 |
//!
//! ## 生成の流れ
//! ```text
//! 自然言語の説明
//!   ↓ AiGoalGenerator::generate()
//! .hb 形式テキスト（AI が出力 or Mock が構築）
//!   ↓ parse_hb()（既存の LSP パーサを再利用）
//! Vec<ContractualGoal>
//!   ↓ MockVerifier::verify_goal()
//! AiGenOutput { goals, raw_hb, warnings }
//! ```

use crate::compiler::verifier::{MockVerifier, Verifier};
use crate::lang::goal::ContractualGoal;
use crate::lsp::parse_hb;
use thiserror::Error;

#[cfg(feature = "ai")]
pub mod client;

// ---------------------------------------------------------------------------
// AiGenError
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AiGenError {
    #[error("HTTP エラー ({status}): {body}")]
    Http { status: u16, body: String },

    #[error("API エラー: {message}")]
    Api { message: String },

    #[error("AI レスポンスのパース失敗:\n{errors}")]
    Parse { raw: String, errors: String },

    #[error("空のレスポンス: AI が .hb テキストを返しませんでした")]
    Empty,

    #[error("認証エラー: API キーが無効または未設定です")]
    Auth,
}

// ---------------------------------------------------------------------------
// AiGenOutput — 生成結果
// ---------------------------------------------------------------------------

/// `AiGoalGenerator::generate` が返す出力。
pub struct AiGenOutput {
    /// 生成された ContractualGoal のリスト
    pub goals: Vec<ContractualGoal>,
    /// AI（または Mock）が出力した生の .hb テキスト
    pub raw_hb: String,
    /// パースは成功したが問題のある条件の警告リスト
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// AiGoalGenerator トレイト
// ---------------------------------------------------------------------------

/// 自然言語の説明から `ContractualGoal` を生成するバックエンドの抽象インターフェース。
pub trait AiGoalGenerator {
    /// 自然言語の説明 (`description`) から ContractualGoal を生成する。
    ///
    /// # Errors
    /// - ネットワークエラー → `AiGenError::Http`
    /// - API 認証失敗 → `AiGenError::Auth`
    /// - レスポンスが空 → `AiGenError::Empty`
    /// - .hb パース失敗 → `AiGenError::Parse`
    fn generate(&self, description: &str) -> Result<AiGenOutput, AiGenError>;
}

// ---------------------------------------------------------------------------
// PromptBuilder — システムプロンプト + ユーザープロンプト生成
// ---------------------------------------------------------------------------

/// AI に送るプロンプトを構築する。
pub struct PromptBuilder;

impl PromptBuilder {
    /// AI へのシステムプロンプトを返す。
    /// Hammurabi の .hb フォーマットと契約の書き方を説明する。
    pub fn system_prompt() -> &'static str {
        r#"You are a ContractualGoal generator for the Hammurabi logic-first language.
Hammurabi defines functions via formal contracts (preconditions, postconditions, invariants)
instead of implementation code.

Given a natural language description, output a ContractualGoal in .hb format.

## .hb Format
```
goal <snake_case_name>
require <predicate>    # precondition (caller guarantees this)
ensure  <predicate>    # postcondition (function guarantees this, at least 1 required)
invariant <predicate>  # invariant (holds throughout execution)
forbid <Pattern>       # forbidden pattern (optional)
```

## Predicate Syntax
- `NonNull(var)`               — var must not be null/None
- `InRange(var, min, max)`     — integer var in [min, max]
- `Not(predicate)`             — logical negation
- `And(predicate, predicate)`  — logical conjunction
- `Or(predicate, predicate)`   — logical disjunction
- `atom_name`                  — named boolean property (snake_case)

## Forbidden Patterns
NonExhaustiveBranch  UnprovenUnwrap  RuntimeNullCheck
ImplicitCoercion     CatchAllSuppression

## Rules
1. Output ONLY valid .hb content — no explanation, no markdown fences
2. snake_case for all names
3. At least one `ensure` is required
4. Add `require NonNull(param)` for pointer/optional/string parameters
5. Add `InRange` for integer parameters with known bound constraints
6. `forbid RuntimeNullCheck` and `forbid UnprovenUnwrap` are recommended

## Example
Input: "Safely divide dividend by divisor. Divisor must not be zero."
Output:
goal safe_division
require Or(InRange(divisor, -9223372036854775808, -1), InRange(divisor, 1, 9223372036854775807))
require InRange(dividend, -9223372036854775808, 9223372036854775807)
ensure result_is_finite
invariant no_memory_aliasing
forbid RuntimeNullCheck
forbid UnprovenUnwrap
"#
    }

    /// ユーザープロンプト（説明文をラップする）を返す。
    pub fn user_prompt(description: &str) -> String {
        format!("Generate a ContractualGoal for:\n{description}")
    }
}

// ---------------------------------------------------------------------------
// MockAiGenerator — キーワード解析（API 不要）
// ---------------------------------------------------------------------------

/// キーワード解析で `ContractualGoal` を生成する開発・テスト用バックエンド。
/// API キーなしで動作し、一般的なパターンを認識して制約を生成する。
#[derive(Debug, Default)]
pub struct MockAiGenerator;

impl AiGoalGenerator for MockAiGenerator {
    fn generate(&self, description: &str) -> Result<AiGenOutput, AiGenError> {
        let raw_hb = build_hb_from_keywords(description);
        hb_to_output(raw_hb, description)
    }
}

/// キーワード解析で .hb テキストを構築する。
fn build_hb_from_keywords(desc: &str) -> String {
    let lower = desc.to_lowercase();
    let fn_name = infer_function_name(&lower);
    let mut lines: Vec<String> = vec![format!("goal {fn_name}")];

    // ── 変数候補を抽出 ────────────────────────────────────────────────
    let vars = extract_variable_names(&lower);

    // ── NonNull 制約 ──────────────────────────────────────────────────
    if lower.contains("non-null")
        || lower.contains("not null")
        || lower.contains("non null")
        || lower.contains("null check")
        || lower.contains("nullpointer")
    {
        for v in vars.iter().filter(|v| looks_like_ref(v)) {
            lines.push(format!("require NonNull({v})"));
        }
    }

    // ── 文字列/スライス引数は常に NonNull ────────────────────────────
    for v in vars.iter().filter(|v| is_string_like(v, &lower)) {
        let entry = format!("require NonNull({v})");
        if !lines.contains(&entry) {
            lines.push(entry);
        }
    }

    // ── 数値範囲制約 ──────────────────────────────────────────────────
    let numeric_vars: Vec<&str> = vars.iter()
        .filter(|v| is_numeric_var(v, &lower))
        .map(String::as_str)
        .collect();

    if lower.contains("positive") || lower.contains("greater than zero") {
        for v in &numeric_vars {
            lines.push(format!("require InRange({v}, 1, 9223372036854775807)"));
        }
    } else if lower.contains("non-negative")
        || lower.contains("nonnegative")
        || lower.contains("not negative")
        || lower.contains("zero or more")
    {
        for v in &numeric_vars {
            lines.push(format!("require InRange({v}, 0, 9223372036854775807)"));
        }
    } else if lower.contains("non-zero") || lower.contains("not zero") || lower.contains("nonzero") {
        for v in &numeric_vars {
            lines.push(format!(
                "require Or(InRange({v}, -9223372036854775808, -1), \
                              InRange({v}, 1, 9223372036854775807))"
            ));
        }
    } else {
        // 範囲制約なしでもデフォルトの全範囲を追加
        for v in &numeric_vars {
            lines.push(format!(
                "require InRange({v}, -9223372036854775808, 9223372036854775807)"
            ));
        }
    }

    // ── 事後条件 ──────────────────────────────────────────────────────
    let ensures = infer_postconditions(&lower, &fn_name);
    for e in ensures {
        lines.push(format!("ensure {e}"));
    }

    // ── 不変条件 ──────────────────────────────────────────────────────
    if lower.contains("thread") || lower.contains("concurrent") || lower.contains("parallel") {
        lines.push("invariant thread_safe".into());
    }
    if lower.contains("memory") || lower.contains("pointer") || lower.contains("reference") {
        lines.push("invariant no_memory_aliasing".into());
    }

    // ── 禁止パターン（常に追加） ─────────────────────────────────────
    lines.push("forbid RuntimeNullCheck".into());
    lines.push("forbid UnprovenUnwrap".into());
    if lower.contains("branch") || lower.contains("match") || lower.contains("exhaustive") {
        lines.push("forbid NonExhaustiveBranch".into());
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// キーワード解析ヘルパー関数
// ---------------------------------------------------------------------------

/// 説明文から関数名を推測する（snake_case）。
fn infer_function_name(lower: &str) -> String {
    // 動詞フレーズを抽出
    let verbs = [
        "divide", "multiply", "add", "subtract", "calculate", "compute",
        "validate", "verify", "check", "parse", "encode", "decode",
        "serialize", "deserialize", "hash", "encrypt", "decrypt",
        "sort", "filter", "search", "find", "get", "set", "create",
        "update", "delete", "insert", "remove", "convert", "transform",
        "format", "normalize", "clamp", "truncate", "round", "compress",
    ];

    // 目的語になりやすい名詞
    let nouns = [
        "division", "multiplication", "addition", "subtraction",
        "calculation", "computation", "validation", "verification",
        "password", "email", "url", "path", "file", "string",
        "integer", "number", "value", "data", "input", "output",
    ];

    // 動詞 + 名詞パターンを探す
    for verb in &verbs {
        if lower.contains(verb) {
            for noun in &nouns {
                if lower.contains(noun) {
                    return format!("{verb}_{noun}");
                }
            }
            // 動詞のみ一致した場合
            let obj = extract_direct_object(lower, verb);
            if let Some(o) = obj {
                return format!("{verb}_{}", to_snake_case(&o));
            }
            return verb.to_string();
        }
    }

    // 特定フレーズのパターンマッチ
    if lower.contains("safe division") || lower.contains("safely divid") {
        return "safe_division".into();
    }
    if lower.contains("square root") { return "sqrt".into(); }
    if lower.contains("factorial")   { return "factorial".into(); }
    if lower.contains("fibonacci")   { return "fibonacci".into(); }
    if lower.contains("gcd") || lower.contains("greatest common") {
        return "gcd".into();
    }

    // フォールバック: 説明の最初の 2〜3 単語を使う
    let words: Vec<&str> = lower
        .split_whitespace()
        .filter(|w| w.len() > 2 && !is_stopword(w))
        .take(2)
        .collect();
    if words.is_empty() {
        "my_function".into()
    } else {
        words.join("_")
    }
}

/// 動詞の直後の名詞を抽出する（簡易）。
fn extract_direct_object<'a>(lower: &'a str, verb: &str) -> Option<String> {
    let idx = lower.find(verb)?;
    let after = lower[idx + verb.len()..].trim_start();
    let word: String = after
        .split_whitespace()
        .next()?
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if word.len() > 2 { Some(word) } else { None }
}

/// 説明文から変数名らしい単語を抽出する。
fn extract_variable_names(lower: &str) -> Vec<String> {
    let param_indicators = [
        "parameter", "argument", "input", "the value", "the integer",
        "the number", "the string", "the divisor", "the dividend",
        "the numerator", "the denominator",
    ];

    let mut vars: Vec<String> = Vec::new();

    // 直接言及される変数名
    for indicator in &param_indicators {
        if let Some(idx) = lower.find(indicator) {
            let after = lower[idx + indicator.len()..].trim_start();
            let word: String = after
                .split_whitespace()
                .next()
                .unwrap_or("")
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .filter(|c| c.is_alphabetic() || *c == '_')
                .collect();
            if word.len() > 1 && !vars.contains(&word) {
                vars.push(word);
            }
        }
    }

    // よく使われる変数名のパターン
    for var in &["divisor", "dividend", "numerator", "denominator",
                  "value", "input", "n", "x", "y", "base", "exponent",
                  "index", "count", "size", "length", "capacity"]
    {
        if lower.contains(var) && !vars.contains(&var.to_string()) {
            vars.push(var.to_string());
        }
    }

    // 何も抽出できなかった場合のデフォルト
    if vars.is_empty() {
        vars.push("input".into());
    }

    vars
}

/// 変数名がポインタ・参照・オプション型らしいかどうかを判定する。
fn looks_like_ref(var: &str) -> bool {
    matches!(var, "ptr" | "ref" | "handle" | "object" | "node" | "pointer" | "reference")
}

/// 変数名が文字列・パス系らしいか（説明文からも判断）。
fn is_string_like(var: &str, lower: &str) -> bool {
    matches!(var, "str" | "string" | "text" | "input" | "name" | "path" | "url" | "email")
        || lower.contains(&format!("{var} string"))
        || lower.contains(&format!("{var} str"))
}

/// 変数名が数値系らしいか判定する。
fn is_numeric_var(var: &str, lower: &str) -> bool {
    matches!(
        var,
        "n" | "x" | "y" | "z" | "value" | "count" | "size" | "length"
        | "capacity" | "index" | "divisor" | "dividend" | "numerator"
        | "denominator" | "base" | "exponent"
    ) || lower.contains(&format!("{var} integer"))
      || lower.contains(&format!("{var} number"))
}

/// 説明から事後条件のアトム名を推測する。
fn infer_postconditions(lower: &str, fn_name: &str) -> Vec<String> {
    let mut posts = Vec::new();

    if lower.contains("finite") || lower.contains("result") {
        posts.push("result_is_finite".into());
    }
    if lower.contains("valid") || lower.contains("correct") {
        posts.push(format!("{fn_name}_result_valid"));
    }
    if lower.contains("sorted") || lower.contains("ordered") {
        posts.push("output_is_sorted".into());
    }
    if lower.contains("unique") || lower.contains("distinct") {
        posts.push("output_contains_no_duplicates".into());
    }
    if lower.contains("empty") && lower.contains("not") {
        posts.push("output_is_non_empty".into());
    }
    if lower.contains("non-negative") || lower.contains("positive") {
        posts.push("result_is_non_negative".into());
    }

    // フォールバック
    if posts.is_empty() {
        posts.push(format!("{fn_name}_succeeds"));
    }
    posts
}

fn is_stopword(w: &str) -> bool {
    matches!(w, "the" | "a" | "an" | "is" | "are" | "was" | "for"
                | "that" | "this" | "with" | "from" | "and" | "or"
                | "not" | "has" | "have" | "will" | "must" | "should")
}

fn to_snake_case(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_owned()
}

// ---------------------------------------------------------------------------
// .hb テキスト → AiGenOutput 変換（パース + 検証）
// ---------------------------------------------------------------------------

/// .hb テキストをパース・検証して `AiGenOutput` にまとめる。
/// パースエラーがあれば `AiGenError::Parse` を返す。
pub fn hb_to_output(raw_hb: String, context: &str) -> Result<AiGenOutput, AiGenError> {
    let parse_result = parse_hb(&raw_hb);

    // パースエラーがある場合 → 失敗
    if !parse_result.errors.is_empty() {
        let errors = parse_result.errors.iter()
            .map(|e| format!("  Line {}: {}", e.span.line + 1, e.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(AiGenError::Parse { raw: raw_hb, errors });
    }

    if parse_result.goals.is_empty() {
        return Err(AiGenError::Empty);
    }

    // 各ゴールを MockVerifier で検証して warnings を収集
    let verifier = MockVerifier::default();
    let mut warnings: Vec<String> = Vec::new();
    let goals: Vec<ContractualGoal> = parse_result.goals
        .into_iter()
        .map(|pg| {
            if let Ok(report) = verifier.verify_goal(&pg.goal) {
                warnings.extend(report.violations.clone());
            }
            pg.goal
        })
        .collect();

    // 説明文を参照して contextual warning を追加
    if context.len() < 10 {
        warnings.push("説明が短すぎます。より詳細な記述で精度が上がります。".into());
    }

    Ok(AiGenOutput { goals, raw_hb, warnings })
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_generates_safe_division() {
        let gen = MockAiGenerator;
        let out = gen.generate(
            "Safely divide dividend by divisor. The divisor must be non-zero."
        ).unwrap();

        assert!(!out.goals.is_empty(), "goals が空");
        let goal = &out.goals[0];
        assert!(!goal.postconditions.is_empty(), "事後条件がない");
        println!("generated .hb:\n{}", out.raw_hb);
    }

    #[test]
    fn mock_generates_from_positive_constraint() {
        let gen = MockAiGenerator;
        let out = gen.generate(
            "Calculate the factorial of n. n must be a positive integer."
        ).unwrap();

        assert!(!out.goals.is_empty());
        // InRange(n, 1, ...) が生成されていること
        assert!(out.raw_hb.contains("InRange(n,") || out.raw_hb.contains("InRange(n ,"),
            "positive 制約が生成されなかった:\n{}", out.raw_hb);
        println!("generated .hb:\n{}", out.raw_hb);
    }

    #[test]
    fn mock_generates_from_non_negative_constraint() {
        let gen = MockAiGenerator;
        let out = gen.generate(
            "Compute the square root of value. Value must be non-negative."
        ).unwrap();

        assert!(!out.goals.is_empty());
        assert!(out.raw_hb.contains("InRange(value, 0,"),
            "non-negative 制約が生成されなかった:\n{}", out.raw_hb);
    }

    #[test]
    fn mock_generates_forbidden_patterns() {
        let gen = MockAiGenerator;
        let out = gen.generate("Validate the email string.").unwrap();
        assert!(out.raw_hb.contains("forbid RuntimeNullCheck"));
        assert!(out.raw_hb.contains("forbid UnprovenUnwrap"));
    }

    #[test]
    fn mock_infers_function_name_from_verb() {
        let gen = MockAiGenerator;
        let out = gen.generate("Parse a JSON string and return the parsed value.").unwrap();
        assert!(!out.goals.is_empty());
        let goal_name = &out.goals[0].name;
        assert!(
            goal_name.contains("parse") || goal_name.contains("json"),
            "関数名の推測に失敗: `{goal_name}`"
        );
    }

    #[test]
    fn prompt_builder_system_prompt_is_not_empty() {
        let prompt = PromptBuilder::system_prompt();
        assert!(prompt.contains(".hb Format"));
        assert!(prompt.contains("require"));
        assert!(prompt.contains("ensure"));
        assert!(prompt.contains("NonNull"));
        assert!(prompt.contains("InRange"));
    }

    #[test]
    fn prompt_builder_user_prompt_wraps_description() {
        let prompt = PromptBuilder::user_prompt("divide two integers");
        assert!(prompt.contains("divide two integers"));
    }

    #[test]
    fn hb_to_output_parse_error_returns_err() {
        let bad_hb = "this is not valid .hb content".to_string();
        let result = hb_to_output(bad_hb, "test");
        assert!(matches!(result, Err(AiGenError::Parse { .. })));
    }

    #[test]
    fn hb_to_output_empty_returns_err() {
        let result = hb_to_output(String::new(), "test");
        // 空の場合は Empty か Parse
        assert!(matches!(result, Err(AiGenError::Empty) | Err(AiGenError::Parse { .. })));
    }

    #[test]
    fn full_flow_generate_and_inspect() {
        let gen = MockAiGenerator;
        let desc = "Compute the GCD of two non-negative integers a and b using Euclid's algorithm.";
        let out = gen.generate(desc).unwrap();

        println!("=== Generated .hb ===\n{}", out.raw_hb);
        println!("=== Goals ({}) ===", out.goals.len());
        for g in &out.goals {
            println!("  name: {}", g.name);
            println!("  pre : {}", g.preconditions.len());
            println!("  post: {}", g.postconditions.len());
        }
        if !out.warnings.is_empty() {
            println!("=== Warnings ===");
            for w in &out.warnings { println!("  ⚠ {w}"); }
        }

        assert!(!out.goals.is_empty());
    }
}
