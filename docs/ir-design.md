# Hammurabi IR Design

## 1. Scope

### 1.1 Purpose of This Document

この文書は、`Hammurabi` における中間表現（IR: Intermediate Representation）の設計方針を定義する。

ここでいう IR は、単なる内部実装の都合ではなく、次の複数レイヤのあいだで共有される**意味論の中心表現**である。

- `.hb` パーサ
- CLI の設定マージ
- AI 生成
- verifier
- codegen
- 将来の module / workspace 構造

本仕様の目的は、「パース後に何が確定していて、何が未解決なのか」を明示し、`Hammurabi` を DSL 群から言語基盤へ進めることである。

### 1.2 What This IR Is Responsible For

IR 層は次を担う。

- `.hb` の構文結果を、**意味が解決された形**に正規化する
- `hb gen` / `hb ai` / `hb check` のコマンド差を吸収し、後段に共通の入力を渡す
- goal 単位、ファイル単位、将来の project 単位の構造を表す
- `needs_ai`, `depends_on`, `model_pin`, `lang_specified` などの実行意味を明示する
- verifier / codegen / AI に必要な情報を、一箇所で確定させる

### 1.3 What This IR Is Not Responsible For

IR 層は次を直接は担わない。

- `.hb` の文字単位・行単位パースそのもの
- CLI 引数の生文字列解析
- 特定出力言語向けの最終コード生成詳細
- Z3 専用の低レベル AST 全体
- proof の永続化フォーマットそのもの

つまり IR は、**構文木**でも**最終出力**でもなく、そのあいだの「言語としての意味表現」である。

## 2. Design Goals

### 2.1 Single Source of Truth After Parsing

現状のコードでは、意味の確定が複数箇所に散っている。

- `ParseResult`
- `ParsedGoal`
- `ContractualGoal`
- `hb.rs` の設定マージ
- `ai_gen` の goal 展開

Phase 2 では、**パース後は IR を唯一の真実源にする**ことを目標とする。

### 2.2 Shared Core for Verifier, Codegen, AI, and CLI

現在は、各層が少しずつ別の見え方で goal を扱っている。

- verifier は `ContractualGoal`
- codegen も `ContractualGoal`
- CLI は `ParseResult` + `HammurabiConfig`
- AI は `.hb` を再パースして `ContractualGoal`

IR はこれらの共通核となるべきである。

### 2.3 Explicit Representation of Resolved Semantics

IR で明示すべき代表例は次のとおり。

- `lang` は明示されたか、既定値か
- `agent` / `model` / `api_key` はどの層から来たか
- goal は完全手書きか、AI 補完待ちか
- 依存先 goal はどれか
- この goal は検証対象か、AI 展開後に検証すべきか

「いまその値がそこにある」だけでなく、**なぜそこにあるのか**を保持できる構造が望ましい。

### 2.4 Stable Boundary for Future Language Growth

今後 `Hammurabi` が module, import, package, workspace を持つようになるなら、IR はその拡張の受け皿になる必要がある。

したがって、IR は現状の `1 file -> many goals` に閉じすぎず、将来の project graph へ自然に拡張できる境界で設計する。

## 3. Position in the Pipeline

### 3.1 End-to-End Flow

現行の概念フローは次のように整理できる。

```text
.hb text / natural language
  ↓
parser / ai_gen
  ↓
ParseResult / ContractualGoal
  ↓
config merge / semantic resolution
  ↓
IR
  ↓
verifier / codegen / proof_store / future tooling
```

Phase 2 での目標は、`ParseResult` と `ContractualGoal` の直後に IR を挿入し、**後段が直接 parser 由来の構造へ依存しないようにする**ことである。

### 3.2 Relationship to `.hb` Parsing

`.hb` パーサは、入力を**寛容に読む**責務を持つ。

- ブロック構文を読む
- コメント規則を適用する
- `intent`, `examples`, `settings` を抽出する
- エラーを収集しつつ可能な限り継続する

一方 IR 層は、パース済み結果から**意味を確定させる**責務を持つ。

### 3.3 Relationship to Execution-Time Configuration

設定優先順位は `docs/config-spec.md` に従う。

```text
CLI > .hb > config.hb > .env > environment variables
```

IR は、この解決済み設定の snapshot を goal / file / command 単位で保持すべきである。

### 3.4 Relationship to Verification and Code Generation

verifier と codegen は、将来的には `ParseResult` ではなく IR を直接入力に取るべきである。

これにより、

- 設定マージ
- AI 展開状態
- goal graph
- 診断と span の対応

が各所で再計算されずに済む。

## 4. Why a Dedicated IR Is Needed

### 4.1 Limits of Using `ParseResult` Directly

`ParseResult` は構文結果としては有用だが、意味論の中心には向かない。

理由:

- `lang_specified` のような presence 情報はあるが、設定の最終解決はまだ
- `errors` を持つ寛容パーサ結果であり、後段の「正規化済み入力」とは性質が違う
- `goals` は parser 都合の構造 (`ParsedGoal`) を含む

### 4.2 Limits of Using `ContractualGoal` Directly

`ContractualGoal` は「何を満たすか」を表す中核型だが、それだけでは足りない。

足りない代表情報:

- human label
- `needs_ai`
- `intent`
- source span
- 設定解決結果
- diagnostics との対応
- goal graph 上の位置

つまり `ContractualGoal` は**契約本体**であり、IR 全体ではない。

### 4.3 Problems Caused by Implicit Resolution

現在の実装では、次が各所に散っている。

- `.hb` 設定と `config.hb` のマージ
- goal 単位の `model_pin`
- `needs_ai` な goal の展開
- verifier に渡すべき goal の選別

この状態では、機能追加のたびに「どこで意味を決めるか」がぶれやすい。IR はそのぶれを止める役割を持つ。

## 5. IR Boundaries

### 5.1 Input to the IR Layer

IR 層の入力候補は次の 2 系統である。

1. `parse_hb()` が返す `ParseResult`
2. `hb ai` や `needs_ai` 展開の結果として得られる `ContractualGoal`

加えて、コマンドに応じた解決済み設定も必要になる。

### 5.2 Output from the IR Layer

IR 層の出力は、最低限次へ渡せる必要がある。

- verifier
- codegen
- AI 展開器
- proof store
- 将来の formatter / analyzer / dependency graph

### 5.3 Where Parsing Ends and Semantic Resolution Begins

次を目安に境界を引く。

- parser の責務: 読める形にする
- IR の責務: 実行・検証・生成に使える形にする

たとえば `lang_specified` は parser が拾うが、**最終的に `lang` が何であるべきか**は IR 構築時に確定する。

## 6. Proposed IR Layers

### 6.1 Syntax-Level Structures

これは現状の `ParseResult` / `ParsedGoal` に近い層である。  
将来的には parser モジュール内部に閉じたい。

### 6.2 Resolved Goal-Level IR

各 goal を、設定・AI 状態・依存関係込みで正規化した層。

これは Phase 2 の最小成果物になりうる。

### 6.3 File-Level / Project-Level IR

1 ファイル内の複数 goal を束ねた単位。  
さらに将来は multi-file の project IR へ拡張する。

### 6.4 Optional Verification-Oriented Lowering

必要に応じて、IR から verifier 向けの lower された構造を作る。  
ただし最初から Z3 専用の低レベル IR を中心に据える必要はない。

## 7. Core IR Entities

以下は提案段階の名前であり、実装時に変更してよい。

### 7.1 `ResolvedFile`

1 つの `.hb` 入力から得られる、解決済みファイル単位の構造。

想定フィールド:

- `goals: Vec<ResolvedGoal>`
- `config: ResolvedConfig`
- `diagnostics`
- `source_map`

### 7.2 `ResolvedGoal`

後段の共通入力となる中心ノード。

想定フィールド:

- `identity`
- `contract`
- `label`
- `intent`
- `examples`
- `depends_on`
- `ai_state`
- `effective_config`
- `source`

### 7.3 `ResolvedSettings`

`require`, `ensure`, `invariant`, `forbid` を正規化した束。  
現状は `ContractualGoal` に内包されているが、必要なら独立型に切り出してよい。

### 7.4 `ResolvedConfig`

設定の最終スナップショット。  
どの層から来たかを provenance 付きで持つ案も有効である。

### 7.5 `GoalGraph`

goal 間依存の有向グラフ。

現状は `depends_on: Vec<String>` だが、IR では参照解決済みの関係を持てるとよい。

### 7.6 `VerificationUnit`

verifier に渡す単位。  
`ResolvedGoal` をそのまま渡せるなら専用型は不要だが、backend ごとに lower したいなら設ける。

### 7.7 `CodegenUnit`

codegen に必要な最小単位。  
goal 本体 + 有効言語 + 補助メタデータの形が基本となる。

## 8. Required Information in the IR

### 8.1 Goal Identity

goal には少なくとも

- human-readable な `name`
- stable な `id`

の 2 種類が必要である。

### 8.2 Goal Name and Human Label

`goal: safe_div "ゼロ除算を防ぐ"` のようなケースでは、識別子と表示ラベルを分離して保持する必要がある。

### 8.3 Inputs and Output Types

`inputs` と `output` は、現状文字列ベースの部分も含む。  
Phase 2 ではまず raw string を保持し、必要なら後に型表現へ段階的に進む。

### 8.4 Preconditions, Postconditions, Invariants, and Forbids

契約本体の中心情報である。これは最終的に `ContractualGoal` と互換である必要がある。

### 8.5 Intent and Examples

AI と人間の両方にとって重要な補助情報。  
IR では optional metadata ではあるが、消してはならない。

### 8.6 Dependencies Between Goals

`depends_on` は将来の project graph の入口なので、早い段階から first-class に扱うべきである。

### 8.7 Resolved Configuration Snapshot

各 goal / file が、どの `agent`, `model`, `lang` で扱われるべきかを保持する。

### 8.8 Source Spans and Diagnostics Mapping

LSP や error reporting を維持するには、IR ノードが元の `.hb` のどこから来たかを追跡できる必要がある。

## 9. Identity Model

### 9.1 Goal Name vs Stable `id`

- `name`: 実装や codegen に使う名前
- `id`: 依存解決や長期参照に使う安定識別子

両者は同一でもよいが、同一である必要はない。

### 9.2 Auto-Generated IDs

`goal: "自然言語のみ"` のようなケースでは、現在は slug 生成が行われる。  
IR では「自動生成された識別子」であることを必要なら保持できるようにする。

### 9.3 Uniqueness Rules

少なくとも file 単位では `id` の重複を禁止する方向が望ましい。  
将来の project 単位では namespace を含む規則が必要になる。

### 9.4 Dependency References

`depends_on` は string list ではなく、最終的には解決済み参照に落とすのが望ましい。

## 10. Configuration in the IR

### 10.1 What Should Be Carried Into IR

最低限:

- `agent`
- `api_key` の有無または解決結果
- `model`
- `lang`
- `verifier` 選択

### 10.2 Raw vs Resolved Configuration

IR では基本的に **resolved** な設定を持つべきである。  
ただし provenance が必要なら、raw source 情報を別途持つ。

### 10.3 Command-Specific Resolution (`hb gen`, `hb ai`, `hb check`)

IR は command-aware であるべきだが、コマンド分岐を全て型に埋め込む必要はない。  
最初は「resolved config snapshot + mode」の形で十分である。

### 10.4 `lang_specified` and Other Presence Flags

presence flag は parser の都合ではなく、意味論的に重要である。  
`lang_specified` は IR に取り込むか、IR 構築時に `ResolvedConfig` へ畳み込む必要がある。

## 11. AI-Related State in the IR

### 11.1 Human-Written vs AI-Assisted Goals

IR では、goal を少なくとも次に区別できるとよい。

- 完全手書き
- AI 展開待ち
- AI 展開済み
- AI 展開失敗

### 11.2 `needs_ai` as a Semantic State

現在の `needs_ai` は parser / CLI の境界にある。  
Phase 2 では、これを IR 上の明示的状態として扱う。

### 11.3 Goal-Level Model Pinning

`model_pin` は file-level config とは別レイヤの意味を持つ。  
IR では goal-level override として first-class に表現する。

### 11.4 Tracking AI-Expanded Results

AI が生成した `ContractualGoal` を元の goal にどう紐づけるかを定義する必要がある。  
最低限、元 goal の `id` と provenance を保持する。

## 12. Verification-Oriented Representation

### 12.1 Mapping Goals to Verification Units

全 goal が即 verifier に渡るわけではない。

- `needs_ai` は展開後に verifier へ
- 一部 goal は構文エラーで verifier 不可

したがって、IR から `VerificationUnit` を切り出す段階が必要になりうる。

### 12.2 Backend-Independent Meaning

IR 自体は Mock / Z3 に依存すべきではない。  
backend 差は lower 後に発生する。

### 12.3 Backend-Specific Lowering

`Predicate` をそのまま使える範囲は広いが、Z3 への lowering は verifier 側で行うべきである。

### 12.4 Representing Proof Results in or beside the IR

proof 結果は IR 本体に埋め込むより、sidecar 的にぶら下げる方が自然な可能性が高い。

## 13. Code Generation-Oriented Representation

### 13.1 What Codegen Needs That Parsing Does Not Provide

codegen が必要とするのは単なる goal ではなく、次である。

- 解決済み言語
- 補助ヒント
- AI 展開済みかどうか
- 型や examples の最終状態

### 13.2 Language-Neutral Intermediate Form

将来的には `codegen.rs` を分割しやすくするため、言語非依存の codegen input を IR から切り出すとよい。

### 13.3 Language-Specific Lowering Boundaries

言語固有の命名、型変換、テンプレートは codegen backend 側に残し、IR はそれ以前に留める。

## 14. Diagnostics and Traceability

### 14.1 Source Span Preservation

IR ノードは、元の `.hb` の span を辿れるべきである。

### 14.2 Mapping IR Nodes Back to `.hb`

LSP・CLI エラー・将来の explain 機能のため、IR と source の対応は重要である。

### 14.3 Error Reporting Across Phases

parser error, semantic resolution error, verification error, codegen error を段階ごとに識別できる構造が望ましい。

## 15. Invariants of the IR

### 15.1 Structural Invariants

- `ResolvedFile` は空 goal 集合を持たない、または明示的に invalid として表す
- `ResolvedGoal` は name を持つ
- 依存参照は可能なら解決済みである

### 15.2 Semantic Invariants

- `needs_ai == false` なら verifier / codegen に渡せる契約が揃っている
- `needs_ai == true` なら未展開状態である

### 15.3 Resolution Invariants

- effective config はマージ済みである
- `lang` は明示/既定の区別を誤って失っていない

### 15.4 Backend-Independent Invariants

IR 自体は Mock / Z3 / language backend に依存しない。

## 16. Conversion Rules

### 16.1 `ParseResult` -> IR

最初の導入では、`ParseResult` と解決済み `HammurabiConfig` から `ResolvedFile` を作る関数が必要になる。

### 16.2 IR -> Verifier Input

`ResolvedGoal` から verifier に必要な `ContractualGoal` または `VerificationUnit` を導く。

### 16.3 IR -> AI Prompt / AI Expansion Input

`needs_ai` goal については、依存 goal・intent・examples を踏まえた prompt input を生成する。

### 16.4 IR -> Codegen Input

goal 契約 + effective language + metadata を codegen backend へ渡す。

## 17. Migration Plan

### 17.1 Minimal First Step

最小の第一歩は、`ResolvedGoal` と `ResolvedFile` を導入し、`hb gen` の設定解決後にそれを構築することである。

### 17.2 Safe Incremental Refactor Order

推奨順:

1. `ResolvedConfig`
2. `ResolvedGoal`
3. `ResolvedFile`
4. `hb gen` からの利用
5. verifier / codegen への入力統一

### 17.3 Temporary Adapters

しばらくは

- `ParseResult -> ResolvedFile`
- `ResolvedGoal -> ContractualGoal`

の adapter を併用してよい。

### 17.4 Exit Criteria for Adopting the IR

次の条件を満たせば、IR 導入の第一段は成功とみなせる。

- `hb gen` が IR 経由で動く
- 設定解決が `hb.rs` に散っていない
- `needs_ai` / `depends_on` / `model_pin` の意味が IR 上で明示される

## 18. Testing Strategy

### 18.1 IR Construction Tests

`ParseResult` + config から正しい `ResolvedFile` が作られることをテストする。

### 18.2 Round-Trip / Integration Tests

`.hb` -> IR -> verifier / codegen の一連が壊れていないことをテストする。

### 18.3 Invariant Tests

IR の structural / semantic invariant をテストで固定する。

### 18.4 Regression Tests for Resolution Semantics

特に、設定優先順位、`lang_specified`, AI 状態遷移は regression test の中心になる。

## 19. Open Questions

### 19.1 How Much Type Semantics Belongs in the IR

型文字列のまま保持するか、独自 type IR を持つかは未確定。

### 19.2 Whether Proof Results Should Live Inside or Alongside the IR

proof は sidecar の方が自然だが、用途によっては IR に参照を持たせる価値もある。

### 19.3 How to Represent Multi-File / Module Boundaries

現状は file 単位だが、将来の module system を見越した設計が必要。

### 19.4 How to Model Future Imports, Packages, and Workspaces

project IR をいつ導入するかは Phase 2 後半の検討課題である。

## 20. Near-Term Decisions

### 20.1 Smallest Useful IR to Introduce First

最小単位は次で足りる。

- `ResolvedConfig`
- `ResolvedGoal`
- `ResolvedFile`

### 20.2 Files to Change First

- `src/bin/hb.rs`
- `src/lsp/mod.rs`
- `src/ai_gen/mod.rs`
- `src/codegen.rs`

### 20.3 What Should Stay As-Is During Phase 2

Phase 2 前半では、次を無理に変えない。

- `Predicate` AST の大幅変更
- verifier backend の内部ロジック
- codegen の各言語実装詳細

まずは「意味の中心を一箇所に集める」ことを優先する。

## 21. The Example for ResolvedConfig / ResolvedGoal / ResolvedFile 

### 21.1 `.hb` / config / CLI のマージ後に得られる解決済み設定。
```rust
/// `.hb` / config / CLI のマージ後に得られる解決済み設定。
///
/// 目的:
/// - verifier / codegen / ai_gen が同じ設定スナップショットを見る
/// - `lang_specified` のような presence 情報も失わず保持する
/// - 必要なら将来 provenance（どこから来た値か）を追加できる
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    /// 最終的に使う AI バックエンド。
    pub agent: AgentKind,

    /// 最終的に使う API キー。
    ///
    /// 既に解決済みの値を持たせる案と、
    /// 「未解決なら None のまま」にする案がある。
    /// Phase 2 前半では現状互換のため Option<String> でよい。
    pub api_key: Option<String>,

    /// 最終的に使うモデル名。
    ///
    /// `None` は「agent の既定モデルを使う」を意味してもよいが、
    /// 後段を簡単にするなら `effective_model` を別途持つ案もある。
    pub model: Option<String>,

    /// 最終的に使う出力言語。
    pub lang: TargetLang,

    /// `.hb` 側で lang が明示されていたかどうか。
    ///
    /// parser default の Rust と、明示された Rust を区別するために残す。
    pub lang_specified_in_hb: bool,
}

/// `ResolvedGoal` がどの状態にあるか。
///
/// parser 後に verifier / codegen / ai_gen へどう進めるかを決める。
#[derive(Debug, Clone, PartialEq)]
pub enum GoalResolutionState {
    /// 人間が fully-specified な goal を書いた状態。
    FullySpecified,

    /// `goal: "自然言語のみ"` などで AI 展開待ちの状態。
    NeedsAiExpansion,

    /// AI 展開済みで、以後は通常 goal と同じように扱える状態。
    AiExpanded,
}

/// source 上の位置を追跡するための最小メタデータ。
///
/// LSP や CLI エラー、将来の explain 機能に使う。
#[derive(Debug, Clone, PartialEq)]
pub struct GoalSourceInfo {
    /// 元の `.hb` ファイルパス（将来 multi-file 対応しやすいよう String で保持してもよい）。
    pub file_path: Option<String>,

    /// goal 名または goal ラベルが現れた位置。
    pub name_span: Option<Span>,

    /// parser が集めた元のエラー・警告をぶら下げたい場合の入口。
    ///
    /// Phase 2 前半では空でもよい。
    pub had_parse_errors: bool,
}

/// parser 後・設定解決後・AI 状態解決後の goal 単位 IR。
///
/// 既存コードとの整合を優先し、契約本体は `ContractualGoal` をそのまま内包する。
#[derive(Debug, Clone)]
pub struct ResolvedGoal {
    /// 契約本体。
    ///
    /// 既存の verifier / codegen と接続しやすくするため、
    /// Phase 2 前半では分解せずそのまま保持する。
    pub contract: ContractualGoal,

    /// `goal: name "label"` や `goal: "自然言語のみ"` のラベル。
    pub label: Option<String>,

    /// `intent: """..."""` の設計意図。
    pub intent: Option<String>,

    /// この goal が現在どの段階にあるか。
    pub state: GoalResolutionState,

    /// goal 単位のモデル pin。
    ///
    /// 既存の `contract.model_pin` と重複しうるが、
    /// 後段で参照しやすいよう top-level に持たせる案。
    pub model_pin: Option<String>,

    /// goal が依存する他 goal の stable id 群。
    ///
    /// 既存の `contract.depends_on` と重複しうるが、
    /// graph 解決の入口として top-level に持つ。
    pub depends_on: Vec<String>,

    /// この goal に対して適用される解決済み設定。
    ///
    /// file 全体と同じことが多いが、
    /// 将来 goal 単位 override が増えても自然に対応できる。
    pub effective_config: ResolvedConfig,

    /// source traceability。
    pub source: GoalSourceInfo,
}

/// 1 ファイルぶんの意味論的に解決済み IR。
///
/// parser の寛容な結果を保持する `ParseResult` とは異なり、
/// verifier / codegen / ai_gen が共通で読む単位を意図する。
#[derive(Debug, Clone)]
pub struct ResolvedFile {
    /// 入力ファイルパス。
    pub file_path: Option<String>,

    /// このファイルに対するファイル単位の解決済み設定。
    ///
    /// `ResolvedGoal.effective_config` のベースになる。
    pub file_config: ResolvedConfig,

    /// このファイルから得られた goal 群。
    pub goals: Vec<ResolvedGoal>,

    /// parser / resolution の段階で得られた診断。
    ///
    /// Phase 2 前半では `ParseError` をそのまま保持してよい。
    pub diagnostics: Vec<ParseError>,
}
```