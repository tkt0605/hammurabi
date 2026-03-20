//! # hammurabi — デモエントリポイント
//!
//! LogicRail と ContractualGoal を使った設計哲学のデモ。

use hammurabi::compiler::verifier::{MockVerifier, Verifier};
use hammurabi::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};
use hammurabi::lang::rail::{Constraint, LogicRail};

fn main() {
    println!("=== hammurabi — Logic-First Runtime Demo ===\n");

    let verifier = MockVerifier::default();

    // -----------------------------------------------------------------------
    // Step 1: ContractualGoal — 「安全な除算」の発注書を定義する
    // -----------------------------------------------------------------------
    let goal = ContractualGoal::new("safe_division")
        .require(Predicate::atom("dividend_is_integer"))
        .require(Predicate::non_null("divisor"))
        .require(Predicate::and(
            Predicate::in_range("divisor", i64::MIN, -1),
            Predicate::in_range("divisor",  1, i64::MAX),
        ))
        .ensure(Predicate::atom("result_is_finite"))
        .ensure(Predicate::for_all(
            "result",
            Predicate::atom("result_within_i64_range"),
        ))
        .invariant(Predicate::atom("no_memory_aliasing"))
        .forbid(ForbiddenPattern::RuntimeNullCheck)
        .forbid(ForbiddenPattern::UnprovenUnwrap);

    println!("{goal}\n");

    // -----------------------------------------------------------------------
    // Step 2: 憲法適合性チェック
    // -----------------------------------------------------------------------
    match verifier.verify_goal(&goal) {
        Ok(report) => println!("{report}"),
        Err(e)     => eprintln!("Constitutional violation: {e}"),
    }

    // -----------------------------------------------------------------------
    // Step 3: LogicRail — 証明済みの値を封印する
    // -----------------------------------------------------------------------
    let dividend = LogicRail::bind(
        "dividend",
        100_i64,
        vec![Constraint::InRange { min: i64::MIN, max: i64::MAX }],
        &verifier,
    )
    .expect("dividend の制約検証失敗");

    let divisor = LogicRail::bind(
        "divisor",
        5_i64,
        vec![
            Constraint::NonNull,
            Constraint::Predicate(Predicate::or(
                Predicate::in_range("divisor", i64::MIN, -1),
                Predicate::in_range("divisor",  1, i64::MAX),
            )),
        ],
        &verifier,
    )
    .expect("divisor の制約検証失敗");

    println!("{dividend}\n{divisor}");
    println!("  dividend proof: {}", dividend.proof());
    println!("  divisor  proof: {}", divisor.proof());

    // -----------------------------------------------------------------------
    // Step 4: LogicRail::map — 安全な変換（証明を引き継ぐ）
    // -----------------------------------------------------------------------
    let result = dividend.map(
        "result",
        |d| d / *divisor.extract(),
        vec![Constraint::InRange { min: i64::MIN, max: i64::MAX }],
        &verifier,
    )
    .expect("result の制約検証失敗");

    println!("\n--- 証明済み計算結果 ---");
    println!("  100 / 5 = {}", result.extract());
    println!("  {}", result.proof());

    // -----------------------------------------------------------------------
    // Step 5: 不変条件の証明テスト
    // -----------------------------------------------------------------------
    println!("\n--- 不変条件証明テスト ---");
    let status = verifier.prove_invariant(
        &[Predicate::atom("input_valid")],
        &Predicate::True,
    ).unwrap();
    println!("  ⊤ の証明結果: {status:?}");

    let status_false = verifier.prove_invariant(&[], &Predicate::False).unwrap();
    println!("  ⊥ の証明結果: {status_false:?}");

    println!("\n=== Demo 完了 ===");
}
