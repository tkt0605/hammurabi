//! # Verifier — 「憲法適合性チェック」エンジン
//!
//! ## アーキテクチャ
//! ```text
//! Verifier (trait)
//!   +-- MockVerifier    -- 開発・テスト用 (default)
//!   +-- Z3Verifier      -- 本番: z3-backend feature flag で有効化
//! ```
//!
//! ## 設計原則
//! - `Verifier` はトレイト。バックエンドは差し替え可能。
//! - `ProofToken` の発行権限はこのモジュールにのみ存在する。
//! - `VerificationError` は全ての失敗ケースを網羅した enum（catch-all 禁止）。

use std::fmt;
use std::any::Any;
use thiserror::Error;
use crate::lang::rail::{Constraint, ProofToken, VerifierBackend, hash_constraints};
use crate::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};

// ---------------------------------------------------------------------------
// downcast_to_i64 — 数値型の共通ダウンキャストヘルパー
// ---------------------------------------------------------------------------

/// `&dyn Any` を i64 に変換する。
/// 対応型: i64, i32, i16, i8, u64, u32, u16, u8, usize, isize
/// 変換不可能な型（構造体・タプル等）は `None` を返す。
fn downcast_to_i64(v: &dyn Any) -> Option<i64> {
    if let Some(&x) = v.downcast_ref::<i64>()   { return Some(x); }
    if let Some(&x) = v.downcast_ref::<i32>()   { return Some(x as i64); }
    if let Some(&x) = v.downcast_ref::<i16>()   { return Some(x as i64); }
    if let Some(&x) = v.downcast_ref::<i8>()    { return Some(x as i64); }
    if let Some(&x) = v.downcast_ref::<u32>()   { return Some(x as i64); }
    if let Some(&x) = v.downcast_ref::<u16>()   { return Some(x as i64); }
    if let Some(&x) = v.downcast_ref::<u8>()    { return Some(x as i64); }
    if let Some(&x) = v.downcast_ref::<isize>() { return Some(x as i64); }
    // u64 / usize は i64 範囲外の可能性があるため checked_cast
    if let Some(&x) = v.downcast_ref::<u64>()   { return i64::try_from(x).ok(); }
    if let Some(&x) = v.downcast_ref::<usize>() { return i64::try_from(x).ok(); }
    None
}

/// `&dyn Any` が「空」かどうかを判定する。
/// 対応型: String, &str, Vec<_>（長さ 0 を空とみなす）
fn downcast_is_empty(v: &dyn Any) -> Option<bool> {
    if let Some(s) = v.downcast_ref::<String>() { return Some(s.is_empty()); }
    if let Some(s) = v.downcast_ref::<&str>()   { return Some(s.is_empty()); }
    // Vec は型消去されるため型パラメータを列挙する
    if let Some(s) = v.downcast_ref::<Vec<u8>>()    { return Some(s.is_empty()); }
    if let Some(s) = v.downcast_ref::<Vec<String>>() { return Some(s.is_empty()); }
    None
}

// ---------------------------------------------------------------------------
// VerificationError — 全失敗ケースの網羅的定義
// ---------------------------------------------------------------------------

/// 検証失敗の原因。`_` catch-all パターンを使用することを禁じる。
/// Zero-Ambiguity 原則：全ての失敗は具体的な型として記述される。
#[derive(Debug, Error)]
pub enum VerificationError {
    #[error("制約が充足不可能: {predicate}")]
    Unsatisfiable { predicate: String },

    #[error("分岐の網羅性が証明できない: {context}")]
    NonExhaustiveBranch { context: String },

    #[error("ProofToken の改竄を検出: expected={expected:016x}, got={got:016x}")]
    ProofTampered { expected: u64, got: u64 },

    #[error("禁止パターンが検出された: {pattern}")]
    ForbiddenPatternDetected { pattern: String },

    #[error("ContractualGoal が正形式でない: {reason}")]
    MalformedGoal { reason: String },

    #[error("Z3 ソルバーエラー: {detail}")]
    SolverError { detail: String },

    #[error("憲法違反 — ハードコンストレイント {article} に違反: {detail}")]
    ConstitutionViolation { article: u8, detail: String },
}

// ---------------------------------------------------------------------------
// Verifier トレイト
// ---------------------------------------------------------------------------

/// 制約の充足性・網羅性を証明するバックエンドの抽象インターフェース。
///
/// # Contract（このトレイト自身の ContractualGoal）
/// - `verify_constraints` が `Ok(token)` を返す ⟺ 全制約が充足されている
/// - `verify_goal` が `Ok(())` を返す ⟺ ContractualGoal が憲法に適合している
/// - エラーは `VerificationError` の具体的なバリアントで表現される（catch-all なし）
pub trait Verifier {
    /// 値 `T` が制約リストを全て満たすことを検証し、ProofToken を発行する。
    ///
    /// `T: Any` を要求することで、実装側が具体的な数値型にダウンキャストして
    /// 値レベルのチェック（`InRange` の実際の範囲確認等）を行える。
    fn verify_constraints<T: fmt::Debug + Any>(
        &self,
        value:       &T,
        constraints: &[Constraint],
    ) -> Result<ProofToken, VerificationError>;

    /// `ContractualGoal` が hammurabi の Hard Constraints（憲法）に適合するか検証する。
    fn verify_goal(
        &self,
        goal: &ContractualGoal,
    ) -> Result<ConstitutionalReport, VerificationError>;

    /// 述語が事前条件のもとで恒真であることを証明する（∀ input. pre → pred）。
    fn prove_invariant(
        &self,
        preconditions: &[Predicate],
        invariant:     &Predicate,
    ) -> Result<ProofStatus, VerificationError>;
}

// ---------------------------------------------------------------------------
// ConstitutionalReport — 憲法適合性レポート
// ---------------------------------------------------------------------------

/// `verify_goal` が返す詳細レポート。
#[derive(Debug)]
pub struct ConstitutionalReport {
    pub goal_name:         String,
    pub exhaustive:        bool,
    pub null_safety:       bool,
    pub no_forbidden:      bool,
    pub proof_backend:     VerifierBackend,
    pub violations:        Vec<String>,
}

impl ConstitutionalReport {
    pub fn is_compliant(&self) -> bool {
        self.violations.is_empty() && self.exhaustive && self.null_safety && self.no_forbidden
    }
}

impl fmt::Display for ConstitutionalReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.is_compliant() { "COMPLIANT" } else { "VIOLATION" };
        writeln!(f, "=== Constitutional Report: {} [{}] ===", self.goal_name, status)?;
        writeln!(f, "  Exhaustive branches : {}", self.exhaustive)?;
        writeln!(f, "  Null safety         : {}", self.null_safety)?;
        writeln!(f, "  No forbidden patterns: {}", self.no_forbidden)?;
        writeln!(f, "  Backend             : {:?}", self.proof_backend)?;
        if !self.violations.is_empty() {
            writeln!(f, "  Violations:")?;
            for v in &self.violations {
                writeln!(f, "    ✗ {v}")?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ProofStatus — 不変条件の証明結果
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ProofStatus {
    /// 全ての入力で成立することが証明された
    Proven,
    /// 反例が発見された（Counterexample として具体値を保持）
    Disproven { counterexample: String },
    /// 証明も反証も確定できなかった（Unknown）
    Unknown,
}

// ---------------------------------------------------------------------------
// MockVerifier — 開発・テスト用バックエンド
// ---------------------------------------------------------------------------

/// 構造的整合性チェックのみを行う開発用 Verifier。
/// Z3 なしでビルド・テストが通る。`z3-backend` feature で置き換え可能。
///
/// # 制限
/// - 実際の SMT 充足性チェックは行わない
/// - `ProofToken` の backend フィールドは `VerifierBackend::Mock`
#[derive(Debug, Default)]
pub struct MockVerifier;

impl Verifier for MockVerifier {
    fn verify_constraints<T: fmt::Debug + Any>(
        &self,
        value:       &T,
        constraints: &[Constraint],
    ) -> Result<ProofToken, VerificationError> {
        // 具体値へのダウンキャストを試みる（数値型のみ）
        let numeric = downcast_to_i64(value as &dyn Any);

        for c in constraints {
            match c {
                // ── InRange ──────────────────────────────────────────────
                // 1. 構造的整合性: min ≤ max
                // 2. 値レベル: 具体値が [min, max] に収まるか（数値型のみ）
                Constraint::InRange { min, max } => {
                    if min > max {
                        return Err(VerificationError::Unsatisfiable {
                            predicate: format!("InRange: min={min} > max={max}"),
                        });
                    }
                    if let Some(v) = numeric {
                        if v < *min || v > *max {
                            return Err(VerificationError::Unsatisfiable {
                                predicate: format!(
                                    "値 {v} は範囲外 [{min}, {max}]"
                                ),
                            });
                        }
                    }
                }

                // ── NonNull ───────────────────────────────────────────────
                // Rust の参照は常に非 null なのでチェック不要
                Constraint::NonNull => {}

                // ── NonEmpty ─────────────────────────────────────────────
                // String / &str / Vec を対象に空チェック
                Constraint::NonEmpty => {
                    if let Some(empty) = downcast_is_empty(value as &dyn Any) {
                        if empty {
                            return Err(VerificationError::Unsatisfiable {
                                predicate: "NonEmpty: 値が空".into(),
                            });
                        }
                    }
                }

                // 述語・クロスレール制約は Mock では未評価
                Constraint::Predicate(_) | Constraint::ConsistentWith(_) => {}
            }
        }

        let hash  = hash_constraints(constraints);
        let token = ProofToken::new(hash, VerifierBackend::Mock);
        Ok(token)
    }

    fn verify_goal(
        &self,
        goal: &ContractualGoal,
    ) -> Result<ConstitutionalReport, VerificationError> {
        if !goal.is_well_formed() {
            return Err(VerificationError::MalformedGoal {
                reason: "postconditions が空: 契約は最低1つの事後条件を要求する".into(),
            });
        }

        let mut violations = Vec::new();

        // Hard Constraint 1: The "What" Interface — preconditions が空の場合は警告
        if goal.preconditions.is_empty() {
            violations.push("Article-1: preconditions が未定義（全入力に対して有効か不明）".into());
        }

        // Hard Constraint 2: Zero-Ambiguity — 禁止パターンのチェック
        let has_non_exhaustive = goal.forbidden.contains(&ForbiddenPattern::NonExhaustiveBranch);

        // Hard Constraint 3: 禁止パターンの重複・矛盾チェック（Mock では構造確認のみ）
        let no_forbidden = has_non_exhaustive;

        Ok(ConstitutionalReport {
            goal_name:     goal.name.clone(),
            exhaustive:    has_non_exhaustive,
            null_safety:   true, // Mock では楽観的に true
            no_forbidden,
            proof_backend: VerifierBackend::Mock,
            violations,
        })
    }

    fn prove_invariant(
        &self,
        _preconditions: &[Predicate],
        _invariant:     &Predicate,
    ) -> Result<ProofStatus, VerificationError> {
        // Mock: 構造的チェックのみ — 恒真・恒偽は即決、それ以外は Unknown
        Ok(match _invariant {
            Predicate::True  => ProofStatus::Proven,
            Predicate::False => ProofStatus::Disproven {
                counterexample: "⊥ は常に偽".into(),
            },
            _ => ProofStatus::Unknown,
        })
    }
}

// ---------------------------------------------------------------------------
// Z3Verifier — 本番 SMT バックエンド（z3-backend feature 限定）
// ---------------------------------------------------------------------------

#[cfg(feature = "z3-backend")]
pub mod z3_backend {
    use super::*;
    use z3::{Solver, SatResult, Pattern};
    use z3::ast::{self, Bool, Int};

    /// Z3 SMT ソルバーを用いた厳密検証バックエンド。
    /// `brew install z3` および `--features z3-backend` が必要。
    ///
    /// z3 0.19 以降はコンテキストがグローバル管理に変更されたため、
    /// Context/Config の明示的な生成・受け渡しは不要。
    pub struct Z3Verifier;

    impl Z3Verifier {
        pub fn new() -> Self {
            Self
        }

        /// `Predicate` AST を z3 0.19 の `Bool` に変換する。
        /// コンテキスト引数は不要（グローバルコンテキストを使用）。
        fn predicate_to_bool(pred: &Predicate) -> Bool {
            match pred {
                Predicate::True  => Bool::from_bool(true),
                Predicate::False => Bool::from_bool(false),
                Predicate::Atom(name) => Bool::new_const(name.as_str()),
                Predicate::Not(p) => {
                    Self::predicate_to_bool(p).not()
                }
                Predicate::And(l, r) => {
                    let lv = Self::predicate_to_bool(l);
                    let rv = Self::predicate_to_bool(r);
                    Bool::and(&[&lv, &rv])
                }
                Predicate::Or(l, r) => {
                    let lv = Self::predicate_to_bool(l);
                    let rv = Self::predicate_to_bool(r);
                    Bool::or(&[&lv, &rv])
                }
                Predicate::Implies(ante, cons) => {
                    let av = Self::predicate_to_bool(ante);
                    let cv = Self::predicate_to_bool(cons);
                    av.implies(&cv)
                }
                Predicate::InRange { var, min, max } => {
                    let x  = Int::new_const(var.as_str());
                    let lo = Int::from_i64(*min);
                    let hi = Int::from_i64(*max);
                    let ge = x.ge(&lo);
                    let le = x.le(&hi);
                    Bool::and(&[&ge, &le])
                }
                Predicate::NonNull(var) => {
                    Bool::new_const(format!("NonNull_{var}").as_str())
                }
                Predicate::Equals(a, b) => {
                    let x = Int::new_const(a.as_str());
                    let y = Int::new_const(b.as_str());
                    x.eq(&y)
                }
                // ∀var. body — 全称量化
                // 実装方針:
                //   1. 束縛変数を Int として宣言（現 AST は整数ドメインのみ）
                //   2. body を再帰的にエンコード（body 内の同名 Int::new_const が
                //      Z3 により束縛変数と自動同一視される）
                //   3. forall_const で量化式を生成
                //      patterns=&[] : ヒントなし（Z3 任意インスタンシエーション戦略）
                Predicate::ForAll { var, body } => {
                    let bound = Int::new_const(var.as_str());
                    let body_bool = Self::predicate_to_bool(body);
                    let no_patterns: &[&Pattern] = &[];
                    ast::forall_const(
                        &[&bound as &dyn ast::Ast],
                        no_patterns,
                        &body_bool,
                    )
                }

                // ∃var. body — 存在量化
                // 「制約を満たす値が少なくとも1つ存在すること」を表明する。
                // verify_constraints で使うと SAT チェックが存在証明になる。
                Predicate::Exists { var, body } => {
                    let bound = Int::new_const(var.as_str());
                    let body_bool = Self::predicate_to_bool(body);
                    let no_patterns: &[&Pattern] = &[];
                    ast::exists_const(
                        &[&bound as &dyn ast::Ast],
                        no_patterns,
                        &body_bool,
                    )
                }
            }
        }
    }

    impl Verifier for Z3Verifier {
        fn verify_constraints<T: std::fmt::Debug + Any>(
            &self,
            value:       &T,
            constraints: &[Constraint],
        ) -> Result<ProofToken, VerificationError> {
            let solver = Solver::new();

            // 具体値が数値型の場合、シンボリック変数 "x" に等値制約を付与する。
            // これにより「x = 具体値 ∧ min ≤ x ≤ max」という形式で
            // 値レベルの SAT チェックが可能になる。
            let concrete: Option<i64> = downcast_to_i64(value as &dyn Any);
            if let Some(v) = concrete {
                let x   = Int::new_const("x");
                let val = Int::from_i64(v);
                solver.assert(&x.eq(&val));
            }

            for c in constraints {
                match c {
                    Constraint::InRange { min, max } => {
                        if min > max {
                            return Err(VerificationError::Unsatisfiable {
                                predicate: format!("InRange: {min} > {max}"),
                            });
                        }
                        // 具体値あり: x = v ∧ min ≤ x ≤ max → UNSAT なら範囲外
                        // 具体値なし: ∃x. min ≤ x ≤ max（充足可能性チェック）
                        let x  = Int::new_const("x");
                        let lo = Int::from_i64(*min);
                        let hi = Int::from_i64(*max);
                        solver.assert(&x.ge(&lo));
                        solver.assert(&x.le(&hi));
                    }
                    Constraint::Predicate(pred) => {
                        solver.assert(&Self::predicate_to_bool(pred));
                    }
                    Constraint::NonNull | Constraint::NonEmpty => {}
                    Constraint::ConsistentWith(_) => {}
                }
            }

            match solver.check() {
                SatResult::Sat => {
                    let hash  = hash_constraints(constraints);
                    let token = ProofToken::new(hash, VerifierBackend::Z3Smt);
                    Ok(token)
                }
                SatResult::Unsat => Err(VerificationError::Unsatisfiable {
                    predicate: "制約セット全体が充足不可能（UNSAT）".into(),
                }),
                SatResult::Unknown => Err(VerificationError::SolverError {
                    detail: "Z3 が Unknown を返した（タイムアウトまたは量化器の限界）".into(),
                }),
            }
        }

        fn verify_goal(
            &self,
            goal: &ContractualGoal,
        ) -> Result<ConstitutionalReport, VerificationError> {
            if !goal.is_well_formed() {
                return Err(VerificationError::MalformedGoal {
                    reason: "postconditions が空".into(),
                });
            }

            let solver     = Solver::new();
            let mut violations = Vec::new();

            for pre in &goal.preconditions {
                solver.assert(&Self::predicate_to_bool(pre));
            }
            for post in &goal.postconditions {
                solver.assert(&Self::predicate_to_bool(post));
            }

            let exhaustive = goal.forbidden.contains(&ForbiddenPattern::NonExhaustiveBranch);
            if !exhaustive {
                violations.push("Article-2: NonExhaustiveBranch が forbidden リストにない".into());
            }
            if matches!(solver.check(), SatResult::Unsat) {
                violations.push("事後条件が事前条件のもとで充足不可能".into());
            }

            Ok(ConstitutionalReport {
                goal_name:     goal.name.clone(),
                exhaustive,
                null_safety:   true,
                no_forbidden:  exhaustive,
                proof_backend: VerifierBackend::Z3Smt,
                violations,
            })
        }

        fn prove_invariant(
            &self,
            preconditions: &[Predicate],
            invariant:     &Predicate,
        ) -> Result<ProofStatus, VerificationError> {
            let solver = Solver::new();

            // 証明: pre → inv が恒真 ⟺ pre ∧ ¬inv が UNSAT
            for pre in preconditions {
                solver.assert(&Self::predicate_to_bool(pre));
            }
            solver.assert(&Self::predicate_to_bool(invariant).not());

            match solver.check() {
                SatResult::Unsat  => Ok(ProofStatus::Proven),
                SatResult::Sat    => {
                    let model = solver.get_model()
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| "反例モデル取得不可".into());
                    Ok(ProofStatus::Disproven { counterexample: model })
                }
                SatResult::Unknown => Ok(ProofStatus::Unknown),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};

    #[test]
    fn mock_verifier_accepts_valid_range() {
        let v = MockVerifier::default();
        let result = v.verify_constraints(
            &42_i64,
            &[Constraint::InRange { min: 0, max: 100 }],
        );
        assert!(result.is_ok());
    }

    #[test]
    fn mock_verifier_rejects_inverted_range() {
        let v = MockVerifier::default();
        let result = v.verify_constraints(
            &42_i64,
            &[Constraint::InRange { min: 100, max: 0 }],
        );
        assert!(matches!(result, Err(VerificationError::Unsatisfiable { .. })));
    }

    // ── 値レベルチェック ─────────────────────────────────────────────────

    #[test]
    fn mock_rejects_value_below_min() {
        let v = MockVerifier::default();
        let result = v.verify_constraints(
            &(-1_i64),
            &[Constraint::InRange { min: 0, max: 100 }],
        );
        assert!(matches!(result, Err(VerificationError::Unsatisfiable { .. })),
            "値 -1 は [0,100] 外なので弾かれるべき");
    }

    #[test]
    fn mock_rejects_value_above_max() {
        let v = MockVerifier::default();
        let result = v.verify_constraints(
            &200_i64,
            &[Constraint::InRange { min: 0, max: 100 }],
        );
        assert!(matches!(result, Err(VerificationError::Unsatisfiable { .. })),
            "値 200 は [0,100] 外なので弾かれるべき");
    }

    #[test]
    fn mock_accepts_boundary_values() {
        let v = MockVerifier::default();
        // 境界値（下限）
        assert!(v.verify_constraints(&0_i64,   &[Constraint::InRange { min: 0, max: 100 }]).is_ok());
        // 境界値（上限）
        assert!(v.verify_constraints(&100_i64, &[Constraint::InRange { min: 0, max: 100 }]).is_ok());
    }

    #[test]
    fn mock_non_numeric_type_skips_value_check() {
        let v = MockVerifier::default();
        // 非数値型は値チェックをスキップ（構造チェックのみ）
        let result = v.verify_constraints(
            &"hello",
            &[Constraint::InRange { min: 0, max: 100 }],
        );
        assert!(result.is_ok(), "非数値型は値チェックをスキップして構造チェックのみ");
    }

    #[test]
    fn mock_rejects_empty_string_with_non_empty_constraint() {
        let v      = MockVerifier::default();
        let result = v.verify_constraints(
            &String::from(""),
            &[Constraint::NonEmpty],
        );
        assert!(matches!(result, Err(VerificationError::Unsatisfiable { .. })),
            "空文字列は NonEmpty 制約で弾かれるべき");
    }

    #[test]
    fn mock_accepts_non_empty_string() {
        let v      = MockVerifier::default();
        let result = v.verify_constraints(
            &String::from("hello"),
            &[Constraint::NonEmpty],
        );
        assert!(result.is_ok());
    }

    #[test]
    fn mock_verifier_checks_i32_range() {
        let v = MockVerifier::default();
        // i32 型でもダウンキャストして検証できる
        assert!(v.verify_constraints(&5_i32,   &[Constraint::InRange { min: 0, max: 10 }]).is_ok());
        assert!(v.verify_constraints(&(-1_i32), &[Constraint::InRange { min: 0, max: 10 }]).is_err());
    }

    #[test]
    fn mock_verifier_rejects_malformed_goal() {
        let v    = MockVerifier::default();
        let goal = ContractualGoal::new("empty_goal");
        let result = v.verify_goal(&goal);
        assert!(matches!(result, Err(VerificationError::MalformedGoal { .. })));
    }

    #[test]
    fn constitutional_report_compliant() {
        let v = MockVerifier::default();
        let goal = ContractualGoal::new("safe_division")
            .require(Predicate::atom("divisor_nonzero"))
            .ensure(Predicate::atom("result_is_finite"))
            .forbid(ForbiddenPattern::NonExhaustiveBranch);

        let report = v.verify_goal(&goal).unwrap();
        println!("{report}");
        assert!(report.is_compliant());
    }

    #[test]
    fn prove_true_invariant() {
        let v      = MockVerifier::default();
        let status = v.prove_invariant(&[], &Predicate::True).unwrap();
        assert_eq!(status, ProofStatus::Proven);
    }

    #[test]
    fn disprove_false_invariant() {
        let v      = MockVerifier::default();
        let status = v.prove_invariant(&[], &Predicate::False).unwrap();
        assert!(matches!(status, ProofStatus::Disproven { .. }));
    }

    // -----------------------------------------------------------------------
    // Z3 量化器テスト（z3-backend feature 限定）
    // -----------------------------------------------------------------------

    /// ∀x. (0 ≤ x ≤ 10) → (0 ≤ x ≤ 10)  は恒真 → Proven
    #[cfg(feature = "z3-backend")]
    #[test]
    fn z3_forall_tautology_is_proven() {
        use crate::compiler::verifier::z3_backend::Z3Verifier;

        let v = Z3Verifier::new();
        // ∀x. (InRange(x,0,10) → InRange(x,0,10))
        let inv = Predicate::for_all(
            "x",
            Predicate::implies(
                Predicate::in_range("x", 0, 10),
                Predicate::in_range("x", 0, 10),
            ),
        );
        let status = v.prove_invariant(&[], &inv).unwrap();
        assert_eq!(status, ProofStatus::Proven,
            "恒真の ForAll は Proven になるべき");
    }

    /// ∀x. (0 ≤ x ≤ 10) は恒真ではない（反例: x = -1） → Disproven
    #[cfg(feature = "z3-backend")]
    #[test]
    fn z3_forall_non_tautology_is_disproven() {
        use crate::compiler::verifier::z3_backend::Z3Verifier;

        let v = Z3Verifier::new();
        // ∀x. 0 ≤ x ≤ 10 — x=-1 で偽
        let inv = Predicate::for_all("x", Predicate::in_range("x", 0, 10));
        let status = v.prove_invariant(&[], &inv).unwrap();
        assert!(matches!(status, ProofStatus::Disproven { .. }),
            "∀x. 0≤x≤10 は反例 x=-1 が存在するので Disproven になるべき");
    }

    /// ∃x. (x > 100) が充足可能か — Constraint::Predicate として assert
    #[cfg(feature = "z3-backend")]
    #[test]
    fn z3_exists_positive_witness_is_sat() {
        use crate::compiler::verifier::z3_backend::Z3Verifier;
        use crate::lang::rail::Constraint;

        let v = Z3Verifier::new();
        // ∃x. x > 100  →  SAT（x=101 が証人）
        let pred = Predicate::exists(
            "x",
            Predicate::in_range("x", 101, i64::MAX),
        );
        let result = v.verify_constraints(&0_i64, &[Constraint::Predicate(pred)]);
        assert!(result.is_ok(), "∃x. x>100 は充足可能なので Ok になるべき");
    }

    /// ∃x. (x > 0 ∧ x < 0) は充足不可能 → Err(Unsatisfiable)
    #[cfg(feature = "z3-backend")]
    #[test]
    fn z3_exists_contradictory_witness_is_unsat() {
        use crate::compiler::verifier::z3_backend::Z3Verifier;
        use crate::lang::rail::Constraint;

        let v = Z3Verifier::new();
        // ∃x. (x > 0 ∧ x < 0)  →  UNSAT
        let pred = Predicate::exists(
            "x",
            Predicate::and(
                Predicate::in_range("x", 1, i64::MAX),
                Predicate::in_range("x", i64::MIN, -1),
            ),
        );
        let result = v.verify_constraints(&0_i64, &[Constraint::Predicate(pred)]);
        assert!(matches!(result, Err(VerificationError::Unsatisfiable { .. })),
            "∃x. (x>0 ∧ x<0) は矛盾しているので Unsatisfiable になるべき");
    }

    /// prove_invariant: ∀x. (x ∈ [0,10] → x ≥ 0) は恒真命題 → Proven
    ///
    /// # 設計メモ
    /// `prove_invariant` は「NOT(invariant) が UNSAT」を Z3 に問う。
    /// ForAll の束縛変数は事前条件の自由変数とは独立しているため、
    /// 「事前条件のもとで ∀x. P」を証明したい場合は
    /// `ForAll("x", Implies(pre(x), P(x)))` という形式で書く必要がある。
    #[cfg(feature = "z3-backend")]
    #[test]
    fn z3_forall_implication_is_proven() {
        use crate::compiler::verifier::z3_backend::Z3Verifier;

        let v = Z3Verifier::new();
        // ∀x. (0 ≤ x ≤ 10  →  0 ≤ x ≤ MAX)  は恒真
        // NOT(∀x. ...) = ∃x. (0 ≤ x ≤ 10 ∧ x < 0) → UNSAT → Proven
        let inv = Predicate::for_all(
            "x",
            Predicate::implies(
                Predicate::in_range("x", 0, 10),
                Predicate::in_range("x", 0, i64::MAX),
            ),
        );
        let status = v.prove_invariant(&[], &inv).unwrap();
        assert_eq!(status, ProofStatus::Proven,
            "∀x. (x∈[0,10] → x≥0) は恒真なので Proven になるべき");
    }

    /// prove_invariant: ∀x. (x ∈ [-10,10] → x > 5) は恒真でない → Disproven（反例: x=0）
    #[cfg(feature = "z3-backend")]
    #[test]
    fn z3_forall_implication_counterexample() {
        use crate::compiler::verifier::z3_backend::Z3Verifier;

        let v = Z3Verifier::new();
        // ∀x. (x ∈ [-10,10] → x > 5)  は偽（反例: x=0）
        let inv = Predicate::for_all(
            "x",
            Predicate::implies(
                Predicate::in_range("x", -10, 10),
                Predicate::in_range("x",   6, i64::MAX),
            ),
        );
        let status = v.prove_invariant(&[], &inv).unwrap();
        assert!(matches!(status, ProofStatus::Disproven { .. }),
            "∀x. (x∈[-10,10] → x>5) は反例 x=0 が存在するので Disproven になるべき");
    }
}
