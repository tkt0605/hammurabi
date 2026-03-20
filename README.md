# Hammurabi

> **Logic over Implementation. Proof over Testing.**
> 「コードを書く」前に「論理を証明する」次世代 AI ネイティブ言語システム

[![Rust](https://img.shields.io/badge/language-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue)](./LICENSE)
[![Build](https://img.shields.io/badge/build-passing-brightgreen)](#getting-started)
[![Z3](https://img.shields.io/badge/SMT_solver-Z3-purple)](https://github.com/Z3Prover/z3)

---

## なぜ Hammurabi か

AI がコードを生成する時代、問題は「コードが書けるか」ではなく「**そのコードが正しいと証明できるか**」になった。

Hammurabi は、プログラミングのパラダイムを

```
「どう実装するか（How）」  →  「何を満たすべきか（What）」
```

へシフトさせるための **論理ファーストな言語基盤** だ。  
関数の定義に `if` 文は不要。代わりに述語論理で「制約」を書く。  
実行前に Z3 SMT ソルバーが「全分岐の網羅性」を数学的に証明する。

プロジェクト名は古代バビロニアの **ハンムラビ法典** に由来する。  
コードにも、法典のように — 曖昧さのないルールを示したい。

---

## コアコンセプト

Hammurabi は3つの原則（**憲法**）を持つ。

| # | 原則 | 意味 |
|---|------|------|
| 1 | **The "What" Interface** | 関数は「何をすべきか」を述語で書く。`if` による実装は排除 |
| 2 | **Zero-Ambiguity** | 全分岐は型定義段階で Z3 が網羅性を証明する |
| 3 | **Hardware-Level Determinism** | メモリ安全性は実行時チェックでなく論理的帰結として保証 |

---

## アーキテクチャ

```
hammurabi/
├── src/
│   ├── lang/
│   │   ├── goal.rs       # ContractualGoal  — AI への発注書（述語論理）
│   │   └── rail.rs       # LogicRail<T>     — 証明を封印したコンテナ
│   └── compiler/
│       └── verifier.rs   # Verifier trait   — 憲法適合性チェックエンジン
```

### データフロー

```
[開発者] ContractualGoal を定義（述語論理）
    │
    ▼
[Verifier] Z3 で充足性・網羅性を証明
    │  証明失敗 → コンパイルエラー（実行前に止まる）
    ▼
[ProofToken] 偽造不可のトークンを発行
    │
    ▼
[LogicRail<T>] 値 + 証明をシールして封印
    │  map / merge で変換しても常に再証明が必要
    ▼
[実行] ProofToken を持たない値は型システムが存在を認識しない
```

---

## 3つのコアコンポーネント

### 1. `ContractualGoal` — AI への発注書

関数の「何をすべきか」を命令形なしで記述する。

```rust
use hammurabi::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};

let goal = ContractualGoal::new("safe_division")
    // 事前条件: 除数はゼロ以外であること
    .require(Predicate::non_null("divisor"))
    .require(Predicate::or(
        Predicate::in_range("divisor", i64::MIN, -1),
        Predicate::in_range("divisor",  1, i64::MAX),
    ))
    // 事後条件: 結果は有限であること
    .ensure(Predicate::atom("result_is_finite"))
    // 不変条件: 実行中メモリエイリアスなし
    .invariant(Predicate::atom("no_memory_aliasing"))
    // 禁止パターン: if 文による場当たり的なヌルチェック
    .forbid(ForbiddenPattern::RuntimeNullCheck);
```

`if divisor == 0 { return Err(...) }` — こういうコードは **書かせない**。  
正しい除数の性質を型の段階で証明させる。

---

### 2. `LogicRail<T>` — 証明を封印したコンテナ

`ProofToken` なしでは `LogicRail<T>` は構築できない。証明なき値は存在しない。

```rust
use hammurabi::lang::rail::{Constraint, LogicRail};
use hammurabi::compiler::verifier::MockVerifier;

let verifier = MockVerifier::default();

// bind: 制約を Verifier に証明させてはじめて値が封印される
let divisor = LogicRail::bind(
    "divisor",
    5_i64,
    vec![
        Constraint::NonNull,
        Constraint::InRange { min: 1, max: i64::MAX },
    ],
    &verifier,
)?;

// map: 変換後も新たな制約と証明が必要（モナド的構造）
let result = dividend.map(
    "result",
    |d| d / *divisor.extract(),
    vec![Constraint::InRange { min: i64::MIN, max: i64::MAX }],
    &verifier,
)?;

println!("{}", result.proof()); // ProofToken[Mock✓|hash=...]
```

`T: Send + Sync` を型パラメータに要求することで、  
Apple M5 Pro の Unified Memory 上でのスレッド安全性を **実行時コストゼロ** で保証する。

---

### 3. `Verifier` — 憲法適合性チェックエンジン

```rust
pub trait Verifier {
    // 値が制約を満たすことを証明し、ProofToken を発行する
    fn verify_constraints<T: Debug>(
        &self, value: &T, constraints: &[Constraint],
    ) -> Result<ProofToken, VerificationError>;

    // ContractualGoal が 3 つの憲法原則に適合するか検証する
    fn verify_goal(
        &self, goal: &ContractualGoal,
    ) -> Result<ConstitutionalReport, VerificationError>;

    // ∀ input. precondition → invariant が成立するか Z3 で証明する
    fn prove_invariant(
        &self, preconditions: &[Predicate], invariant: &Predicate,
    ) -> Result<ProofStatus, VerificationError>;
}
```

エラーは **全ケースが網羅された enum** で表現される。`_` catch-all パターンは使わせない。

```rust
pub enum VerificationError {
    Unsatisfiable        { predicate: String },
    NonExhaustiveBranch  { context: String },
    ProofTampered        { expected: u64, got: u64 },
    ForbiddenPatternDetected { pattern: String },
    MalformedGoal        { reason: String },
    SolverError          { detail: String },
    ConstitutionViolation { article: u8, detail: String },
}
```

---

## Getting Started

### 必須環境

- Rust 1.70+（[インストール](https://rustup.rs/)）

```bash
git clone https://github.com/<your-account>/hammurabi.git
cd hammurabi

# デフォルトビルド（MockVerifier, Z3 不要）
cargo build

# テスト実行
cargo test
```

### Z3 本番バックエンドを有効化

```bash
# macOS
brew install z3

# ビルド
cargo build --features z3-backend
```

`z3-backend` feature が有効な場合、`Z3Verifier` が MockVerifier を置き換え、  
全ての制約検証に本物の SMT ソルバーが使用される。

---

## テスト結果

```
running 9 tests
test compiler::verifier::tests::constitutional_report_compliant   ... ok
test compiler::verifier::tests::disprove_false_invariant          ... ok
test compiler::verifier::tests::mock_verifier_accepts_valid_range ... ok
test compiler::verifier::tests::mock_verifier_rejects_inverted_range ... ok
test compiler::verifier::tests::mock_verifier_rejects_malformed_goal ... ok
test compiler::verifier::tests::prove_true_invariant              ... ok
test lang::rail::tests::bind_succeeds_with_valid_constraints      ... ok
test lang::rail::tests::map_produces_new_rail                     ... ok
test lang::rail::tests::merge_combines_two_rails                  ... ok

test result: ok. 9 passed; 0 failed
```

---

## ロードマップ

| ステータス | 内容 |
|-----------|------|
| ✅ | `ContractualGoal` — 述語論理による仕様記述 |
| ✅ | `LogicRail<T>` — 証明封印コンテナ |
| ✅ | `MockVerifier` — 開発用バックエンド |
| ✅ | `Z3Verifier` — SMT バックエンド（feature flag） |
| 🔄 | `ProofToken` の永続化と署名検証 |
| ⏳ | `ContractualGoal` から Rust コードの自動生成 |
| ⏳ | WASM ターゲットでのブラウザ実行 |
| ⏳ | LSP（Language Server Protocol）統合 |
| ⏳ | AI エージェントとの `ContractualGoal` 自動生成連携 |

---

## コントリビューション

Issue・PR・Star すべて歓迎です。

特に以下の領域での貢献を求めています。

- **述語論理の拡張** — `ForAll`/`Exists` の Z3 量化器エンコード実装
- **新しい `Constraint` タイプ** — 正規表現制約、型クラス制約 など
- **テストケース追加** — エッジケースや反例の充実
- **ドキュメント** — 英語 README、設計解説記事

```bash
# フォーク → ブランチ作成 → PR
git checkout -b feature/your-idea
cargo test  # テストが全て通ることを確認してから PR
```

---

## ドキュメント

| ファイル | 説明 |
|----------|------|
| [CLAUDE.md](./CLAUDE.md) | AI (Claude) 向けプロジェクトガイド |
| `src/lang/goal.rs` | `ContractualGoal` と `Predicate` の設計詳細 |
| `src/lang/rail.rs` | `LogicRail<T>` と `ProofToken` の設計詳細 |
| `src/compiler/verifier.rs` | `Verifier` トレイトと Z3 バックエンドの実装 |

---

## ライセンス

MIT License — 詳細は [LICENSE](./LICENSE) を参照。
