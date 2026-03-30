//! # codegen — ContractualGoal から各言語のコードスケルトンを生成するモジュール
//!
//! ## 対応言語
//! `TargetLang` enum で選択: Rust / Python / Go / Java / JavaScript / TypeScript
//!
//! ## 生成の流れ
//! ```text
//! ContractualGoal  +  TargetLang
//!   ↓ extract_variables()   — Predicate AST から変数・制約を収集
//!   ↓ emit_header()         — import / use 宣言
//!   ↓ emit_doc()            — 契約をコメントとして出力
//!   ↓ emit_signature()      — 言語固有のシグネチャ
//!   ↓ emit_body()           — 制約チェック + TODO
//! CodegenOutput { source, lang }
//! ```

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::str::FromStr;
use crate::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};

// ---------------------------------------------------------------------------
// TargetLang — 出力対象の言語
// ---------------------------------------------------------------------------

/// コード生成の対象言語。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TargetLang {
    #[default]
    Rust,
    Python,
    Go,
    Java,
    JavaScript,
    TypeScript,
}

impl TargetLang {
    /// ソースファイルの拡張子を返す。
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Rust       => "rs",
            Self::Python     => "py",
            Self::Go         => "go",
            Self::Java       => "java",
            Self::JavaScript => "js",
            Self::TypeScript => "ts",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Rust       => "Rust",
            Self::Python     => "Python",
            Self::Go         => "Go",
            Self::Java       => "Java",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
        }
    }
}

impl FromStr for TargetLang {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "rust"             => Ok(Self::Rust),
            "python" | "py"   => Ok(Self::Python),
            "go" | "golang"   => Ok(Self::Go),
            "java"             => Ok(Self::Java),
            "javascript" | "js" => Ok(Self::JavaScript),
            "typescript" | "ts" => Ok(Self::TypeScript),
            other => Err(format!(
                "未知の言語: `{other}` — rust / python / go / java / javascript / typescript のいずれかを指定"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// マクロ補助 — push! で format+push を 1 行に
// ---------------------------------------------------------------------------

macro_rules! push {
    ($buf:expr, $fmt:literal $(, $args:expr)*) => {
        $buf.push_str(&format!($fmt $(, $args)*))
    };
}

// ---------------------------------------------------------------------------
// VarInfo — 述語 AST から抽出した変数ひとつ分の情報
// ---------------------------------------------------------------------------

/// 述語から抽出した「変数 + その変数に課される制約コード文字列のリスト」。
#[derive(Debug, Clone)]
struct VarInfo {
    /// Rust 識別子として使える変数名
    name: String,
    /// `Constraint::Xxx { ... }` の形式のコード文字列
    constraints: Vec<String>,
}

// ---------------------------------------------------------------------------
// 型変換ヘルパー — Rust スタイルの型を各言語の型に変換
// ---------------------------------------------------------------------------

/// Rust スタイルの型文字列を対象言語の型表現に変換する。
/// ネストした `<>` や `()` を考慮して再帰的に処理する。
pub(crate) fn translate_type(type_str: &str, lang: &TargetLang) -> String {
    let s = type_str.trim();
    match lang {
        TargetLang::Rust => s.to_owned(),
        TargetLang::Python  => rust_to_python(s),
        TargetLang::Go      => rust_to_go(s),
        TargetLang::Java    => rust_to_java(s),
        TargetLang::JavaScript  => String::new(), // JS は型なし
        TargetLang::TypeScript  => rust_to_ts(s),
    }
}

fn rust_to_python(s: &str) -> String {
    match s {
        "i8"|"i16"|"i32"|"i64"|"i128"|"isize"|
        "u8"|"u16"|"u32"|"u64"|"u128"|"usize" => "int".into(),
        "f32"|"f64"    => "float".into(),
        "bool"         => "bool".into(),
        "String"|"str"|"&str" => "str".into(),
        "()"           => "None".into(),
        _ => {
            if let Some(inner) = strip_generic("Option", s) {
                format!("Optional[{}]", rust_to_python(inner))
            } else if let Some(inner) = strip_generic("Vec", s) {
                format!("list[{}]", rust_to_python(inner))
            } else if let Some((k, v)) = strip_generic2("HashMap", s) {
                format!("dict[{}, {}]", rust_to_python(k), rust_to_python(v))
            } else if let Some((ok, err)) = strip_generic2("Result", s) {
                // Python では例外モデル; 戻り値は Ok 型のみ
                let _ = err;
                rust_to_python(ok)
            } else if let Some(inner) = strip_generic("Box", s) {
                rust_to_python(inner)
            } else {
                s.to_owned()
            }
        }
    }
}

fn rust_to_go(s: &str) -> String {
    match s {
        "i8"   => "int8".into(),  "i16"  => "int16".into(),
        "i32"  => "int32".into(), "i64"  => "int64".into(),
        "isize"=> "int".into(),   "i128" => "int64".into(),
        "u8"   => "uint8".into(), "u16"  => "uint16".into(),
        "u32"  => "uint32".into(),"u64"  => "uint64".into(),
        "usize"=> "uint".into(),  "u128" => "uint64".into(),
        "f32"  => "float32".into(),"f64" => "float64".into(),
        "bool" => "bool".into(),
        "String"|"str"|"&str" => "string".into(),
        "()"   => "".into(),
        _ => {
            if let Some(inner) = strip_generic("Option", s) {
                format!("*{}", rust_to_go(inner))
            } else if let Some(inner) = strip_generic("Vec", s) {
                format!("[]{}", rust_to_go(inner))
            } else if let Some((k, v)) = strip_generic2("HashMap", s) {
                format!("map[{}]{}", rust_to_go(k), rust_to_go(v))
            } else if let Some((ok, _err)) = strip_generic2("Result", s) {
                format!("({}, error)", rust_to_go(ok))
            } else if let Some(inner) = strip_generic("Box", s) {
                format!("*{}", rust_to_go(inner))
            } else {
                s.to_owned()
            }
        }
    }
}

fn rust_to_java(s: &str) -> String {
    match s {
        "i8"   => "byte".into(),  "i16"  => "short".into(),
        "i32"  => "int".into(),   "i64"  => "long".into(),
        "u8"|"u16"|"u32"   => "int".into(),
        "u64"|"i128"|"u128"=> "long".into(),
        "usize"|"isize"    => "long".into(),
        "f32"  => "float".into(), "f64"  => "double".into(),
        "bool" => "boolean".into(),
        "String"|"str"|"&str" => "String".into(),
        "()"   => "void".into(),
        _ => {
            if let Some(inner) = strip_generic("Option", s) {
                format!("Optional<{}>", rust_to_java(inner))
            } else if let Some(inner) = strip_generic("Vec", s) {
                format!("List<{}>", rust_to_java(inner))
            } else if let Some((k, v)) = strip_generic2("HashMap", s) {
                format!("Map<{}, {}>", rust_to_java(k), rust_to_java(v))
            } else if let Some((ok, _)) = strip_generic2("Result", s) {
                rust_to_java(ok)
            } else if let Some(inner) = strip_generic("Box", s) {
                rust_to_java(inner)
            } else {
                s.to_owned()
            }
        }
    }
}

fn rust_to_ts(s: &str) -> String {
    match s {
        "i8"|"i16"|"i32"|"i64"|"i128"|"isize"|
        "u8"|"u16"|"u32"|"u64"|"u128"|"usize"|
        "f32"|"f64" => "number".into(),
        "bool"      => "boolean".into(),
        "String"|"str"|"&str" => "string".into(),
        "()"        => "void".into(),
        _ => {
            if let Some(inner) = strip_generic("Option", s) {
                format!("{} | null", rust_to_ts(inner))
            } else if let Some(inner) = strip_generic("Vec", s) {
                format!("{}[]", rust_to_ts(inner))
            } else if let Some((k, v)) = strip_generic2("HashMap", s) {
                format!("Record<{}, {}>", rust_to_ts(k), rust_to_ts(v))
            } else if let Some((ok, _)) = strip_generic2("Result", s) {
                format!("{} | Error", rust_to_ts(ok))
            } else if let Some(inner) = strip_generic("Box", s) {
                rust_to_ts(inner)
            } else {
                s.to_owned()
            }
        }
    }
}

/// `Wrapper<Inner>` から `Inner` を取り出す
fn strip_generic<'a>(wrapper: &str, s: &'a str) -> Option<&'a str> {
    let prefix = format!("{wrapper}<");
    if s.starts_with(prefix.as_str()) && s.ends_with('>') {
        Some(&s[prefix.len()..s.len() - 1])
    } else {
        None
    }
}

/// `Wrapper<K, V>` から `(K, V)` を取り出す（トップレベルカンマで分割）
fn strip_generic2<'a>(wrapper: &str, s: &'a str) -> Option<(&'a str, &'a str)> {
    let prefix = format!("{wrapper}<");
    if !s.starts_with(prefix.as_str()) || !s.ends_with('>') {
        return None;
    }
    let inner = &s[prefix.len()..s.len() - 1];
    // トップレベルのカンマを探す
    let mut depth = 0i32;
    for (i, ch) in inner.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                return Some((&inner[..i], inner[i + 1..].trim_start()));
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 変数抽出ロジック
// ---------------------------------------------------------------------------

/// `Predicate` の木を再帰的に走査して `VarInfo` を収集する。
/// 同じ変数名が複数の述語に登場した場合は制約をマージする。
fn extract_variables(predicates: &[Predicate]) -> Vec<VarInfo> {
    let mut vars: Vec<VarInfo> = Vec::new();

    for pred in predicates {
        collect_pred(pred, &mut vars);
    }
    vars
}

fn collect_pred(pred: &Predicate, vars: &mut Vec<VarInfo>) {
    match pred {
        Predicate::NonNull(var) => {
            upsert(vars, var, "Constraint::NonNull".into());
        }
        Predicate::InRange { var, min, max } => {
            upsert(
                vars,
                var,
                format!("Constraint::InRange {{ min: {min}_i64, max: {max}_i64 }}"),
            );
        }
        Predicate::Equals(a, b) => {
            upsert(
                vars,
                a,
                format!("Constraint::ConsistentWith(\"{b}\".into())"),
            );
        }
        // 複合述語は再帰的に展開
        Predicate::And(l, r) | Predicate::Or(l, r) | Predicate::Implies(l, r) => {
            collect_pred(l, vars);
            collect_pred(r, vars);
        }
        Predicate::Not(p) => collect_pred(p, vars),
        Predicate::ForAll { body, .. } | Predicate::Exists { body, .. } => {
            collect_pred(body, vars);
        }
        // Atom / True / False は変数を持たない
        Predicate::Atom(_) | Predicate::True | Predicate::False => {}
    }
}

/// 変数リストに upsert する（既存なら制約を追記、なければ新規挿入）。
fn upsert(vars: &mut Vec<VarInfo>, name: &str, constraint: String) {
    if let Some(v) = vars.iter_mut().find(|v| v.name == name) {
        if !v.constraints.contains(&constraint) {
            v.constraints.push(constraint);
        }
    } else {
        vars.push(VarInfo {
            name: name.to_owned(),
            constraints: vec![constraint],
        });
    }
}

// ---------------------------------------------------------------------------
// CodeGenerator — 生成エントリポイント
// ---------------------------------------------------------------------------

/// `ContractualGoal` から指定言語のコードスケルトンを生成するジェネレーター。
///
/// # 使い方
/// ```rust,ignore
/// use hammurabi::codegen::{CodeGenerator, TargetLang};
/// use hammurabi::lang::goal::{ContractualGoal, Predicate};
///
/// let goal = ContractualGoal::new("safe_division")
///     .require(Predicate::in_range("divisor", 1, i64::MAX))
///     .ensure(Predicate::atom("result_is_finite"));
///
/// // Rust（デフォルト）
/// let output = CodeGenerator::new().generate(&goal);
/// output.save_to_file("src/safe_division.rs").unwrap();
///
/// // Python
/// let output = CodeGenerator::for_lang(TargetLang::Python).generate(&goal);
/// output.save_to_file("safe_division.py").unwrap();
/// ```
#[derive(Debug, Default)]
pub struct CodeGenerator {
    pub lang: TargetLang,
}

impl CodeGenerator {
    /// Rust 向けジェネレーター（後方互換）
    pub fn new() -> Self {
        Self { lang: TargetLang::Rust }
    }

    /// 指定言語向けジェネレーター
    pub fn for_lang(lang: TargetLang) -> Self {
        Self { lang }
    }

    /// `ContractualGoal` から `CodegenOutput` を生成する。
    pub fn generate(&self, goal: &ContractualGoal) -> CodegenOutput {
        let source = match self.lang {
            TargetLang::Rust       => self.gen_rust(goal),
            TargetLang::Python     => self.gen_python(goal),
            TargetLang::Go         => self.gen_go(goal),
            TargetLang::Java       => self.gen_java(goal),
            TargetLang::JavaScript => self.gen_javascript(goal),
            TargetLang::TypeScript => self.gen_typescript(goal),
        };
        CodegenOutput { source, lang: self.lang.clone() }
    }

    // =======================================================================
    // Rust
    // =======================================================================

    fn gen_rust(&self, goal: &ContractualGoal) -> String {
        let mut buf = String::new();
        let vars     = extract_variables(&goal.preconditions);
        let fn_name  = to_snake_case(&goal.name);
        let ensures  = postcond_strings(goal);

        // ヘッダー
        push!(buf, "//! Generated by Hammurabi — do not edit manually.\n");
        push!(buf, "//! ContractualGoal: `{}`\n\n", goal.name);
        push!(buf, "#![allow(unused_variables, dead_code)]\n\n");
        push!(buf, "use hammurabi::compiler::verifier::{{Verifier, VerificationError}};\n");
        push!(buf, "use hammurabi::lang::rail::{{Constraint, LogicRail}};\n\n");

        // doc コメント
        push!(buf, "/// # Contract: `{}`\n///\n", goal.name);
        if !goal.inputs.is_empty() {
            let sig = goal.inputs.iter()
                .map(|p| format!("{}: {}", p.name, p.type_str))
                .collect::<Vec<_>>().join(", ");
            push!(buf, "/// ## Signature\n/// `fn {fn_name}({sig})`\n///\n");
        }
        push!(buf, "/// ## Preconditions\n");
        for pre in &goal.preconditions { push!(buf, "/// - `{pre}`\n"); }
        push!(buf, "///\n/// ## Postconditions\n");
        for post in &goal.postconditions { push!(buf, "/// - `{}`\n", post); }
        if !goal.invariants.is_empty() {
            push!(buf, "///\n/// ## Invariants\n");
            for inv in &goal.invariants { push!(buf, "/// - `{inv}`\n"); }
        }
        push!(buf, "///\n/// ## Forbidden Patterns\n");
        for fp in &goal.forbidden {
            push!(buf, "/// - `{fp}` — {}\n", forbidden_description(fp));
        }
        push!(buf, "///\n");

        // シグネチャ: inputs: が指定されている場合はそちらを使う
        let ret_type = goal.output.as_deref().unwrap_or("Result<LogicRail<i64>, VerificationError>");
        if !goal.inputs.is_empty() {
            push!(buf, "pub fn {fn_name}(\n");
            for p in &goal.inputs {
                push!(buf, "    {}: {},\n", p.name, p.type_str);
            }
            push!(buf, ") -> {ret_type} {{\n");
        } else {
            // フォールバック: 述語から変数を推定
            push!(buf, "pub fn {fn_name}<V>(\n    verifier: &V,\n");
            for v in &vars {
                push!(buf, "    {}: i64,\n", v.name);
            }
            push!(buf, ") -> {ret_type}\nwhere\n    V: Verifier,\n{{\n");
        }

        // ボディ: precondition binds
        if !vars.is_empty() {
            push!(buf, "    // ── Precondition binds ───────────────────────────────────\n");
            for v in &vars {
                let vn = &v.name;
                push!(buf, "    let {vn} = LogicRail::bind(\n        \"{vn}\",\n        {vn},\n        vec![\n");
                for c in &v.constraints { push!(buf, "            {c},\n"); }
                push!(buf, "        ],\n        verifier,\n    )?;\n");
            }
            push!(buf, "\n");
        }

        // 不変条件
        if !goal.invariants.is_empty() {
            push!(buf, "    // ── Invariants ────────────────────────────────────────────\n");
            for inv in &goal.invariants { push!(buf, "    // invariant: {inv}\n"); }
            push!(buf, "\n");
        }

        // 禁止パターン
        push!(buf, "    // ── Forbidden Patterns ──────────────────────────────────────\n");
        for fp in &goal.forbidden { push!(buf, "    // ✗ {fp}\n"); }
        push!(buf, "\n");

        // When ブロック（条件付き事後条件）
        let when_block = render_when_postconds(goal, &TargetLang::Rust, "    ");
        if !when_block.is_empty() {
            push!(buf, "{when_block}\n");
        }

        // 通常の事後条件 TODO
        if ensures.is_empty() && when_block.is_empty() {
            push!(buf, "    todo!(\"Implement {fn_name}\")\n");
        } else if !ensures.is_empty() {
            push!(buf, "    // TODO: Postconditions を満たす実装を書いてください\n");
            for e in &ensures { push!(buf, "    // ensure: {e}\n"); }
            push!(buf, "    todo!(\"Implement {fn_name}: ensure {}\")\n", ensures[0]);
        }
        push!(buf, "}}\n");
        buf.push_str(&emit_examples(goal, &TargetLang::Rust));
        buf
    }

    // =======================================================================
    // Python
    // =======================================================================

    fn gen_python(&self, goal: &ContractualGoal) -> String {
        let mut buf = String::new();
        let vars    = extract_variables(&goal.preconditions);
        let fn_name = to_snake_case(&goal.name);
        let ensures = postcond_strings(goal);

        push!(buf, "# Generated by Hammurabi — do not edit manually.\n");
        push!(buf, "# ContractualGoal: `{}`\n\n", goal.name);
        push!(buf, "from typing import Optional\n\n\n");

        // シグネチャ: inputs/output 指定があればそちらを、なければ変数推定
        let params: Vec<String> = if !goal.inputs.is_empty() {
            goal.inputs.iter()
                .map(|p| {
                    let py_type = translate_type(&p.type_str, &TargetLang::Python);
                    if py_type.is_empty() { p.name.clone() }
                    else { format!("{}: {}", p.name, py_type) }
                }).collect()
        } else {
            vars.iter().map(|v| format!("{}: int", v.name)).collect()
        };
        let ret_type = goal.output.as_deref()
            .map(|t| translate_type(t, &TargetLang::Python))
            .unwrap_or_else(|| "int".into());
        push!(buf, "def {}({}) -> {}:\n", fn_name, params.join(", "), ret_type);
        push!(buf, "    \"\"\"\n    Contract: {}\n\n", goal.name);
        push!(buf, "    Preconditions:\n");
        for pre in &goal.preconditions { push!(buf, "      - {pre}\n"); }
        push!(buf, "\n    Postconditions:\n");
        for post in &goal.postconditions { push!(buf, "      - {}\n", post); }
        if !goal.invariants.is_empty() {
            push!(buf, "\n    Invariants:\n");
            for inv in &goal.invariants { push!(buf, "      - {inv}\n"); }
        }
        push!(buf, "\n    Forbidden: {}\n", goal.forbidden.iter()
            .map(|f| f.to_string()).collect::<Vec<_>>().join(", "));
        push!(buf, "    \"\"\"\n");

        // 事前条件チェック
        if !vars.is_empty() {
            push!(buf, "    # ── Precondition checks ─────────────────────────────\n");
            for v in &vars {
                for c in &v.constraints {
                    push!(buf, "{}", python_check(&v.name, c, "    "));
                }
            }
            push!(buf, "\n");
        }

        // 不変条件
        if !goal.invariants.is_empty() {
            push!(buf, "    # ── Invariants ──────────────────────────────────────\n");
            for inv in &goal.invariants { push!(buf, "    # invariant: {inv}\n"); }
            push!(buf, "\n");
        }

        // When ブロック（条件付き事後条件）
        let when_block = render_when_postconds(goal, &TargetLang::Python, "    ");
        if !when_block.is_empty() {
            push!(buf, "{when_block}\n");
        }

        // 通常の事後条件 TODO
        if ensures.is_empty() && when_block.is_empty() {
            push!(buf, "    raise NotImplementedError(\"Implement {fn_name}\")\n");
        } else if !ensures.is_empty() {
            push!(buf, "    # TODO: Postconditions を満たす実装を書いてください\n");
            for e in &ensures { push!(buf, "    # ensure: {e}\n"); }
            push!(buf, "    raise NotImplementedError(\"Implement {fn_name}: ensure {}\")\n", ensures[0]);
        }
        buf.push_str(&emit_examples(goal, &TargetLang::Python));
        buf
    }

    // =======================================================================
    // Go
    // =======================================================================

    fn gen_go(&self, goal: &ContractualGoal) -> String {
        let mut buf = String::new();
        let vars    = extract_variables(&goal.preconditions);
        let fn_name = to_camel_case(&goal.name);
        let ensures = postcond_strings(goal);

        push!(buf, "// Generated by Hammurabi — do not edit manually.\n");
        push!(buf, "// ContractualGoal: {}\n\n", goal.name);
        push!(buf, "package hammurabi\n\nimport \"fmt\"\n\n");

        // コメント
        push!(buf, "// {} implements the ContractualGoal: {}\n//\n", fn_name, goal.name);
        push!(buf, "// Preconditions:\n");
        for pre in &goal.preconditions { push!(buf, "//   - {pre}\n"); }
        push!(buf, "//\n// Postconditions:\n");
        for post in &goal.postconditions { push!(buf, "//   - {}\n", post); }
        if !goal.invariants.is_empty() {
            push!(buf, "//\n// Invariants:\n");
            for inv in &goal.invariants { push!(buf, "//   - {inv}\n"); }
        }
        push!(buf, "//\n// Forbidden: {}\n", goal.forbidden.iter()
            .map(|f| f.to_string()).collect::<Vec<_>>().join(", "));

        // シグネチャ: inputs/output 指定があればそちらを使う
        let params: Vec<String> = if !goal.inputs.is_empty() {
            goal.inputs.iter()
                .map(|p| format!("{} {}", p.name, translate_type(&p.type_str, &TargetLang::Go)))
                .collect()
        } else {
            vars.iter().map(|v| format!("{} int64", v.name)).collect()
        };
        let ret_type = goal.output.as_deref()
            .map(|t| {
                let go_t = translate_type(t, &TargetLang::Go);
                if go_t.is_empty() { String::new() } else { format!("({go_t}, error)") }
            })
            .unwrap_or_else(|| "(int64, error)".into());
        if ret_type.is_empty() {
            push!(buf, "func {}({}) error {{\n", fn_name, params.join(", "));
        } else {
            push!(buf, "func {}({}) {} {{\n", fn_name, params.join(", "), ret_type);
        }

        // 事前条件チェック
        if !vars.is_empty() {
            push!(buf, "\t// ── Precondition checks ─────────────────────────────\n");
            for v in &vars {
                for c in &v.constraints {
                    push!(buf, "{}", go_check(&v.name, c, "\t"));
                }
            }
            push!(buf, "\n");
        }

        // 不変条件
        if !goal.invariants.is_empty() {
            push!(buf, "\t// ── Invariants ──────────────────────────────────────\n");
            for inv in &goal.invariants { push!(buf, "\t// invariant: {inv}\n"); }
            push!(buf, "\n");
        }

        // When ブロック（条件付き事後条件）
        let when_block = render_when_postconds(goal, &TargetLang::Go, "\t");
        if !when_block.is_empty() {
            push!(buf, "{when_block}\n");
        }

        // 通常の事後条件 TODO
        if ensures.is_empty() && when_block.is_empty() {
            push!(buf, "\tpanic(\"implement {fn_name}\")\n");
        } else if !ensures.is_empty() {
            push!(buf, "\t// TODO: Postconditions を満たす実装を書いてください\n");
            for e in &ensures { push!(buf, "\t// ensure: {e}\n"); }
            push!(buf, "\tpanic(\"implement {fn_name}: ensure {}\")\n", ensures[0]);
        }
        push!(buf, "}}\n");
        buf.push_str(&emit_examples(goal, &TargetLang::Go));
        buf
    }

    // =======================================================================
    // Java
    // =======================================================================

    fn gen_java(&self, goal: &ContractualGoal) -> String {
        let mut buf = String::new();
        let vars    = extract_variables(&goal.preconditions);
        let fn_name = to_camel_case(&goal.name);
        let class   = to_pascal_case(&goal.name);
        let ensures = postcond_strings(goal);

        push!(buf, "// Generated by Hammurabi — do not edit manually.\n\n");
        push!(buf, "import java.util.Objects;\n\n");
        push!(buf, "public class {} {{\n\n", class);

        // Javadoc
        push!(buf, "    /**\n     * Contract: {}\n     *\n", goal.name);
        push!(buf, "     * <p>Preconditions:</p><ul>\n");
        for pre in &goal.preconditions { push!(buf, "     * <li>{pre}</li>\n"); }
        push!(buf, "     * </ul>\n     * <p>Postconditions:</p><ul>\n");
        for post in &goal.postconditions { push!(buf, "     * <li>{}</li>\n", post); }
        push!(buf, "     * </ul>\n");
        if !goal.invariants.is_empty() {
            push!(buf, "     * <p>Invariants:</p><ul>\n");
            for inv in &goal.invariants { push!(buf, "     * <li>{inv}</li>\n"); }
            push!(buf, "     * </ul>\n");
        }
        push!(buf, "     * <p>Forbidden: {}</p>\n", goal.forbidden.iter()
            .map(|f| f.to_string()).collect::<Vec<_>>().join(", "));
        push!(buf, "     */\n");

        // シグネチャ: inputs/output 指定があればそちらを使う
        let params: Vec<String> = if !goal.inputs.is_empty() {
            goal.inputs.iter()
                .map(|p| format!("{} {}", translate_type(&p.type_str, &TargetLang::Java), p.name))
                .collect()
        } else {
            vars.iter().map(|v| format!("long {}", v.name)).collect()
        };
        let ret_type = goal.output.as_deref()
            .map(|t| translate_type(t, &TargetLang::Java))
            .unwrap_or_else(|| "long".into());
        push!(buf, "    public static {} {}({}) {{\n", ret_type, fn_name, params.join(", "));

        // 事前条件チェック
        if !vars.is_empty() {
            push!(buf, "        // ── Precondition checks ─────────────────────────────\n");
            for v in &vars {
                for c in &v.constraints {
                    push!(buf, "{}", java_check(&v.name, c, "        "));
                }
            }
            push!(buf, "\n");
        }

        // 不変条件
        if !goal.invariants.is_empty() {
            push!(buf, "        // ── Invariants ──────────────────────────────────────\n");
            for inv in &goal.invariants { push!(buf, "        // invariant: {inv}\n"); }
            push!(buf, "\n");
        }

        // When ブロック（条件付き事後条件）
        let when_block = render_when_postconds(goal, &TargetLang::Java, "        ");
        if !when_block.is_empty() {
            push!(buf, "{when_block}\n");
        }

        // 通常の事後条件 TODO
        if ensures.is_empty() && when_block.is_empty() {
            push!(buf, "        throw new UnsupportedOperationException(\"Implement {fn_name}\");\n");
        } else if !ensures.is_empty() {
            push!(buf, "        // TODO: Postconditions を満たす実装を書いてください\n");
            for e in &ensures { push!(buf, "        // ensure: {e}\n"); }
            push!(buf, "        throw new UnsupportedOperationException(\n");
            push!(buf, "            \"Implement {fn_name}: ensure {}\");\n", ensures[0]);
        }
        push!(buf, "    }}\n}}\n");
        buf.push_str(&emit_examples(goal, &TargetLang::Java));
        buf
    }

    // =======================================================================
    // JavaScript
    // =======================================================================

    fn gen_javascript(&self, goal: &ContractualGoal) -> String {
        let mut buf = String::new();
        let vars    = extract_variables(&goal.preconditions);
        let fn_name = to_camel_case(&goal.name);
        let ensures = postcond_strings(goal);

        push!(buf, "// Generated by Hammurabi — do not edit manually.\n");
        push!(buf, "// ContractualGoal: {}\n\n", goal.name);

        // JSDoc
        push!(buf, "/**\n * Contract: {}\n *\n", goal.name);
        push!(buf, " * @description\n * Preconditions:\n");
        for pre in &goal.preconditions { push!(buf, " *   - {pre}\n"); }
        push!(buf, " * Postconditions:\n");
        for post in &goal.postconditions { push!(buf, " *   - {}\n", post); }
        if !goal.inputs.is_empty() {
            for p in &goal.inputs { push!(buf, " * @param {{{}}} {}\n", p.type_str, p.name); }
        } else {
            for v in &vars { push!(buf, " * @param {{number}} {}\n", v.name); }
        }
        push!(buf, " * @returns {{{}}}\n */\n",
            goal.output.as_deref().unwrap_or("number"));

        // シグネチャ（JS は型なし）
        let params: Vec<String> = if !goal.inputs.is_empty() {
            goal.inputs.iter().map(|p| p.name.clone()).collect()
        } else {
            vars.iter().map(|v| v.name.clone()).collect()
        };
        push!(buf, "function {}({}) {{\n", fn_name, params.join(", "));

        // 事前条件チェック
        if !vars.is_empty() {
            push!(buf, "  // ── Precondition checks ───────────────────────────────\n");
            for v in &vars {
                for c in &v.constraints {
                    push!(buf, "{}", js_check(&v.name, c, "  "));
                }
            }
            push!(buf, "\n");
        }

        // When ブロック（条件付き事後条件）
        let when_block = render_when_postconds(goal, &TargetLang::JavaScript, "  ");
        if !when_block.is_empty() {
            push!(buf, "{when_block}\n");
        }

        // 通常の事後条件 TODO
        if ensures.is_empty() && when_block.is_empty() {
            push!(buf, "  throw new Error('Implement {fn_name}');\n");
        } else if !ensures.is_empty() {
            push!(buf, "  // TODO: Postconditions を満たす実装を書いてください\n");
            for e in &ensures { push!(buf, "  // ensure: {e}\n"); }
            push!(buf, "  throw new Error('Implement {fn_name}: ensure {}');\n", ensures[0]);
        }
        push!(buf, "}}\n\nmodule.exports = {{ {fn_name} }};\n");
        buf.push_str(&emit_examples(goal, &TargetLang::JavaScript));
        buf
    }

    // =======================================================================
    // TypeScript
    // =======================================================================

    fn gen_typescript(&self, goal: &ContractualGoal) -> String {
        let mut buf = String::new();
        let vars    = extract_variables(&goal.preconditions);
        let fn_name = to_camel_case(&goal.name);
        let ensures = postcond_strings(goal);

        push!(buf, "// Generated by Hammurabi — do not edit manually.\n");
        push!(buf, "// ContractualGoal: {}\n\n", goal.name);

        // TSDoc
        push!(buf, "/**\n * Contract: {}\n *\n", goal.name);
        push!(buf, " * @description\n * Preconditions:\n");
        for pre in &goal.preconditions { push!(buf, " *   - {pre}\n"); }
        push!(buf, " * Postconditions:\n");
        for post in &goal.postconditions { push!(buf, " *   - {}\n", post); }
        if !goal.inputs.is_empty() {
            for p in &goal.inputs {
                let ts_type = translate_type(&p.type_str, &TargetLang::TypeScript);
                push!(buf, " * @param {} - {}\n", p.name, ts_type);
            }
        } else {
            for v in &vars { let vn = &v.name; push!(buf, " * @param {vn} - satisfies constraints from ContractualGoal\n"); }
        }
        let ret_ts = goal.output.as_deref()
            .map(|t| translate_type(t, &TargetLang::TypeScript))
            .unwrap_or_else(|| "number".into());
        push!(buf, " * @returns {ret_ts}\n */\n");

        // シグネチャ（型付き）
        let params: Vec<String> = if !goal.inputs.is_empty() {
            goal.inputs.iter()
                .map(|p| format!("{}: {}", p.name, translate_type(&p.type_str, &TargetLang::TypeScript)))
                .collect()
        } else {
            vars.iter().map(|v| format!("{}: number", v.name)).collect()
        };
        push!(buf, "export function {}({}): {} {{\n", fn_name, params.join(", "), ret_ts);

        // 事前条件チェック
        if !vars.is_empty() {
            push!(buf, "  // ── Precondition checks ───────────────────────────────\n");
            for v in &vars {
                for c in &v.constraints {
                    push!(buf, "{}", ts_check(&v.name, c, "  "));
                }
            }
            push!(buf, "\n");
        }

        // When ブロック（条件付き事後条件）
        let when_block = render_when_postconds(goal, &TargetLang::TypeScript, "  ");
        if !when_block.is_empty() {
            push!(buf, "{when_block}\n");
        }

        // 通常の事後条件 TODO
        if ensures.is_empty() && when_block.is_empty() {
            push!(buf, "  throw new Error('Implement {fn_name}');\n");
        } else if !ensures.is_empty() {
            push!(buf, "  // TODO: Postconditions を満たす実装を書いてください\n");
            for e in &ensures { push!(buf, "  // ensure: {e}\n"); }
            push!(buf, "  throw new Error('Implement {fn_name}: ensure {}');\n", ensures[0]);
        }
        push!(buf, "}}\n");
        buf.push_str(&emit_examples(goal, &TargetLang::TypeScript));
        buf
    }
}

// ---------------------------------------------------------------------------
// 命名変換ヘルパー
// ---------------------------------------------------------------------------

/// `SafeDivision` / `safe division` → `safe_division`
fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch == ' ' || ch == '-' {
            out.push('_');
        } else if ch.is_uppercase() && i > 0 {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

/// `safe_division` / `SafeDivision` → `safeDivision`（Go/Java/JS/TS 用）
fn to_camel_case(s: &str) -> String {
    let snake = to_snake_case(s);
    let mut out = String::new();
    let mut cap_next = false;
    for (i, ch) in snake.chars().enumerate() {
        if ch == '_' {
            cap_next = true;
        } else if cap_next {
            out.push(ch.to_ascii_uppercase());
            cap_next = false;
        } else if i == 0 {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `safe_division` → `SafeDivision`（Java クラス名用）
fn to_pascal_case(s: &str) -> String {
    let camel = to_camel_case(s);
    let mut chars = camel.chars();
    match chars.next() {
        None    => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

// ---------------------------------------------------------------------------
// 言語別 制約チェックコード生成
// ---------------------------------------------------------------------------
// 各関数は 1 制約 → 複数行のチェックコード文字列を返す。
// indent は行頭に付加するインデント文字列。

fn python_check(var: &str, constraint: &str, indent: &str) -> String {
    let mut out = String::new();
    if let Some((min, max)) = parse_in_range(constraint) {
        out.push_str(&format!(
            "{indent}if not ({min} <= {var} <= {max}):\n\
             {indent}    raise ValueError(f\"{var}={{{var}}} violates constraint: {min} <= {var} <= {max}\")\n"
        ));
    } else if constraint.contains("NonNull") {
        out.push_str(&format!(
            "{indent}if {var} is None:\n\
             {indent}    raise ValueError(\"{var} must not be None\")\n"
        ));
    } else if constraint.contains("NonEmpty") {
        out.push_str(&format!(
            "{indent}if not {var}:\n\
             {indent}    raise ValueError(\"{var} must not be empty\")\n"
        ));
    } else {
        out.push_str(&format!("{indent}# constraint: {constraint}\n"));
    }
    out
}

fn go_check(var: &str, constraint: &str, indent: &str) -> String {
    let mut out = String::new();
    if let Some((min, max)) = parse_in_range(constraint) {
        out.push_str(&format!(
            "{indent}if {var} < {min} || {var} > {max} {{\n\
             {indent}\treturn 0, fmt.Errorf(\"{var}=%d violates constraint: {min} <= {var} <= {max}\", {var})\n\
             {indent}}}\n"
        ));
    } else if constraint.contains("NonNull") {
        out.push_str(&format!(
            "{indent}if {var} == nil {{\n\
             {indent}\treturn nil, fmt.Errorf(\"{var} must not be nil\")\n\
             {indent}}}\n"
        ));
    } else if constraint.contains("NonEmpty") {
        out.push_str(&format!(
            "{indent}if len({var}) == 0 {{\n\
             {indent}\treturn \"\", fmt.Errorf(\"{var} must not be empty\")\n\
             {indent}}}\n"
        ));
    } else {
        out.push_str(&format!("{indent}// constraint: {constraint}\n"));
    }
    out
}

fn java_check(var: &str, constraint: &str, indent: &str) -> String {
    let mut out = String::new();
    if let Some((min, max)) = parse_in_range(constraint) {
        out.push_str(&format!(
            "{indent}if ({var} < {min}L || {var} > {max}L) {{\n\
             {indent}    throw new IllegalArgumentException(\n\
             {indent}        \"{var}=\" + {var} + \" violates constraint: {min} <= {var} <= {max}\");\n\
             {indent}}}\n"
        ));
    } else if constraint.contains("NonNull") {
        out.push_str(&format!(
            "{indent}Objects.requireNonNull({var}, \"{var} must not be null\");\n"
        ));
    } else if constraint.contains("NonEmpty") {
        out.push_str(&format!(
            "{indent}if ({var} == null || {var}.isEmpty()) {{\n\
             {indent}    throw new IllegalArgumentException(\"{var} must not be empty\");\n\
             {indent}}}\n"
        ));
    } else {
        out.push_str(&format!("{indent}// constraint: {constraint}\n"));
    }
    out
}

fn js_check(var: &str, constraint: &str, indent: &str) -> String {
    let mut out = String::new();
    if let Some((min, max)) = parse_in_range(constraint) {
        out.push_str(&format!(
            "{indent}if ({var} < {min} || {var} > {max}) {{\n\
             {indent}  throw new RangeError(`{var}=${{{var}}} violates constraint: {min} <= {var} <= {max}`);\n\
             {indent}}}\n"
        ));
    } else if constraint.contains("NonNull") {
        out.push_str(&format!(
            "{indent}if ({var} == null || {var} === undefined) {{\n\
             {indent}  throw new TypeError(`{var} must not be null or undefined`);\n\
             {indent}}}\n"
        ));
    } else if constraint.contains("NonEmpty") {
        out.push_str(&format!(
            "{indent}if (!{var} || {var}.length === 0) {{\n\
             {indent}  throw new RangeError(`{var} must not be empty`);\n\
             {indent}}}\n"
        ));
    } else {
        out.push_str(&format!("{indent}// constraint: {constraint}\n"));
    }
    out
}

/// TypeScript は JavaScript とほぼ同じ（型アノテーション付きのためそのまま再利用）
fn ts_check(var: &str, constraint: &str, indent: &str) -> String {
    js_check(var, constraint, indent)
}

/// `Constraint::InRange { min: N_i64, max: M_i64 }` から (N, M) を取り出す。
fn parse_in_range(constraint: &str) -> Option<(i64, i64)> {
    if !constraint.contains("InRange") { return None; }
    // "Constraint::InRange { min: -9223372036854775808_i64, max: 9223372036854775807_i64 }"
    let min = extract_i64(constraint, "min:")?;
    let max = extract_i64(constraint, "max:")?;
    Some((min, max))
}

fn extract_i64(s: &str, key: &str) -> Option<i64> {
    let after = s.split(key).nth(1)?.trim();
    // 数値部分（符号 + 数字）を取り出し、_i64 サフィックスを除去
    let num: String = after.chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    // 隣接する文字列（_i64 など）は無視して先頭の数字列だけ parse
    // まずサフィックス除去: "9223372036854775807_i64" → "9223372036854775807"
    let trimmed = num.trim_end_matches(|c: char| c == '_' || c.is_alphabetic());
    trimmed.parse().ok()
}

// ---------------------------------------------------------------------------
// predicate_to_expr — Predicate を言語別ランタイム式に変換
// ---------------------------------------------------------------------------

/// `Predicate` を指定言語のブール式文字列に変換する。
/// `When(cond, cons)` の条件節・帰結節をコード生成するために使う。
pub fn predicate_to_expr(pred: &Predicate, lang: &TargetLang) -> String {
    match pred {
        Predicate::True  => match lang {
            TargetLang::Python => "True".into(),
            _ => "true".into(),
        },
        Predicate::False => match lang {
            TargetLang::Python => "False".into(),
            _ => "false".into(),
        },
        Predicate::Atom(s) => s.clone(),

        Predicate::InRange { var, min, max } => match lang {
            TargetLang::Python =>
                format!("{min} <= {var} <= {max}"),
            _ =>
                format!("{var} >= {min} && {var} <= {max}"),
        },

        Predicate::NonNull(var) => match lang {
            TargetLang::Rust       => format!("{var}.is_some()"),
            TargetLang::Python     => format!("{var} is not None"),
            TargetLang::Go         => format!("{var} != nil"),
            TargetLang::Java       => format!("{var} != null"),
            TargetLang::JavaScript |
            TargetLang::TypeScript => format!("{var} !== null && {var} !== undefined"),
        },

        Predicate::Equals(a, b) => match lang {
            TargetLang::JavaScript |
            TargetLang::TypeScript => format!("{a} === {b}"),
            _ => format!("{a} == {b}"),
        },

        Predicate::Not(p) => {
            let inner = predicate_to_expr(p, lang);
            match lang {
                TargetLang::Python => format!("not ({inner})"),
                _ => format!("!({inner})"),
            }
        },

        Predicate::And(l, r) => {
            let le = predicate_to_expr(l, lang);
            let re = predicate_to_expr(r, lang);
            match lang {
                TargetLang::Python => format!("({le} and {re})"),
                _ => format!("({le} && {re})"),
            }
        },

        Predicate::Or(l, r) => {
            let le = predicate_to_expr(l, lang);
            let re = predicate_to_expr(r, lang);
            match lang {
                TargetLang::Python => format!("({le} or {re})"),
                _ => format!("({le} || {re})"),
            }
        },

        // Implies をネストで使う場合: !(cond) || cons
        Predicate::Implies(cond, cons) => {
            let ce = predicate_to_expr(cond, lang);
            let ke = predicate_to_expr(cons, lang);
            match lang {
                TargetLang::Python => format!("(not ({ce}) or {ke})"),
                _ => format!("(!({ce}) || {ke})"),
            }
        },

        Predicate::ForAll { var, body } => {
            let be = predicate_to_expr(body, lang);
            format!("/* ∀{var}. {be} */")
        },
        Predicate::Exists { var, body } => {
            let be = predicate_to_expr(body, lang);
            format!("/* ∃{var}. {be} */")
        },
    }
}

// ---------------------------------------------------------------------------
// render_when_postconds — When ブロックのコード出力
// ---------------------------------------------------------------------------

/// 事後条件の中から `Implies(cond, cons)` を抽出し、言語別の条件付きブロックを生成する。
/// `When` なしの goal では空文字列を返す。
fn render_when_postconds(goal: &ContractualGoal, lang: &TargetLang, indent: &str) -> String {
    let when_posts: Vec<(&Predicate, &Predicate)> = goal.postconditions
        .iter()
        .filter_map(|p| {
            if let Predicate::Implies(cond, cons) = p {
                Some((cond.as_ref(), cons.as_ref()))
            } else {
                None
            }
        })
        .collect();

    if when_posts.is_empty() {
        return String::new();
    }

    let mut buf = String::new();
    push!(buf, "{indent}// ── When: 条件付き事後条件 ────────────────────────────────\n");

    for (cond, cons) in &when_posts {
        let cond_expr = predicate_to_expr(cond, lang);
        let cons_str  = cons.to_string();

        match lang {
            TargetLang::Python => {
                push!(buf, "{indent}if {cond_expr}:\n");
                push!(buf, "{indent}    # TODO: ensure {cons_str}\n");
                push!(buf, "{indent}    raise NotImplementedError(\"ensure {cons_str}\")\n");
            }
            TargetLang::Go => {
                push!(buf, "{indent}if {cond_expr} {{\n");
                push!(buf, "{indent}\t// TODO: ensure {cons_str}\n");
                push!(buf, "{indent}\treturn 0, fmt.Errorf(\"not implemented: ensure {cons_str}\")\n");
                push!(buf, "{indent}}}\n");
            }
            TargetLang::Java => {
                push!(buf, "{indent}if ({cond_expr}) {{\n");
                push!(buf, "{indent}    // TODO: ensure {cons_str}\n");
                push!(buf, "{indent}    throw new UnsupportedOperationException(\"ensure {cons_str}\");\n");
                push!(buf, "{indent}}}\n");
            }
            TargetLang::JavaScript | TargetLang::TypeScript => {
                push!(buf, "{indent}if ({cond_expr}) {{\n");
                push!(buf, "{indent}  // TODO: ensure {cons_str}\n");
                push!(buf, "{indent}  throw new Error('not implemented: ensure {cons_str}');\n");
                push!(buf, "{indent}}}\n");
            }
            TargetLang::Rust => {
                push!(buf, "{indent}if {cond_expr} {{\n");
                push!(buf, "{indent}    // TODO: ensure {cons_str}\n");
                push!(buf, "{indent}    todo!(\"ensure {cons_str}\")\n");
                push!(buf, "{indent}}}\n");
            }
        }
    }
    buf
}

// ---------------------------------------------------------------------------
// postcond_strings — 非 When 事後条件のみを文字列化
// ---------------------------------------------------------------------------

/// 事後条件のうち `Implies`（When）以外を文字列リストで返す。
/// `When` ブロックは `render_when_postconds` が担当するため除外する。
fn postcond_strings(goal: &ContractualGoal) -> Vec<String> {
    goal.postconditions
        .iter()
        .filter(|p| !matches!(p, Predicate::Implies(_, _)))
        .map(|p| p.to_string())
        .collect()
}

/// `ForbiddenPattern` の日本語説明を返す。
fn forbidden_description(fp: &ForbiddenPattern) -> &'static str {
    match fp {
        ForbiddenPattern::NonExhaustiveBranch  => "全分岐を型レベルで網羅すること",
        ForbiddenPattern::RuntimeNullCheck     => "NonNull は型システムで証明すること",
        ForbiddenPattern::ImplicitCoercion     => "暗黙の型強制を禁止",
        ForbiddenPattern::UnprovenUnwrap       => "unwrap/expect は証明なしに使用禁止",
        ForbiddenPattern::CatchAllSuppression  => "_ パターンによるロジック隠蔽を禁止",
    }
}

// ---------------------------------------------------------------------------
// emit_examples — 言語別テストコード生成
// ---------------------------------------------------------------------------

/// examples を言語に合わせたテストコードとして出力する。
fn emit_examples(goal: &ContractualGoal, lang: &TargetLang) -> String {
    if goal.examples.is_empty() { return String::new(); }
    let mut buf = String::new();
    match lang {
        TargetLang::Rust       => emit_examples_rust(goal, &mut buf),
        TargetLang::Python     => emit_examples_python(goal, &mut buf),
        TargetLang::Go         => emit_examples_go(goal, &mut buf),
        TargetLang::Java       => emit_examples_java(goal, &mut buf),
        TargetLang::JavaScript => emit_examples_js(goal, &mut buf),
        TargetLang::TypeScript => emit_examples_ts(goal, &mut buf),
    }
    buf
}

fn emit_examples_rust(goal: &ContractualGoal, buf: &mut String) {
    let fn_name = to_snake_case(&goal.name);
    push!(buf, "\n#[cfg(test)]\nmod tests {{\n    use super::*;\n\n");
    for (i, ex) in goal.examples.iter().enumerate() {
        let test_name = if let Some(ref lbl) = ex.label {
            format!("example_{}", to_snake_case(lbl))
        } else {
            format!("example_{}", i + 1)
        };
        push!(buf, "    #[test]\n    fn {test_name}() {{\n");
        if let Some(ref lbl) = ex.label {
            push!(buf, "        // {lbl}\n");
        }
        push!(buf, "        let result = {fn_name}({});\n", ex.inputs_raw);
        push!(buf, "        assert_eq!(result, {});\n", ex.output_raw);
        push!(buf, "    }}\n\n");
    }
    push!(buf, "}}\n");
}

fn emit_examples_python(goal: &ContractualGoal, buf: &mut String) {
    let fn_name = to_snake_case(&goal.name);
    push!(buf, "\n\n# ── pytest examples ─────────────────────────────────\n");
    for (i, ex) in goal.examples.iter().enumerate() {
        let test_name = if let Some(ref lbl) = ex.label {
            format!("test_{}_{}", fn_name, to_snake_case(lbl))
        } else {
            format!("test_{}_example_{}", fn_name, i + 1)
        };
        push!(buf, "\ndef {test_name}():\n");
        if let Some(ref lbl) = ex.label {
            push!(buf, "    # {lbl}\n");
        }
        push!(buf, "    assert {fn_name}({}) == {}\n", ex.inputs_raw, ex.output_raw);
    }
}

fn emit_examples_go(goal: &ContractualGoal, buf: &mut String) {
    let fn_name = to_camel_case(&goal.name);
    push!(buf, "\n// ── Examples (Go test) ─────────────────────────────\n");
    push!(buf, "func Test{}Examples(t *testing.T) {{\n", to_pascal_case(&goal.name));
    for (i, ex) in goal.examples.iter().enumerate() {
        let default_label = format!("example {}", i + 1);
        let label = ex.label.as_deref().unwrap_or(&default_label);
        push!(buf, "    t.Run(\"{label}\", func(t *testing.T) {{\n");
        push!(buf, "        got, _ := {fn_name}({})\n", ex.inputs_raw);
        push!(buf, "        if got != {} {{ t.Errorf(\"%v != %v\", got, {}) }}\n",
            ex.output_raw, ex.output_raw);
        push!(buf, "    }})\n");
    }
    push!(buf, "}}\n");
}

fn emit_examples_java(goal: &ContractualGoal, buf: &mut String) {
    let fn_name   = to_camel_case(&goal.name);
    let class     = to_pascal_case(&goal.name);
    push!(buf, "\n// ── Examples (JUnit) ───────────────────────────────\n");
    push!(buf, "import org.junit.Test;\nimport static org.junit.Assert.*;\n\n");
    push!(buf, "public class {class}Test {{\n");
    for (i, ex) in goal.examples.iter().enumerate() {
        let test_name = if let Some(ref lbl) = ex.label {
            format!("example{}", to_pascal_case(lbl))
        } else {
            format!("example{}", i + 1)
        };
        push!(buf, "    @Test\n    public void {test_name}() {{\n");
        if let Some(ref lbl) = ex.label { push!(buf, "        // {lbl}\n"); }
        push!(buf, "        assertEquals({}, {fn_name}({}));\n",
            ex.output_raw, ex.inputs_raw);
        push!(buf, "    }}\n\n");
    }
    push!(buf, "}}\n");
}

fn emit_examples_js(goal: &ContractualGoal, buf: &mut String) {
    let fn_name = to_camel_case(&goal.name);
    push!(buf, "\n// ── Examples (Jest) ────────────────────────────────\n");
    push!(buf, "describe('{}', () => {{\n", goal.name);
    for (i, ex) in goal.examples.iter().enumerate() {
        let default_label = format!("example {}", i + 1);
        let label = ex.label.as_deref().unwrap_or(&default_label);
        push!(buf, "  test('{}', () => {{\n", label);
        push!(buf, "    expect({fn_name}({})).toEqual({});\n",
            ex.inputs_raw, ex.output_raw);
        push!(buf, "  }});\n");
    }
    push!(buf, "}});\n");
}

fn emit_examples_ts(goal: &ContractualGoal, buf: &mut String) {
    emit_examples_js(goal, buf);
}

// ---------------------------------------------------------------------------
// CodegenOutput — 生成されたソースコードの出力物
// ---------------------------------------------------------------------------

/// `CodeGenerator::generate` が返す出力物。
pub struct CodegenOutput {
    source: String,
    pub lang: TargetLang,
}

impl CodegenOutput {
    /// 生成されたソースコードの文字列を返す。
    pub fn source(&self) -> &str {
        &self.source
    }

    /// 対象言語に合ったファイル拡張子を返す（例: `"rs"`, `"py"`）。
    pub fn extension(&self) -> &'static str {
        self.lang.extension()
    }

    /// ファイルへ書き出す（wasm32 では利用不可）。
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::write(path, &self.source)
    }
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};

    fn safe_division_goal() -> ContractualGoal {
        ContractualGoal::new("safe_division")
            .require(Predicate::non_null("divisor"))
            .require(Predicate::in_range("divisor", 1, i64::MAX))
            .require(Predicate::in_range("dividend", i64::MIN, i64::MAX))
            .ensure(Predicate::atom("result_is_finite"))
            .ensure(Predicate::atom("result_within_i64_range"))
            .invariant(Predicate::atom("no_memory_aliasing"))
            .forbid(ForbiddenPattern::RuntimeNullCheck)
    }

    // ── Rust 生成テスト ──────────────────────────────────────────────────

    #[test]
    fn rust_generates_function_name() {
        let output = CodeGenerator::new().generate(&safe_division_goal());
        assert!(output.source().contains("pub fn safe_division<V>"));
    }

    #[test]
    fn rust_generates_doc_comment_sections() {
        let output = CodeGenerator::new().generate(&safe_division_goal());
        let src = output.source();
        assert!(src.contains("## Preconditions"));
        assert!(src.contains("## Postconditions"));
        assert!(src.contains("## Invariants"));
        assert!(src.contains("## Forbidden Patterns"));
    }

    #[test]
    fn rust_generates_logic_rail_binds() {
        let output = CodeGenerator::new().generate(&safe_division_goal());
        let src = output.source();
        assert!(src.contains("let divisor = LogicRail::bind("));
        assert!(src.contains("let dividend = LogicRail::bind("));
    }

    #[test]
    fn rust_generates_constraint_non_null() {
        let output = CodeGenerator::new().generate(&safe_division_goal());
        assert!(output.source().contains("Constraint::NonNull"));
    }

    #[test]
    fn rust_generates_constraint_in_range() {
        let src = CodeGenerator::new().generate(&safe_division_goal()).source().to_string();
        assert!(src.contains("Constraint::InRange { min: 1_i64"));
    }

    #[test]
    fn rust_generates_todo_with_postcondition() {
        let src = CodeGenerator::new().generate(&safe_division_goal()).source().to_string();
        assert!(src.contains("todo!(\"Implement safe_division: ensure result_is_finite\")"));
    }

    #[test]
    fn rust_generates_forbidden_pattern_comments() {
        let src = CodeGenerator::new().generate(&safe_division_goal()).source().to_string();
        assert!(src.contains("✗ RuntimeNullCheck"));
    }

    #[test]
    fn rust_compound_predicate_extracts_both_branches() {
        let goal = ContractualGoal::new("check")
            .require(Predicate::and(
                Predicate::non_null("x"),
                Predicate::in_range("x", 0, 100),
            ))
            .ensure(Predicate::atom("x_valid"));
        let src = CodeGenerator::new().generate(&goal).source().to_string();
        assert!(src.contains("let x = LogicRail::bind("));
        assert!(src.contains("Constraint::NonNull"));
        assert!(src.contains("Constraint::InRange { min: 0_i64, max: 100_i64 }"));
    }

    #[test]
    fn rust_extension_is_rs() {
        let output = CodeGenerator::new().generate(&safe_division_goal());
        assert_eq!(output.extension(), "rs");
    }

    // ── Python 生成テスト ────────────────────────────────────────────────

    #[test]
    fn python_generates_def_and_docstring() {
        let output = CodeGenerator::for_lang(TargetLang::Python).generate(&safe_division_goal());
        let src = output.source();
        println!("=== Python ===\n{src}");
        assert!(src.contains("def safe_division("));
        assert!(src.contains("Contract: safe_division"));
        assert!(src.contains("Preconditions:"));
    }

    #[test]
    fn python_generates_range_check() {
        let output = CodeGenerator::for_lang(TargetLang::Python).generate(&safe_division_goal());
        let src = output.source();
        assert!(src.contains("if not ("));
        assert!(src.contains("raise ValueError("));
    }

    #[test]
    fn python_generates_raise_not_implemented() {
        let src = CodeGenerator::for_lang(TargetLang::Python)
            .generate(&safe_division_goal()).source().to_string();
        assert!(src.contains("raise NotImplementedError("));
    }

    #[test]
    fn python_extension_is_py() {
        let output = CodeGenerator::for_lang(TargetLang::Python).generate(&safe_division_goal());
        assert_eq!(output.extension(), "py");
    }

    // ── Go 生成テスト ────────────────────────────────────────────────────

    #[test]
    fn go_generates_func_with_error_return() {
        let output = CodeGenerator::for_lang(TargetLang::Go).generate(&safe_division_goal());
        let src = output.source();
        println!("=== Go ===\n{src}");
        assert!(src.contains("func safeDivision("));
        assert!(src.contains("(int64, error)"));
        assert!(src.contains("fmt.Errorf("));
    }

    #[test]
    fn go_extension_is_go() {
        let output = CodeGenerator::for_lang(TargetLang::Go).generate(&safe_division_goal());
        assert_eq!(output.extension(), "go");
    }

    // ── Java 生成テスト ──────────────────────────────────────────────────

    #[test]
    fn java_generates_class_and_method() {
        let output = CodeGenerator::for_lang(TargetLang::Java).generate(&safe_division_goal());
        let src = output.source();
        println!("=== Java ===\n{src}");
        assert!(src.contains("public class SafeDivision"));
        assert!(src.contains("public static long safeDivision("));
        assert!(src.contains("throw new IllegalArgumentException("));
    }

    #[test]
    fn java_extension_is_java() {
        let output = CodeGenerator::for_lang(TargetLang::Java).generate(&safe_division_goal());
        assert_eq!(output.extension(), "java");
    }

    // ── JavaScript 生成テスト ────────────────────────────────────────────

    #[test]
    fn javascript_generates_jsdoc_and_function() {
        let output = CodeGenerator::for_lang(TargetLang::JavaScript).generate(&safe_division_goal());
        let src = output.source();
        println!("=== JavaScript ===\n{src}");
        assert!(src.contains("function safeDivision("));
        assert!(src.contains("@returns {number}"));
        assert!(src.contains("throw new RangeError("));
    }

    #[test]
    fn javascript_extension_is_js() {
        let output = CodeGenerator::for_lang(TargetLang::JavaScript).generate(&safe_division_goal());
        assert_eq!(output.extension(), "js");
    }

    // ── TypeScript 生成テスト ────────────────────────────────────────────

    #[test]
    fn typescript_generates_typed_function() {
        let output = CodeGenerator::for_lang(TargetLang::TypeScript).generate(&safe_division_goal());
        let src = output.source();
        println!("=== TypeScript ===\n{src}");
        assert!(src.contains("export function safeDivision("));
        assert!(src.contains("divisor: number"));
        assert!(src.contains("): number {"));
        assert!(src.contains("throw new RangeError("));
    }

    #[test]
    fn typescript_extension_is_ts() {
        let output = CodeGenerator::for_lang(TargetLang::TypeScript).generate(&safe_division_goal());
        assert_eq!(output.extension(), "ts");
    }

    // ── TargetLang FromStr テスト ────────────────────────────────────────

    #[test]
    fn target_lang_from_str() {
        use std::str::FromStr;
        assert_eq!(TargetLang::from_str("rust").unwrap(), TargetLang::Rust);
        assert_eq!(TargetLang::from_str("Python").unwrap(), TargetLang::Python);
        assert_eq!(TargetLang::from_str("go").unwrap(), TargetLang::Go);
        assert_eq!(TargetLang::from_str("java").unwrap(), TargetLang::Java);
        assert_eq!(TargetLang::from_str("js").unwrap(), TargetLang::JavaScript);
        assert_eq!(TargetLang::from_str("ts").unwrap(), TargetLang::TypeScript);
        assert!(TargetLang::from_str("cobol").is_err());
    }

    // ── 命名変換テスト ───────────────────────────────────────────────────

    #[test]
    fn naming_conversions() {
        assert_eq!(to_snake_case("SafeDivision"), "safe_division");
        assert_eq!(to_snake_case("safe division"), "safe_division");
        assert_eq!(to_camel_case("safe_division"), "safeDivision");
        assert_eq!(to_camel_case("SafeDivision"), "safeDivision");
        assert_eq!(to_pascal_case("safe_division"), "SafeDivision");
    }

    // ── ファイル書き出しテスト ───────────────────────────────────────────

    #[test]
    fn save_to_file_writes_rust_source() {
        let output = CodeGenerator::new().generate(&safe_division_goal());
        let path = std::env::temp_dir().join("hammurabi_codegen_test.rs");
        output.save_to_file(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("pub fn safe_division"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn save_to_file_writes_python_source() {
        let output = CodeGenerator::for_lang(TargetLang::Python).generate(&safe_division_goal());
        let path = std::env::temp_dir().join("hammurabi_codegen_test.py");
        output.save_to_file(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("def safe_division("));
        std::fs::remove_file(path).ok();
    }
}
