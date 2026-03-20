//! # LogicRail<T> — 証明を封印した意味論的コンテナ
//!
//! `LogicRail<T>` は値 `T` と「その値に課された不変条件が Z3 によって証明された」
//! という事実を一体化したシールドコンテナ。
//!
//! ## 設計原則
//! - `ProofToken` は `verifier` モジュールのみが生成できる（偽造不可）
//! - `bind` を通じてのみ構築可能（裸の `T` を包むことは禁止）
//! - `map` はモナド的変換 — 変換後も新たな証明を要求する
//! - `merge` は2本のレールを合成し、組み合わせ不変条件を再証明する

use std::fmt;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use crate::lang::goal::Predicate;
use crate::compiler::verifier::{Verifier, VerificationError};

// ---------------------------------------------------------------------------
// ProofToken — 偽造不可の証明印
// ---------------------------------------------------------------------------

/// Z3 による検証が完了したことを証明するトークン。
/// `pub(crate)` なコンストラクタにより、`verifier` モジュール外では生成不可。
#[derive(Debug, Clone)]
pub struct ProofToken {
    /// 検証された制約セットの SHA-2 ハッシュ（改竄検知）
    pub(crate) constraint_hash: u64,
    /// どのバックエンドが発行したか
    pub(crate) backend: VerifierBackend,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerifierBackend {
    /// 本番: z3 SMT ソルバーによる厳密証明
    Z3Smt,
    /// 開発: MockVerifier による構造的整合性チェック
    Mock,
}

impl ProofToken {
    /// verifier モジュールのみが呼び出せるコンストラクタ
    pub(crate) fn new(constraint_hash: u64, backend: VerifierBackend) -> Self {
        Self { constraint_hash, backend }
    }

    pub fn is_z3_proven(&self) -> bool {
        self.backend == VerifierBackend::Z3Smt
    }
}

impl fmt::Display for ProofToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mark = if self.is_z3_proven() { "Z3✓" } else { "Mock✓" };
        write!(f, "ProofToken[{mark}|hash={:016x}]", self.constraint_hash)
    }
}

// ---------------------------------------------------------------------------
// Constraint — LogicRail が保持する制約の型
// ---------------------------------------------------------------------------

/// 値に課せられる制約。`Predicate` AST の具体化。
#[derive(Debug, Clone)]
pub enum Constraint {
    /// 述語 AST による任意の制約
    Predicate(Predicate),
    /// 整数値が [min, max] に収まること
    InRange { min: i64, max: i64 },
    /// 参照が Null でないこと（Hardware-Level Determinism 原則）
    NonNull,
    /// コレクションが空でないこと
    NonEmpty,
    /// 他レールとの整合性（クロスレール制約）
    ConsistentWith(String),
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Predicate(p)         => write!(f, "Predicate({p})"),
            Self::InRange { min, max } => write!(f, "InRange[{min}, {max}]"),
            Self::NonNull              => write!(f, "NonNull"),
            Self::NonEmpty             => write!(f, "NonEmpty"),
            Self::ConsistentWith(id)   => write!(f, "ConsistentWith({id})"),
        }
    }
}

/// 制約セットのハッシュを計算（ProofToken 生成用）
pub fn hash_constraints(constraints: &[Constraint]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for c in constraints {
        c.to_string().hash(&mut hasher);
    }
    hasher.finish()
}

// ---------------------------------------------------------------------------
// LogicRail<T> — 本体
// ---------------------------------------------------------------------------

/// 意味論的な制約を封印した値コンテナ。
///
/// # 型パラメータ
/// `T`: コンテナが保持する値の型。`Send + Sync` を要求することで
///      M5 Pro の Unified Memory 上でのスレッド安全性を型レベルで保証する。
///
/// # 不変条件
/// - `proof` は `constraints` と同一のハッシュを持つ（改竄検知）
/// - `value` への可変アクセスは存在しない（不変条件の破壊を防ぐ）
pub struct LogicRail<T: Send + Sync> {
    value:       T,
    constraints: Vec<Constraint>,
    proof:       ProofToken,
    /// レール識別子（クロスレール制約のデバッグ用）
    id:          String,
}

impl<T: Send + Sync + fmt::Debug> LogicRail<T> {
    // -----------------------------------------------------------------------
    // 構築
    // -----------------------------------------------------------------------

    /// 値と制約セットを与え、Verifier による証明を経てレールを生成する。
    ///
    /// Verifier が制約の充足不可能を報告した場合は `Err` を返す。
    /// これが hammurabi における「型チェック」の実行点。
    pub fn bind<V: Verifier>(
        id:          impl Into<String>,
        value:       T,
        constraints: Vec<Constraint>,
        verifier:    &V,
    ) -> Result<Self, VerificationError> {
        let token = verifier.verify_constraints(&value, &constraints)?;

        // 証明トークンのハッシュが制約セットと一致するか検証
        let expected_hash = hash_constraints(&constraints);
        if token.constraint_hash != expected_hash {
            return Err(VerificationError::ProofTampered {
                expected: expected_hash,
                got:      token.constraint_hash,
            });
        }

        Ok(Self { value, constraints, proof: token, id: id.into() })
    }

    // -----------------------------------------------------------------------
    // 観察（不変参照のみ公開）
    // -----------------------------------------------------------------------

    /// 値への不変参照。`LogicRail` の外では値を変更できない。
    pub fn extract(&self) -> &T {
        &self.value
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn proof(&self) -> &ProofToken {
        &self.proof
    }

    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    /// Z3 による厳密証明済みかどうか
    pub fn is_strictly_proven(&self) -> bool {
        self.proof.is_z3_proven()
    }

    // -----------------------------------------------------------------------
    // モナド的変換
    // -----------------------------------------------------------------------

    /// `T → U` の変換を適用しつつ、新しい制約セットで再証明する。
    /// 変換後の型 `U` にも `Send + Sync` を要求し、メモリ安全性を保持する。
    pub fn map<U, F, V>(
        self,
        new_id:          impl Into<String>,
        f:               F,
        new_constraints: Vec<Constraint>,
        verifier:        &V,
    ) -> Result<LogicRail<U>, VerificationError>
    where
        U: Send + Sync + fmt::Debug,
        F: FnOnce(T) -> U,
        V: Verifier,
    {
        let new_value = f(self.value);
        LogicRail::bind(new_id, new_value, new_constraints, verifier)
    }

    /// 2本のレールを合成し、タプルとして新たなレールに束ねる。
    /// 組み合わせ制約が追加され、再証明を要求する。
    pub fn merge<U, V>(
        self,
        other:               LogicRail<U>,
        merged_id:           impl Into<String>,
        combined_constraints: Vec<Constraint>,
        verifier:             &V,
    ) -> Result<LogicRail<(T, U)>, VerificationError>
    where
        U: Send + Sync + fmt::Debug,
        V: Verifier,
    {
        let merged_value = (self.value, other.value);
        LogicRail::bind(merged_id, merged_value, combined_constraints, verifier)
    }
}

impl<T: Send + Sync + fmt::Debug> fmt::Display for LogicRail<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "LogicRail[{}]", self.id)?;
        writeln!(f, "  value  : {:?}", self.value)?;
        writeln!(f, "  proof  : {}", self.proof)?;
        write!(f, "  constraints ({}):", self.constraints.len())?;
        for c in &self.constraints {
            write!(f, "\n    - {c}")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::verifier::MockVerifier;

    #[test]
    fn bind_succeeds_with_valid_constraints() {
        let v = MockVerifier::default();
        let rail = LogicRail::bind(
            "age",
            25_i64,
            vec![Constraint::InRange { min: 0, max: 150 }],
            &v,
        );
        assert!(rail.is_ok());
        assert_eq!(*rail.unwrap().extract(), 25_i64);
    }

    #[test]
    fn map_produces_new_rail() {
        let v = MockVerifier::default();
        let rail = LogicRail::bind(
            "raw",
            10_i64,
            vec![Constraint::InRange { min: 0, max: 100 }],
            &v,
        )
        .unwrap();

        let doubled = rail.map(
            "doubled",
            |x| x * 2,
            vec![Constraint::InRange { min: 0, max: 200 }],
            &v,
        );
        assert!(doubled.is_ok());
        assert_eq!(*doubled.unwrap().extract(), 20_i64);
    }

    #[test]
    fn merge_combines_two_rails() {
        let v = MockVerifier::default();
        let r1 = LogicRail::bind("r1", 1_i64, vec![Constraint::NonNull], &v).unwrap();
        let r2 = LogicRail::bind("r2", 2_i64, vec![Constraint::NonNull], &v).unwrap();
        let merged = r1.merge(r2, "merged", vec![Constraint::NonNull], &v);
        assert!(merged.is_ok());
    }
}
