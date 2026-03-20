//! # wasm — ブラウザ向け wasm-bindgen バインディング
//!
//! `wasm-pack build --target web --features wasm` でビルドする。
//!
//! ## JavaScript からの使い方
//! ```js
//! import init, { WasmGoalBuilder, generate_code_from_json } from './pkg/hammurabi.js';
//!
//! await init();
//!
//! const goal = new WasmGoalBuilder("safe_division");
//! goal.require_non_null("divisor");
//! goal.require_in_range("divisor", 1, Number.MAX_SAFE_INTEGER);
//! goal.ensure_atom("result_is_finite");
//! goal.add_invariant("no_memory_aliasing");
//!
//! console.log(goal.generate_code());   // Rust スケルトン
//! console.log(goal.verify());          // JSON 形式の検証レポート
//! ```

use wasm_bindgen::prelude::*;
use crate::codegen::CodeGenerator;
use crate::compiler::verifier::{MockVerifier, Verifier};
use crate::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};

// ---------------------------------------------------------------------------
// 初期化（パニックを console.error に転送）
// ---------------------------------------------------------------------------

/// WASM モジュールの初期化。HTML から `await init()` 後に自動呼び出しされる。
#[wasm_bindgen(start)]
pub fn wasm_init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

// ---------------------------------------------------------------------------
// WasmGoalBuilder — JS から ContractualGoal を構築するクラス
// ---------------------------------------------------------------------------

/// JavaScript から `ContractualGoal` を組み立てるビルダー。
///
/// ```js
/// const g = new WasmGoalBuilder("safe_division");
/// g.require_non_null("divisor");
/// g.require_in_range("divisor", 1, 1e15);
/// g.ensure_atom("result_is_finite");
/// const code = g.generate_code();
/// ```
#[wasm_bindgen]
pub struct WasmGoalBuilder {
    goal: ContractualGoal,
}

#[wasm_bindgen]
impl WasmGoalBuilder {
    /// 関数名を指定してビルダーを生成する。
    #[wasm_bindgen(constructor)]
    pub fn new(name: &str) -> Self {
        Self {
            goal: ContractualGoal::new(name),
        }
    }

    // ── 事前条件（preconditions） ─────────────────────────────────────────

    /// `Atom` 述語を事前条件として追加する（例: "dividend_is_integer"）。
    pub fn require_atom(&mut self, pred: &str) {
        self.goal = self.goal.clone().require(Predicate::atom(pred));
    }

    /// `NonNull(var)` を事前条件として追加する。
    pub fn require_non_null(&mut self, var: &str) {
        self.goal = self.goal.clone().require(Predicate::non_null(var));
    }

    /// `InRange(var, min, max)` を事前条件として追加する。
    /// JavaScript の Number（f64）を受け取り i64 に変換する。
    pub fn require_in_range(&mut self, var: &str, min: f64, max: f64) {
        self.goal = self.goal.clone().require(
            Predicate::in_range(var, min as i64, max as i64),
        );
    }

    // ── 事後条件（postconditions） ────────────────────────────────────────

    /// `Atom` 述語を事後条件として追加する（例: "result_is_finite"）。
    pub fn ensure_atom(&mut self, pred: &str) {
        self.goal = self.goal.clone().ensure(Predicate::atom(pred));
    }

    // ── 不変条件（invariants） ────────────────────────────────────────────

    /// `Atom` 述語を不変条件として追加する（例: "no_memory_aliasing"）。
    pub fn add_invariant(&mut self, pred: &str) {
        self.goal = self.goal.clone().invariant(Predicate::atom(pred));
    }

    // ── 禁止パターン ─────────────────────────────────────────────────────

    /// 禁止パターンを追加する。
    /// 受け付けるキー: `"RuntimeNullCheck"`, `"ImplicitCoercion"`,
    /// `"NonExhaustiveBranch"`, `"UnprovenUnwrap"`, `"CatchAllSuppression"`
    pub fn forbid(&mut self, pattern: &str) {
        let fp = match pattern {
            "RuntimeNullCheck"     => ForbiddenPattern::RuntimeNullCheck,
            "ImplicitCoercion"     => ForbiddenPattern::ImplicitCoercion,
            "NonExhaustiveBranch"  => ForbiddenPattern::NonExhaustiveBranch,
            "UnprovenUnwrap"       => ForbiddenPattern::UnprovenUnwrap,
            "CatchAllSuppression"  => ForbiddenPattern::CatchAllSuppression,
            _ => return,
        };
        self.goal = self.goal.clone().forbid(fp);
    }

    // ── 出力 ─────────────────────────────────────────────────────────────

    /// `ContractualGoal` から Rust 関数スケルトンを生成して返す。
    pub fn generate_code(&self) -> String {
        CodeGenerator::new().generate(&self.goal).source().to_owned()
    }

    /// `ContractualGoal` の憲法適合性を検証し、JSON レポートを返す。
    ///
    /// ```json
    /// {
    ///   "goal_name": "safe_division",
    ///   "compliant": true,
    ///   "exhaustive": true,
    ///   "null_safety": true,
    ///   "violations": []
    /// }
    /// ```
    pub fn verify(&self) -> String {
        let verifier = MockVerifier::default();
        match verifier.verify_goal(&self.goal) {
            Ok(report) => serde_json::json!({
                "ok": true,
                "goal_name":   report.goal_name,
                "compliant":   report.is_compliant(),
                "exhaustive":  report.exhaustive,
                "null_safety": report.null_safety,
                "no_forbidden": report.no_forbidden,
                "backend":     format!("{:?}", report.proof_backend),
                "violations":  report.violations,
            }).to_string(),
            Err(e) => serde_json::json!({
                "ok": false,
                "error": e.to_string(),
            }).to_string(),
        }
    }

    /// `ContractualGoal` を JSON 文字列にシリアライズして返す。
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.goal)
            .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"))
    }

    /// 現在の状態のサマリを返す（デバッグ用）。
    pub fn summary(&self) -> String {
        format!(
            "Goal: {} | pre: {} | post: {} | inv: {} | forbidden: {}",
            self.goal.name,
            self.goal.preconditions.len(),
            self.goal.postconditions.len(),
            self.goal.invariants.len(),
            self.goal.forbidden.len(),
        )
    }
}

// ---------------------------------------------------------------------------
// スタンドアロン関数 — JSON 文字列 IN / 文字列 OUT
// ---------------------------------------------------------------------------

/// JSON 形式の `ContractualGoal` を受け取り、Rust コードスケルトンを返す。
///
/// # Errors
/// JSON のパースに失敗した場合はエラー文字列を返す。
#[wasm_bindgen]
pub fn generate_code_from_json(goal_json: &str) -> Result<String, JsValue> {
    let goal: ContractualGoal = serde_json::from_str(goal_json)
        .map_err(|e| JsValue::from_str(&format!("JSON parse error: {e}")))?;
    Ok(CodeGenerator::new().generate(&goal).source().to_owned())
}

/// JSON 形式の `ContractualGoal` を受け取り、検証レポートを JSON で返す。
#[wasm_bindgen]
pub fn verify_goal_from_json(goal_json: &str) -> String {
    let goal: ContractualGoal = match serde_json::from_str(goal_json) {
        Ok(g)  => g,
        Err(e) => return serde_json::json!({ "ok": false, "error": e.to_string() }).to_string(),
    };
    let verifier = MockVerifier::default();
    match verifier.verify_goal(&goal) {
        Ok(report) => serde_json::json!({
            "ok":          true,
            "goal_name":   report.goal_name,
            "compliant":   report.is_compliant(),
            "exhaustive":  report.exhaustive,
            "null_safety": report.null_safety,
            "violations":  report.violations,
        }).to_string(),
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }).to_string(),
    }
}

/// ライブラリのバージョン文字列を返す。
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
