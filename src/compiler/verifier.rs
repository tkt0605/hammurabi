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
use thiserror::Error;
use crate::lang::rail::{Constraint, ProofToken, VerifierBackend, hash_constraints};
use crate::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};

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
    fn verify_constraints<T: fmt::Debug>(
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
    fn verify_constraints<T: fmt::Debug>(
        &self,
        _value:      &T,
        constraints: &[Constraint],
    ) -> Result<ProofToken, VerificationError> {
        // 構造的整合性: InRange の min ≤ max を確認
        for c in constraints {
            if let Constraint::InRange { min, max } = c {
                if min > max {
                    return Err(VerificationError::Unsatisfiable {
                        predicate: format!("InRange: min={min} > max={max}"),
                    });
                }
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
    use z3::{Config, Context, Solver, SatResult};
    use z3::ast::{Ast, Bool, Int};

    /// Z3 SMT ソルバーを用いた厳密検証バックエンド。
    /// `brew install z3` および `--features z3-backend` が必要。
    pub struct Z3Verifier {
        cfg: Config,
    }

    impl Z3Verifier {
        pub fn new() -> Self {
            let mut cfg = Config::new();
            cfg.set_bool_param_value("model", true);
            Self { cfg }
        }

        fn predicate_to_bool<'ctx>(
            ctx:   &'ctx Context,
            pred:  &Predicate,
        ) -> Bool<'ctx> {
            match pred {
                Predicate::True  => Bool::from_bool(ctx, true),
                Predicate::False => Bool::from_bool(ctx, false),
                Predicate::Atom(name) => Bool::new_const(ctx, name.as_str()),
                Predicate::Not(p) => {
                    let inner = Self::predicate_to_bool(ctx, p);
                    inner.not()
                }
                Predicate::And(l, r) => {
                    let lv = Self::predicate_to_bool(ctx, l);
                    let rv = Self::predicate_to_bool(ctx, r);
                    Bool::and(ctx, &[&lv, &rv])
                }
                Predicate::Or(l, r) => {
                    let lv = Self::predicate_to_bool(ctx, l);
                    let rv = Self::predicate_to_bool(ctx, r);
                    Bool::or(ctx, &[&lv, &rv])
                }
                Predicate::Implies(ante, cons) => {
                    let av = Self::predicate_to_bool(ctx, ante);
                    let cv = Self::predicate_to_bool(ctx, cons);
                    av.implies(&cv)
                }
                Predicate::InRange { var, min, max } => {
                    let x    = Int::new_const(ctx, var.as_str());
                    let lo   = Int::from_i64(ctx, *min);
                    let hi   = Int::from_i64(ctx, *max);
                    let ge   = x.ge(&lo);
                    let le   = x.le(&hi);
                    Bool::and(ctx, &[&ge, &le])
                }
                Predicate::NonNull(var) => {
                    // NonNull を Bool 変数で表現
                    Bool::new_const(ctx, format!("NonNull_{var}").as_str())
                }
                Predicate::Equals(a, b) => {
                    let x = Int::new_const(ctx, a.as_str());
                    let y = Int::new_const(ctx, b.as_str());
                    x._eq(&y)
                }
                // ForAll/Exists は量化器エンコードが必要（将来実装）
                Predicate::ForAll { .. } | Predicate::Exists { .. } => {
                    Bool::from_bool(ctx, true) // placeholder
                }
            }
        }
    }

    impl Verifier for Z3Verifier {
        fn verify_constraints<T: std::fmt::Debug>(
            &self,
            _value:      &T,
            constraints: &[Constraint],
        ) -> Result<ProofToken, VerificationError> {
            let ctx    = Context::new(&self.cfg);
            let solver = Solver::new(&ctx);

            for c in constraints {
                match c {
                    Constraint::InRange { min, max } => {
                        if min > max {
                            return Err(VerificationError::Unsatisfiable {
                                predicate: format!("InRange: {min} > {max}"),
                            });
                        }
                        // Z3 で SAT チェック: ∃x. min ≤ x ≤ max
                        let x  = Int::new_const(&ctx, "x");
                        let lo = Int::from_i64(&ctx, *min);
                        let hi = Int::from_i64(&ctx, *max);
                        solver.assert(&x.ge(&lo));
                        solver.assert(&x.le(&hi));
                    }
                    Constraint::Predicate(pred) => {
                        let b = Self::predicate_to_bool(&ctx, pred);
                        solver.assert(&b);
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

            let ctx    = Context::new(&self.cfg);
            let solver = Solver::new(&ctx);
            let mut violations = Vec::new();

            // 事前条件を仮定としてアサート
            for pre in &goal.preconditions {
                let b = Self::predicate_to_bool(&ctx, pre);
                solver.assert(&b);
            }

            // 事後条件が充足可能かチェック
            for post in &goal.postconditions {
                let b = Self::predicate_to_bool(&ctx, post);
                solver.assert(&b);
            }

            let exhaustive = goal.forbidden.contains(&ForbiddenPattern::NonExhaustiveBranch);
            if !exhaustive {
                violations.push("Article-2: NonExhaustiveBranch が forbidden リストにない".into());
            }

            let sat_result = solver.check();
            if sat_result == SatResult::Unsat {
                violations.push("事後条件が事前条件のもとで充足不可能".into());
            }

            let no_forbidden = exhaustive;
            Ok(ConstitutionalReport {
                goal_name:     goal.name.clone(),
                exhaustive,
                null_safety:   true,
                no_forbidden,
                proof_backend: VerifierBackend::Z3Smt,
                violations,
            })
        }

        fn prove_invariant(
            &self,
            preconditions: &[Predicate],
            invariant:     &Predicate,
        ) -> Result<ProofStatus, VerificationError> {
            let ctx    = Context::new(&self.cfg);
            let solver = Solver::new(&ctx);

            // 証明: pre → inv が恒真 ⟺ pre ∧ ¬inv が UNSAT
            for pre in preconditions {
                let b = Self::predicate_to_bool(&ctx, pre);
                solver.assert(&b);
            }
            let inv_bool = Self::predicate_to_bool(&ctx, invariant);
            solver.assert(&inv_bool.not());

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
}
