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
/// - `invariants`    : 実行中ずっと成立しなければならない不変条件
/// - `forbidden`     : AIが生成したコードに含んではならないパターン
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractualGoal {
    pub name:           String,
    pub preconditions:  Vec<Predicate>,
    pub postconditions: Vec<Predicate>,
    pub invariants:     Vec<Predicate>,
    pub forbidden:      Vec<ForbiddenPattern>,
}

impl ContractualGoal {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name:           name.into(),
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
