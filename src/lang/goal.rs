//! # ContractualGoal — AI への発注書
//!
//! 関数の「何をすべきか」を述語論理で記述する。命令形（どうやるか）は含まない。
//! これが hammurabi の「The What Interface」を実現する核心データ構造。

use std::fmt;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Predicate — 一階述語論理の AST
// ---------------------------------------------------------------------------

/// 述語論理を表す再帰的な AST ノード。
/// `if` 文の代わりにこの木構造でロジックの全分岐を記述する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Predicate {
    /// 恒真
    True,
    /// 恒偽
    False,
    /// 名前付きアトム述語（例: "is_positive", "is_utf8"）
    Atom(String),
    /// 論理否定
    Not(Box<Predicate>),
    /// 論理積
    And(Box<Predicate>, Box<Predicate>),
    /// 論理和
    Or(Box<Predicate>, Box<Predicate>),
    /// 含意: p → q
    Implies(Box<Predicate>, Box<Predicate>),
    /// 全称量化: ∀ var. body
    ForAll { var: String, body: Box<Predicate> },
    /// 存在量化: ∃ var. body
    Exists { var: String, body: Box<Predicate> },
    /// 整数範囲制約: min ≤ var ≤ max
    InRange { var: String, min: i64, max: i64 },
    /// 非 Null 制約
    NonNull(String),
    /// 等値制約
    Equals(String, String),
}

impl Predicate {
    /// ショートハンドコンストラクタ群
    pub fn atom(s: impl Into<String>) -> Self {
        Self::Atom(s.into())
    }
    pub fn not(p: Predicate) -> Self {
        Self::Not(Box::new(p))
    }
    pub fn and(l: Predicate, r: Predicate) -> Self {
        Self::And(Box::new(l), Box::new(r))
    }
    pub fn or(l: Predicate, r: Predicate) -> Self {
        Self::Or(Box::new(l), Box::new(r))
    }
    pub fn implies(ante: Predicate, cons: Predicate) -> Self {
        Self::Implies(Box::new(ante), Box::new(cons))
    }
    pub fn for_all(var: impl Into<String>, body: Predicate) -> Self {
        Self::ForAll { var: var.into(), body: Box::new(body) }
    }
    pub fn exists(var: impl Into<String>, body: Predicate) -> Self {
        Self::Exists { var: var.into(), body: Box::new(body) }
    }
    pub fn in_range(var: impl Into<String>, min: i64, max: i64) -> Self {
        Self::InRange { var: var.into(), min, max }
    }
    pub fn non_null(var: impl Into<String>) -> Self {
        Self::NonNull(var.into())
    }
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Predicate::True  => write!(f, "⊤"),
            Predicate::False => write!(f, "⊥"),
            Predicate::Atom(s) => write!(f, "{s}"),
            Predicate::Not(p)  => write!(f, "¬({p})"),
            Predicate::And(l, r) => write!(f, "({l} ∧ {r})"),
            Predicate::Or(l, r)  => write!(f, "({l} ∨ {r})"),
            Predicate::Implies(a, c) => write!(f, "({a} → {c})"),
            Predicate::ForAll { var, body } => write!(f, "∀{var}. {body}"),
            Predicate::Exists { var, body } => write!(f, "∃{var}. {body}"),
            Predicate::InRange { var, min, max } => write!(f, "{min} ≤ {var} ≤ {max}"),
            Predicate::NonNull(v) => write!(f, "NonNull({v})"),
            Predicate::Equals(a, b) => write!(f, "{a} = {b}"),
        }
    }
}

// ---------------------------------------------------------------------------
// ForbiddenPattern — 「憲法」違反パターンの列挙
// Zero-Ambiguity 原則を具体化する
// ---------------------------------------------------------------------------

/// コンパイラが検出・拒絶するべきパターン群。
/// if 文の場当たり的な例外処理、未網羅な分岐などが対象。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ForbiddenPattern {
    /// 網羅されていない分岐（全パターンが型レベルで証明されていないマッチ）
    NonExhaustiveBranch,
    /// 実行時 Null チェック（型システムで NonNull を証明すべき）
    RuntimeNullCheck,
    /// 暗黙の型強制
    ImplicitCoercion,
    /// 証明なしの unwrap/expect（パニック可能性）
    UnprovenUnwrap,
    /// Catch-all パターンによるロジックの隠蔽
    CatchAllSuppression,
}

impl fmt::Display for ForbiddenPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonExhaustiveBranch  => write!(f, "NonExhaustiveBranch"),
            Self::RuntimeNullCheck     => write!(f, "RuntimeNullCheck"),
            Self::ImplicitCoercion     => write!(f, "ImplicitCoercion"),
            Self::UnprovenUnwrap       => write!(f, "UnprovenUnwrap"),
            Self::CatchAllSuppression  => write!(f, "CatchAllSuppression"),
        }
    }
}

// ---------------------------------------------------------------------------
// ContractualGoal — AI への発注書
// ---------------------------------------------------------------------------

/// 関数が満たすべき「契約」を述語論理で記述した構造体。
///
/// hammurabi では、この `ContractualGoal` が実装の代わりに先に定義され、
/// AI（またはコンパイラ）がこの契約を満たす実装を生成・検証する。
///
/// # 設計思想
/// - `preconditions` : 呼び出し元が保証する入力の性質
/// - `postconditions`: 実装が保証しなければならない出力の性質  
// ---------------------------------------------------------------------------
// Param — 関数の入力パラメータ（名前 + 型）
// ---------------------------------------------------------------------------

/// 関数シグネチャの入力パラメータ。
/// `inputs: dividend: i32, divisor: i32` でパースされ、コード生成に使われる。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Param {
    /// パラメータ名（例: `dividend`）
    pub name:     String,
    /// Rust スタイルの型文字列（例: `i32`, `Option<String>`, `Vec<u8>`）
    pub type_str: String,
}

impl Param {
    pub fn new(name: impl Into<String>, type_str: impl Into<String>) -> Self {
        Self { name: name.into(), type_str: type_str.into() }
    }
}

// ---------------------------------------------------------------------------
// Example — 具体的な入出力例
// ---------------------------------------------------------------------------

/// `examples:` フィールドに書く具体的な入出力ペア。
///
/// ```text
/// examples: [
///   - (10, 2)   => Ok(5)          // 正常割り算
///   - (7, 0)    => Err("ゼロ除算")
///   - "負の値": (-6, 3) => Ok(-2)  // 任意のラベル付き
/// ]
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Example {
    /// 任意のラベル（`"正常割り算":` の形式）
    pub label:      Option<String>,
    /// 入力引数部分の生文字列（例: `10, 2` / `dividend: 10, divisor: 2` / `10`）
    pub inputs_raw: String,
    /// 期待される出力の生文字列（例: `Ok(5)` / `5` / `Err("ゼロ除算")`）
    pub output_raw: String,
}

impl Example {
    pub fn new(
        label:      Option<String>,
        inputs_raw: impl Into<String>,
        output_raw: impl Into<String>,
    ) -> Self {
        Self { label, inputs_raw: inputs_raw.into(), output_raw: output_raw.into() }
    }
}

// ---------------------------------------------------------------------------
// ContractualGoal
// ---------------------------------------------------------------------------

/// - `invariants`    : 実行中ずっと成立しなければならない不変条件
/// - `forbidden`     : AIが生成したコードに含んではならないパターン
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractualGoal {
    pub name:           String,
    /// goal の一意識別子（依存グラフのノードキー）。`id: safe_divide_v1` で指定。
    pub id:             Option<String>,
    /// AI モデルバージョン固定（再現性の基盤）。`model: gpt-4o@2024-05-13` で指定。
    pub model_pin:      Option<String>,
    /// 関数の入力パラメータ。指定された場合はコード生成のシグネチャに使われる。
    pub inputs:         Vec<Param>,
    /// 関数の返り値型（Rust スタイル）。指定された場合はコード生成の戻り値に使われる。
    pub output:         Option<String>,
    /// 具体的な入出力例。テストコード生成と AI プロンプトに使われる。
    pub examples:       Vec<Example>,
    pub preconditions:  Vec<Predicate>,
    pub postconditions: Vec<Predicate>,
    pub invariants:     Vec<Predicate>,
    pub forbidden:      Vec<ForbiddenPattern>,
}

impl ContractualGoal {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name:           name.into(),
            id:             None,
            model_pin:      None,
            inputs:         Vec::new(),
            output:         None,
            examples:       Vec::new(),
            preconditions:  Vec::new(),
            postconditions: Vec::new(),
            invariants:     Vec::new(),
            // Zero-Ambiguity 原則のデフォルト禁止セット
            forbidden: vec![
                ForbiddenPattern::NonExhaustiveBranch,
                ForbiddenPattern::UnprovenUnwrap,
                ForbiddenPattern::CatchAllSuppression,
            ],
        }
    }

    /// 入力パラメータを追加するビルダーメソッド
    pub fn with_input(mut self, name: impl Into<String>, type_str: impl Into<String>) -> Self {
        self.inputs.push(Param::new(name, type_str));
        self
    }

    /// 返り値型を設定するビルダーメソッド
    pub fn with_output(mut self, type_str: impl Into<String>) -> Self {
        self.output = Some(type_str.into());
        self
    }

    pub fn require(mut self, pre: Predicate) -> Self {
        self.preconditions.push(pre);
        self
    }

    pub fn ensure(mut self, post: Predicate) -> Self {
        self.postconditions.push(post);
        self
    }

    pub fn invariant(mut self, inv: Predicate) -> Self {
        self.invariants.push(inv);
        self
    }

    pub fn forbid(mut self, pattern: ForbiddenPattern) -> Self {
        if !self.forbidden.contains(&pattern) {
            self.forbidden.push(pattern);
        }
        self
    }

    /// 全ての事後条件が事前条件のもとで意味を持つか静的チェック（形式検証は Verifier に委ねる）
    pub fn is_well_formed(&self) -> bool {
        !self.postconditions.is_empty()
    }
}

impl fmt::Display for ContractualGoal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "ContractualGoal: {}", self.name)?;
        if let Some(ref id) = self.id {
            writeln!(f, "  id        : {id}")?;
        }
        if let Some(ref model) = self.model_pin {
            writeln!(f, "  model     : {model}")?;
        }
        if !self.inputs.is_empty() {
            let params = self.inputs.iter()
                .map(|p| format!("{}: {}", p.name, p.type_str))
                .collect::<Vec<_>>().join(", ");
            writeln!(f, "  inputs   : ({params})")?;
        }
        if let Some(ref out) = self.output {
            writeln!(f, "  output   : {out}")?;
        }
        if !self.examples.is_empty() {
            writeln!(f, "  examples :")?;
            for ex in &self.examples {
                let label = ex.label.as_deref().map(|l| format!("{l}: ")).unwrap_or_default();
                writeln!(f, "    - {}({}) => {}", label, ex.inputs_raw, ex.output_raw)?;
            }
        }
        writeln!(f, "  require  : {}", self.preconditions.iter()
            .map(|p| p.to_string()).collect::<Vec<_>>().join(" ∧ "))?;
        writeln!(f, "  ensure   : {}", self.postconditions.iter()
            .map(|p| p.to_string()).collect::<Vec<_>>().join(" ∧ "))?;
        writeln!(f, "  invariant: {}", self.invariants.iter()
            .map(|p| p.to_string()).collect::<Vec<_>>().join(" ∧ "))?;
        write!(f, "  forbidden: {}", self.forbidden.iter()
            .map(|p| p.to_string()).collect::<Vec<_>>().join(", "))
    }
}
