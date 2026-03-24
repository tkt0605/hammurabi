# Hammurabi

> **Logic over Implementation. Proof over Testing.**
> 「コードを書く」前に「論理を証明する」AI ネイティブ言語システム

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
│   │   ├── goal.rs          # ContractualGoal — AI への発注書（述語論理）
│   │   └── rail.rs          # LogicRail<T>    — 証明を封印したコンテナ
│   ├── compiler/
│   │   └── verifier.rs      # Verifier trait  — 憲法適合性チェックエンジン
│   ├── codegen.rs            # ContractualGoal → 多言語コードスケルトン生成
│   ├── ai_gen/
│   │   ├── mod.rs            # AiGoalGenerator — 自然言語 → ContractualGoal
│   │   └── client.rs         # OpenAI / Anthropic HTTP クライアント
│   ├── config.rs             # config.hb / .env パーサー
│   ├── lsp/
│   │   └── mod.rs            # .hb パーサー + LSP 実装（補完・検証・ホバー）
│   ├── proof_store.rs        # ProofToken の永続化と署名検証
│   ├── math.rs               # 数学的ユーティリティ
│   └── wasm.rs               # WASM バインディング（feature: wasm）
├── src/bin/
│   ├── run_hb.rs             # メイン CLI（コード生成 / AI ゴール生成）
│   └── hammurabi_lsp.rs      # LSP サーバーバイナリ
├── config.hb                 # AI エージェント設定ファイル
├── .env.example              # 環境変数テンプレート
└── test.hb                   # サンプル .hb ファイル
```

### データフロー

#### モード 1：`.hb` ファイルからコード生成

```
[.hb ファイル]  goal ブロックを記述
    │
    ▼
[LSP パーサー]  parse_hb() — .hb テキスト → Vec<ContractualGoal>
    │
    ▼
[Verifier]  MockVerifier / Z3Verifier で制約を証明
    │  証明失敗 → エラー（実行前に止まる）
    ▼
[CodeGenerator]  ContractualGoal → 各言語のコードスケルトン
    │
    ▼
[出力]  Rust / Python / Go / Java / JavaScript / TypeScript
```

#### モード 2：自然言語から AI ゴール生成

```
[自然言語の説明]  "Safely divide two integers."
    │
    ▼
[AiGoalGenerator]  OpenAI / Anthropic / Mock
    │  GPT-4o / Claude に .hb フォーマットで出力させる
    ▼
[.hb テキスト]  AI が生成した ContractualGoal の定義
    │
    ▼
[LSP パーサー]  parse_hb() → Vec<ContractualGoal>
    │
    ▼
[CodeGenerator]  多言語コードスケルトンとして出力
```

---

## `.hb` ファイルフォーマット

Hammurabi 専用の DSL（ドメイン固有言語）。`ContractualGoal` を宣言的に記述する。

```
// ファイル設定（goal ブロックの前に記述）
agent   openai              // AI バックエンド: openai | anthropic | mock
model   gpt-4o              // モデル名（省略時はエージェントのデフォルト）
lang    rust                // 出力言語: rust | python | go | java | javascript | typescript
// api_key $OPENAI_API_KEY  // API キー（.env 推奨）

// goal ブロック（1 ファイルに複数定義可）
goal safe_division
require Or(InRange(divisor, -9223372036854775808, -1), InRange(divisor, 1, 9223372036854775807))
require InRange(dividend, -9223372036854775808, 9223372036854775807)
ensure result_is_finite
ensure result_within_i64_range
invariant no_memory_aliasing
forbid RuntimeNullCheck
forbid UnprovenUnwrap
```

### 述語一覧

| 述語 | 意味 |
|------|------|
| `NonNull(x)` | 変数 `x` が null でないこと |
| `InRange(x, min, max)` | 変数 `x` が `[min, max]` の範囲内にあること |
| `Or(p1, p2)` | 述語 `p1` または `p2` が成立すること |
| `<atom>` | 任意のアトム述語（意味は Z3 / Verifier が解釈） |

### 禁止パターン（`forbid`）

| パターン | 意味 |
|----------|------|
| `RuntimeNullCheck` | 実行時のヌルチェック（`if x == nil`）を禁止 |
| `UnprovenUnwrap` | 証明なき `unwrap()` / `!` を禁止 |
| `NonExhaustiveBranch` | 非網羅的な分岐を禁止 |
| `CatchAllSuppression` | `_ =>` / catch-all による抑制を禁止 |

---

## 3つのコアコンポーネント

### 1. `ContractualGoal` — AI への発注書

関数の「何をすべきか」を命令形なしで記述する。

```rust
use hammurabi::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};

let goal = ContractualGoal::new("safe_division")
    .require(Predicate::or(
        Predicate::in_range("divisor", i64::MIN, -1),
        Predicate::in_range("divisor",  1, i64::MAX),
    ))
    .ensure(Predicate::atom("result_is_finite"))
    .invariant(Predicate::atom("no_memory_aliasing"))
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

let divisor = LogicRail::bind(
    "divisor",
    5_i64,
    vec![
        Constraint::NonNull,
        Constraint::InRange { min: 1, max: i64::MAX },
    ],
    &verifier,
)?;

let result = dividend.map(
    "result",
    |d| d / *divisor.extract(),
    vec![Constraint::InRange { min: i64::MIN, max: i64::MAX }],
    &verifier,
)?;

println!("{}", result.proof()); // ProofToken[Mock✓|hash=...]
```

---

### 3. `Verifier` — 憲法適合性チェックエンジン

```rust
pub trait Verifier {
    fn verify_constraints<T: Debug>(
        &self, value: &T, constraints: &[Constraint],
    ) -> Result<ProofToken, VerificationError>;

    fn verify_goal(
        &self, goal: &ContractualGoal,
    ) -> Result<ConstitutionalReport, VerificationError>;

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

### API キーの設定（AI 機能を使う場合）

```bash
cp .env.example .env
# .env を編集して OPENAI_API_KEY または ANTHROPIC_API_KEY を設定
```

```bash
# .env
OPENAI_API_KEY=sk-proj-xxxxxxxxxxxxxxxxxxxxxxxxxxxx
ANTHROPIC_API_KEY=sk-ant-xxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

### Z3 本番バックエンドを有効化

```bash
# macOS
brew install z3

cargo build --features z3-backend
```

---

## CLI — `run_hb`

### 1. `.hb` ファイルからコード生成

```bash
# デフォルト（.hb 内の lang 設定を使用）
cargo run --bin run_hb -- test.hb

# 言語オーバーライド
cargo run --bin run_hb -- test.hb --lang python
cargo run --bin run_hb -- test.hb --lang typescript
```

### 2. `config.hb` を使った AI ゴール生成

```bash
cargo run --features ai --bin run_hb -- \
    --config config.hb \
    --ai "Safely divide two integers. Divisor must not be zero."
```

### 3. CLI 引数で AI エージェントを直接指定

```bash
cargo run --features ai --bin run_hb -- \
    --agent openai \
    --api-key sk-proj-... \
    --model gpt-4o \
    --lang typescript \
    --ai "Validate an email address."
```

### 4. Mock エージェント（API キー不要・開発用）

```bash
cargo run --bin run_hb -- \
    --agent mock \
    --ai "Compute the GCD of two non-negative integers."
```

### 優先順位

```
CLI 引数  >  .hb ファイル内設定  >  config.hb  >  デフォルト値
```

---

## LSP — `hammurabi_lsp`

`.hb` ファイルに対して LSP（Language Server Protocol）の補完・診断・ホバーを提供するサーバー。

```bash
cargo build --bin hammurabi_lsp
```

対応するエディタ（VS Code など）に LSP サーバーとして登録することで、`.hb` ファイルの編集支援が有効になる。

---

## AI エージェント

| エージェント | `--agent` | 必要なもの | 説明 |
|-------------|-----------|------------|------|
| `MockAiGenerator` | `mock` | なし | キーワード解析、オフライン動作（開発・テスト用） |
| `OpenAiGenerator` | `openai` | `ai` feature + API キー | GPT-4o による ContractualGoal 生成 |
| `AnthropicGenerator` | `anthropic` | `ai` feature + API キー | Claude による ContractualGoal 生成 |

---

## コード生成対応言語

| 言語 | `--lang` | 拡張子 |
|------|----------|--------|
| Rust | `rust` | `.rs` |
| Python | `python` | `.py` |
| Go | `go` | `.go` |
| Java | `java` | `.java` |
| JavaScript | `javascript` | `.js` |
| TypeScript | `typescript` | `.ts` |

---

## ロードマップ

| ステータス | 内容 |
|-----------|------|
| ✅ | `ContractualGoal` — 述語論理による仕様記述 |
| ✅ | `LogicRail<T>` — 証明封印コンテナ |
| ✅ | `MockVerifier` — 開発用バックエンド |
| ✅ | `Z3Verifier` — SMT バックエンド（`z3-backend` feature） |
| ✅ | `ProofToken` の永続化と署名検証（`proof_store`） |
| ✅ | `.hb` DSL パーサー（LSP 共通実装） |
| ✅ | `ContractualGoal` から多言語コードスケルトン自動生成（6言語対応） |
| ✅ | `AiGoalGenerator` — 自然言語 → ContractualGoal（OpenAI / Anthropic / Mock） |
| ✅ | `config.hb` / `.env` による設定管理 |
| ✅ | LSP サーバー — `.hb` ファイルの補完・診断・ホバー |
| ✅ | WASM ターゲットでのブラウザ実行（`wasm` feature） |
| ⏳ | VS Code 拡張の公開 |
| ⏳ | `ForAll` / `Exists` 量化述語の Z3 エンコード実装 |
| ⏳ | 正規表現制約など新しい `Constraint` タイプ |

---

## コントリビューション

Issue・PR・Star すべて歓迎です。

特に以下の領域での貢献を求めています。

- **述語論理の拡張** — `ForAll`/`Exists` の Z3 量化器エンコード実装
- **新しい `Constraint` タイプ** — 正規表現制約、型クラス制約 など
- **テストケース追加** — エッジケースや反例の充実
- **ドキュメント** — 英語 README、設計解説記事

```bash
git checkout -b feature/your-idea
cargo test  # テストが全て通ることを確認してから PR
```

---

## ドキュメント

| ファイル | 説明 |
|----------|------|
| [CLAUDE.md](./CLAUDE.md) | AI (Claude) 向けプロジェクトガイド |
| [config.hb](./config.hb) | AI エージェント設定ファイル（テンプレート） |
| [.env.example](./.env.example) | 環境変数テンプレート |
| [test.hb](./test.hb) | `.hb` フォーマットのサンプルファイル |
| `src/lang/goal.rs` | `ContractualGoal` と `Predicate` の設計詳細 |
| `src/lang/rail.rs` | `LogicRail<T>` と `ProofToken` の設計詳細 |
| `src/compiler/verifier.rs` | `Verifier` トレイトと Z3 バックエンドの実装 |
| `src/codegen.rs` | 多言語コードジェネレーターの実装 |
| `src/ai_gen/mod.rs` | AI ゴール生成パイプラインの実装 |
| `src/config.rs` | `config.hb` / `.env` パーサーの実装 |
| `src/lsp/mod.rs` | `.hb` パーサーと LSP 実装 |

---

## ライセンス

MIT License — 詳細は [LICENSE](./LICENSE) を参照。
