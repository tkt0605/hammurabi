# Hammurabi Verification Model

## 1. Scope

この文書は、`Hammurabi` における**検証モデル**、**検証バックエンドの責務**、**ProofToken の意味**、および **Mock / Z3 の差**を定義する。

対象は主に次の概念である。

- `Verifier` トレイト
- `MockVerifier`
- `Z3Verifier`
- `ProofToken`
- `ProofStatus`
- `ConstitutionalReport`
- `LogicRail<T>` と制約検証の関係

主な実装参照先は `src/compiler/verifier.rs` および `src/lang/rail.rs` だが、**規範としては本仕様を優先**する。

## 2. Purpose

`Hammurabi` は、契約 DSL から得られた制約や述語を、単にコメントや生成ヒントとして扱うのではなく、**検証可能な意味論的対象**として扱う。

本仕様の目的は次のとおり。

- 何を「検証」と呼ぶかを固定する
- Mock と Z3 の役割差を明確にする
- `ProofToken` の信頼境界を定義する
- CLI、LSP、proof store、codegen が同じ期待値を共有できるようにする

## 3. Architecture

検証アーキテクチャは、バックエンド差し替え可能な構造を取る。

```text
Verifier (trait)
  +-- MockVerifier    -- 開発・テスト用
  +-- Z3Verifier      -- 厳密 SMT 検証用（feature: z3-backend）
```

この設計により、`Hammurabi` は次を両立する。

- 依存のない最小ビルド
- 厳密検証が必要なときだけ Z3 を有効化
- 共通 API (`Verifier`) による呼び出し統一

## 4. Core Concepts

### 4.1 Constraint Verification

値 `T` が与えられたとき、`Constraint` の集合を満たすかを確認すること。

成功時は `ProofToken` を返す。

### 4.2 Goal Verification

`ContractualGoal` が `Hammurabi` の hard constraints に適合しているかを確認すること。

結果は `ConstitutionalReport` として表現する。

### 4.3 Invariant Proving

ある不変条件が、与えられた事前条件のもとで成立するかを判定すること。

結果は `ProofStatus` で表す。

## 5. Verifier Trait Contract

`Verifier` は、検証バックエンドの共通抽象である。

少なくとも次の契約を満たすこと。

- `verify_constraints` が `Ok(token)` を返すとき、そのバックエンドの意味において制約集合は受理されたとみなす
- `verify_goal` は `ContractualGoal` の妥当性・憲法適合性を診断する
- `prove_invariant` は不変条件に対する `Proven` / `Disproven` / `Unknown` を返す
- エラーは `VerificationError` の具体的な種別で表現される

### 5.1 Important Note

`Ok(token)` は常に「数学的に完全な証明」を意味するわけではない。  
**その token がどの backend から発行されたか**によって、保証の強さは異なる。

この点は `ProofToken.backend` により明示される。

## 6. ProofToken

### 6.1 Meaning

`ProofToken` は、**ある制約集合が特定バックエンドによって受理されたこと**を示す証明印である。

### 6.2 Fields

- `constraint_hash`
- `backend`

### 6.3 Security Boundary

`ProofToken` のコンストラクタは `verifier` モジュールに閉じており、外部から任意生成できない。

したがって、少なくとも API 境界としては、

- verifier だけが token を発行できる
- token は対応する制約集合と結びついている
- hash 不一致は改竄として扱える

### 6.4 Guarantee Level

`ProofToken` の保証は backend に依存する。

- `Mock` backend の token  
  -> 構造的・局所的な検証に通過したことを示す
- `Z3Smt` backend の token  
  -> Z3 のモデルとエンコーディングに基づく厳密検証に通過したことを示す

## 7. VerifierBackend

現行 backend 種別は次の 2 つ。

- `Mock`
- `Z3Smt`

この区別は表示上の都合ではなく、**意味論的に重要**である。

`LogicRail` や `proof_store` を利用する側は、必要に応じて backend を考慮しなければならない。

## 8. MockVerifier

### 8.1 Purpose

`MockVerifier` は、依存なしで動作する**開発・テスト用バックエンド**である。

目的は次のとおり。

- 最小ビルドで検証パスを動かす
- 構造不良や明らかな矛盾を早期に発見する
- テスト時の高速フィードバックを提供する

### 8.2 What It Checks

現行実装ベースでは、少なくとも次を扱う。

- `InRange` の構造整合性 (`min <= max`)
- 数値型に対する `InRange` の具体値チェック
- `NonEmpty` の空判定
- `Regex` のパターン妥当性と、具体文字列に対するマッチ確認
- 一部 `ConsistentWith` のパターン的解釈
- `ContractualGoal` の well-formedness と禁止パターンの基本確認

### 8.3 What It Does Not Check

`MockVerifier` は**一般の述語論理の充足性証明をしない**。

特に次は限定的または未評価である。

- `Predicate` 全体の厳密充足判定
- 量化子 (`ForAll`, `Exists`) の一般証明
- シンボリック変数を含む本格的な SMT 推論
- 複数制約間の完全な論理整合性

### 8.4 Interpretation of Success

`MockVerifier` の `Ok(token)` は、

**「この入力は、開発用の構造的・具体値ベースの検証には通過した」**

ことを意味する。

これは有用だが、**完全な形式証明と同一視してはならない。**

### 8.5 Recommended Usage

- ローカル開発
- DSL や codegen の回帰テスト
- AI を使わない最小構成
- CI の軽量パス

## 9. Z3Verifier

### 9.1 Purpose

`Z3Verifier` は、Z3 SMT ソルバーを用いる**厳密検証バックエンド**である。

利用には次が必要である。

- Cargo feature: `z3-backend`
- Z3 ライブラリの利用可能な環境

### 9.2 What It Checks

現行実装ベースでは、少なくとも次を扱う。

- `Predicate` AST の Z3 へのエンコード
- `InRange`, `And`, `Or`, `Implies`, `Equals`
- 量化子 (`ForAll`, `Exists`)
- `prove_invariant` における反証・証明
- `verify_constraints` における SAT / UNSAT / UNKNOWN 判定

### 9.3 Current Limitations

現行実装には、少なくとも次の制限がある。

- `Regex` は完全な文字列理論としては扱わず、具体値がある場合に Rust 側で評価する
- `NonNull` は Z3 上では補助的シンボルとして表現される
- 一部の高水準制約は完全な意味論へ落ち切っていない
- Z3 自体が `Unknown` を返す場合がある

したがって Z3 backend も、**エンコーディングされたモデルの範囲で厳密**なのであって、未表現の意味まで自動的に証明するわけではない。

### 9.4 Interpretation of Success

`Z3Verifier` の `Ok(token)` は、

**「現行の制約エンコーディングと Z3 の判定において、制約集合が充足可能または証明可能である」**

ことを意味する。

### 9.5 Interpretation of Failure

主な失敗形は次のとおり。

- `Unsatisfiable`
- `SolverError`
- `MalformedGoal`

`Unknown` は、必ずしも偽を意味しない。  
それは「現 backend / solver 条件では確定できなかった」ことを意味する。

## 10. Mock vs Z3

### 10.1 Summary Table

| 観点 | Mock | Z3 |
|------|------|----|
| 主目的 | 開発用・軽量チェック | 厳密検証 |
| 依存 | なし | Z3 必要 |
| 一般述語論理 | 限定的 | 対応 |
| 量化子 | 実質未対応 | 対応 |
| 具体値チェック | 強い | 強い |
| solver の `Unknown` | なし | ありうる |
| token の意味 | 構造的受理 | SMT ベース受理 |

### 10.2 Practical Rule

次の期待値で使い分けるのが望ましい。

- **Mock に通る**  
  -> DSL・構造・具体例として大きな破綻は少ない
- **Z3 に通る**  
  -> 現在の論理モデルの範囲で、より強い保証がある

したがって、

**Mock 通過 = 形式証明完了**

とは解釈してはならない。

## 11. ProofStatus

不変条件証明の結果は `ProofStatus` で表す。

- `Proven`
- `Disproven { counterexample }`
- `Unknown`

### 11.1 Meaning

- `Proven`  
  指定 backend の意味で成立が証明された
- `Disproven`  
  反例が見つかった
- `Unknown`  
  証明も反証も確定しなかった

### 11.2 Backend Difference

`Mock` では、現行実装上 `True` / `False` などの単純ケース以外は `Unknown` に寄りやすい。  
`Z3` では、量化子や論理式に対してより強い判定が可能である。

## 12. ConstitutionalReport

`verify_goal` の結果は `ConstitutionalReport` によって表される。

主な観点:

- `exhaustive`
- `null_safety`
- `no_forbidden`
- `violations`
- `proof_backend`

### 12.1 Purpose

これは単なる bool ではなく、**goal が Hammurabi の hard constraints にどの程度適合しているか**を説明するレポートである。

### 12.2 Interpretation

`report.is_compliant()` は、その backend の観点で問題が見つからなかったことを意味する。

ここでも backend 差は重要であり、Mock の compliant は Z3 の compliant と同一ではない。

## 13. VerificationError

検証失敗は `VerificationError` の具体バリアントで表す。

代表例:

- `Unsatisfiable`
- `NonExhaustiveBranch`
- `ProofTampered`
- `ForbiddenPatternDetected`
- `MalformedGoal`
- `SolverError`
- `ConstitutionViolation`

### 13.1 Design Principle

`Hammurabi` は Zero-Ambiguity 原則に従い、catch-all 的な曖昧エラーを避ける。  
したがって検証失敗は、できるだけ具体的な原因種別として返すべきである。

## 14. Constraint Verification Semantics

### 14.1 Input

`verify_constraints` は次を受け取る。

- 値 `T`
- 制約列 `&[Constraint]`

### 14.2 Output

- 成功 -> `ProofToken`
- 失敗 -> `VerificationError`

### 14.3 Semantics

制約検証は、

- 具体値に対する即時評価
- 制約構造の整合性確認
- backend に応じたシンボリック推論

の組み合わせで構成される。

### 14.4 Hash Binding

成功時に返される `ProofToken` は、制約列のハッシュと結びついている。  
後続処理は、このハッシュにより token と制約集合の整合性を確認できる。

## 15. Goal Verification Semantics

### 15.1 Input

`verify_goal` は `ContractualGoal` を受け取る。

### 15.2 Purpose

これは「ある値が制約を満たすか」ではなく、**goal そのものが妥当な仕様として成立しているか**を見る操作である。

### 15.3 Typical Checks

現行設計上、少なくとも次が対象になる。

- goal が well-formed か
- forbidden pattern が適切に宣言されているか
- 事前条件・事後条件の基本整合性
- 一部 hard constraints への適合

## 16. Invariant Proving Semantics

### 16.1 Goal

`prove_invariant(pre, inv)` は、「事前条件のもとで invariant が成り立つか」を調べる。

### 16.2 Z3 Interpretation

Z3 backend では、概ね

```text
pre ∧ ¬inv
```

が UNSAT かどうかで成立を判定する。

### 16.3 Mock Interpretation

Mock backend では、簡易判定の範囲にとどまる。

## 17. Relationship to LogicRail

`LogicRail<T>` は、値・制約・proof を束ねる意味論的コンテナである。

### 17.1 Construction Rule

`LogicRail` は verifier による検証を経てのみ生成されるべきである。

### 17.2 Important Consequence

`LogicRail` の信頼性は、最終的に `ProofToken.backend` に依存する。

つまり、

- Mock backend で構築された rail
- Z3 backend で構築された rail

は、同じ API で扱えても**保証の強度は同じではない**。

## 18. Relationship to Proof Store

`proof_store` は `ProofToken` や制約情報を永続化するが、そこに保存されるのは backend 付きの証明記録である。

したがって保存・共有される proof は、単に「証明済み」ではなく、

- どの backend で
- どの制約ハッシュに対して
- いつ発行されたか

を含めて解釈しなければならない。

## 19. Relationship to CLI

CLI 上の `--verifier` 指定は、検証バックエンドの選択に対応する。

### 19.1 `hb check`

`hb check` は検証器を明示的に切り替える主要入口である。

### 19.2 `hb gen`

`hb gen --verifier z3` は、コード生成前に仕様を Z3 で検証する。  
違反があればコード生成を中止する。

### 19.3 Interpretation

CLI 利用者は、backend の違いを「速度差」ではなく、**保証差**として理解すべきである。

## 20. Operational Guidance

推奨運用は次のとおり。

- 日常開発: Mock
- リリース前または厳密性が必要な確認: Z3
- proof の保存や外部共有: backend を明示した上で行う

## 21. Examples

### 21.1 Mock is useful but weaker

`MockVerifier` は、範囲や空文字、正規表現などの具体値チェックに有用である。  
しかし、量化子や一般論理式の完全証明までは担わない。

### 21.2 Z3 can prove quantified invariants

`Z3Verifier` は、`ForAll` / `Exists` を含む述語に対して `ProofStatus::Proven` や `Disproven` を返せる。

### 21.3 Regex caveat

現時点では、`Regex` の厳密な SMT モデル化は限定的であり、具体文字列がある場合の Rust 側評価に依存する部分がある。

## 22. Non-Goals

本仕様は、次を現段階では保証しない。

- 全ての `Predicate` が完全に数学的意味論へ落ちていること
- 全ての制約が backend 非依存で同じ強さで検証できること
- Mock が形式証明の代替になること

## 23. Versioning and Future Work

将来の改訂候補:

- Predicate ごとの完全意味論
- Regex / String theory の強化
- cross-rail 制約の正式化
- backend ごとの差分表の詳細化
- hard constraints の独立仕様化
