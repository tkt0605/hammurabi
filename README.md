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

## Getting Started

### インストール

```bash
# crates.io 公開後（ロードマップ参照）
# cargo install hammurabi --features full

# ローカル開発（リポジトリを clone した場合）
git clone https://github.com/hammurabi-lang/hammurabi.git
cd hammurabi
cargo install --path . --features full   # AI + Z3 を使う場合の例
```

### プロジェクトの初期化

```bash
# config.hb と .env.example を自動生成
hb init

# API キーを設定
cp .env.example .env
# .env を編集して OPENAI_API_KEY または ANTHROPIC_API_KEY を入力
```

### Z3 バックエンドを有効化する場合

```bash
# macOS
brew install z3

cargo build --features z3-backend
```

---

## CLI — `hb`

### サブコマンド一覧

各コマンドは **1 つの `.hb` ファイル** を引数に取る（ワイルドカードで複数ファイルを一度に渡しても、先頭の 1 ファイルだけが使われる）。

```
hb gen  <file.hb>   [OPTIONS]   .hb の契約から実装コードを生成
hb ai   "<prompt>"  [OPTIONS]   AI でゴールを生成 → コード生成
hb init [--force]               config.hb / .env.example を作成
hb check <file.hb>  [OPTIONS]   構文チェック + 契約検証（コード生成なし）
```

### 共通オプション（`gen` / `ai`）

```
--config  <path>   config.hb のパス（省略時はカレントディレクトリを自動検索）
--agent   <name>   openai | anthropic | mock
--api-key <key>    API キー（省略時は .env / 環境変数から自動解決）
--model   <name>   gpt-4o / claude-3-5-sonnet-20241022 など
--lang    <lang>   rust | python | go | java | javascript | typescript
--verifier <name>  mock（既定）| z3（要 z3-backend feature）
```

`--verifier` は **`hb gen` と `hb check` で有効**。`hb ai` は現状パースのみで生成フローには使わない（生成後の検証が必要なら `hb check` を別途実行）。

### `check` 専用（契約の整合性を Mock または Z3 で検証）

`--verifier` に取れるのは **`mock` と `z3` のみ**（モデル名は `--model` 用）。

```
hb check test.hb --verifier z3
hb check test.hb --verifier mock
```

### 設定の優先順位

```
CLI 引数  >  .hb ファイル内の指定  >  config.hb  >  .env  >  環境変数
```

### 使用例

```bash
# .hb ファイルからコード生成
hb gen test.hb
hb gen test.hb --lang python
# Z3 で仕様を証明してからコード生成（要: cargo build --features z3-backend）
hb gen test.hb --verifier z3

# AI でゴールを生成してコードを出力（Mock、API キー不要）
hb ai "Safely divide two integers. Divisor must not be zero." --agent mock

# AI でゴールを生成（OpenAI / Anthropic は `ai` feature が必要）
hb ai "Validate an email address." --agent openai --lang typescript

# 構文チェック + 契約検証（Mock または Z3）
hb check test.hb
hb check test.hb --verifier z3

# プロジェクト初期化（既存ファイルを上書きする場合は --force）
hb init
hb init --force
```

### Cargo features（`hb` バイナリのビルド）

デフォルトの `cargo build` は **依存最小**（Mock のみ・外部 AI / Z3 なし）。OpenAI / Anthropic や Z3 を使う場合は feature を付けてビルドする。

```bash
cargo build                              # 最小（Mock）
cargo build --features ai                # OpenAI / Anthropic 連携
cargo build --features z3-backend        # `--verifier z3`
cargo build --features full               # ai + z3-backend まとめて
cargo install --path . --features full  # ローカルにフル機能で入れる例
```

---

## `.hb` ファイルフォーマット

Hammurabi 専用の DSL。`ContractualGoal`（関数の論理仕様）を宣言的に記述する。

```hb
// ファイル設定（goal ブロックの前に記述）
// agent:   openai              // openai | anthropic | mock
// model:   gpt-4o              // 省略時はエージェントのデフォルト
// lang:    python              // rust | python | go | java | javascript | typescript
// api_key: $OPENAI_API_KEY    // API キー（.env 推奨）

// goal ブロック（1 ファイルに複数定義可）
goal: safe_division
  require:   Or(InRange(divisor, -9223372036854775808, -1), InRange(divisor, 1, 9223372036854775807))
  require:   InRange(dividend, -9223372036854775808, 9223372036854775807)
  ensure:    result_is_finite
  ensure:    result_within_i64_range
  invariant: no_memory_aliasing
  forbid:    RuntimeNullCheck
  forbid:    UnprovenUnwrap
```

### 述語一覧

| 述語 | 意味 |
|------|------|
| `NonNull(x)` | 変数 `x` が null でないこと |
| `InRange(x, min, max)` | 変数 `x` が `[min, max]` の範囲内にあること |
| `Or(p1, p2)` | 述語 `p1` または `p2` が成立すること |
| `<atom>` | 任意のアトム述語（意味は Verifier が解釈） |

### 禁止パターン（`forbid`）

| パターン | 意味 |
|----------|------|
| `RuntimeNullCheck` | 実行時のヌルチェック（`if x == nil`）を禁止 |
| `UnprovenUnwrap` | 証明なき `unwrap()` / `!` を禁止 |
| `NonExhaustiveBranch` | 非網羅的な分岐を禁止 |
| `CatchAllSuppression` | `_ =>` / catch-all による抑制を禁止 |

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
│   │   └── mod.rs            # .hb パーサー + LSP 実装（補完・診断・ホバー）
│   ├── proof_store.rs        # ProofToken の永続化と署名検証
│   ├── math.rs               # 数学的ユーティリティ
│   └── wasm.rs               # WASM バインディング（feature: wasm）
├── src/bin/
│   ├── hb.rs                 # メイン CLI（gen / ai / init / check）
│   ├── run_hb.rs             # 旧 CLI（後方互換）
│   └── hammurabi_lsp.rs      # LSP サーバーバイナリ
├── config.hb                 # AI エージェント設定ファイル
├── .env.example              # 環境変数テンプレート
└── test.hb                   # サンプル .hb ファイル
```

### データフロー

#### `hb gen` — `.hb` ファイルからコード生成

```
[.hb ファイル]  goal ブロックを記述
    │
    ▼
[LSP パーサー]  parse_hb() — .hb テキスト → Vec<ContractualGoal>
    │  構文エラー → 警告 / エラーを表示して終了
    ▼
[Verifier]  `--verifier z3` のときのみ Z3 で仕様を証明（失敗時は生成中止）
    │  既定の mock では gen はここで検証しない（`hb check` で常に検証可）
    ▼
[CodeWriter]  ContractualGoal → 各言語のコードスケルトン
    │          agent が mock 以外の場合は AI が実装コードを生成
    ▼
[出力]  Rust / Python / Go / Java / JavaScript / TypeScript
```

#### `hb ai` — 自然言語から AI ゴール生成

```
[自然言語の説明]  "Safely divide two integers."
    │
    ▼
[AiGoalGenerator]  OpenAI / Anthropic / Mock
    │  AI が .hb フォーマットで ContractualGoal を出力
    ▼
[LSP パーサー]  parse_hb() → Vec<ContractualGoal>
    │
    ▼
[CodeWriter]  多言語コードスケルトン（または AI 実装コード）として出力
```

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

## AI エージェント

| エージェント | `--agent` | feature | 説明 |
|-------------|-----------|---------|------|
| `MockAiGenerator` | `mock` | 不要 | キーワード解析、オフライン動作（開発・テスト用） |
| `OpenAiGenerator` | `openai` | `ai` | GPT 系による ContractualGoal / コード生成 |
| `AnthropicGenerator` | `anthropic` | `ai` | Claude による ContractualGoal / コード生成 |

`Cargo.toml` の `default` features は空です。OpenAI / Anthropic を使うには **`cargo build --features ai`**（または `full`）でビルドする。API キーなしで試す場合は `--agent mock` を指定する。

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

## LSP — `hammurabi_lsp`

`.hb` ファイルに対して LSP（Language Server Protocol）の補完・診断・ホバーを提供するサーバー。

```bash
cargo build --bin hammurabi_lsp
```

対応するエディタ（VS Code など）に LSP サーバーとして登録することで、`.hb` ファイルの編集支援が有効になる。

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
| ✅ | `hb` CLI — サブコマンド構造（`gen` / `ai` / `init` / `check`） |
| ✅ | LSP サーバー — `.hb` ファイルの補完・診断・ホバー |
| ✅ | WASM ターゲットでのブラウザ実行（`wasm` feature） |
| ⏳ | VS Code 拡張の公開 |
| ⏳ | `ForAll` / `Exists` 量化述語の Z3 エンコード実装 |
| ⏳ | 正規表現制約など新しい `Constraint` タイプ |
| ⏳ | `crates.io` への公開 |

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
