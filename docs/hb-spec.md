# Hammurabi `.hb` Specification

## 1. Scope

この文書は、`Hammurabi` における `.hb` ファイルの**構文**、**受理条件**、および一部の**基本意味**を定義する。


この文書の目的は次の 3 つである。

- `hb gen` / `hb check` / `hb ai` の共通入力形式を固定する
- LSP、AI 出力、README、テストの基準文書になる
- 実装依存の暗黙ルールを仕様として明文化する

実装上の主な参照先は `src/lsp/mod.rs` および `src/lsp/brace.rs` だが、**規範としては本仕様を優先**する。

## 2. Purpose

`.hb` は、関数や処理の実装そのものではなく、**満たすべき契約**を宣言的に記述する DSL である。

`.hb` で表される中心概念は `ContractualGoal` であり、主に次を記述する。

- goal 名または自然言語ラベル
- 入出力の型
- `require` / `ensure` / `invariant` / `forbid` による制約
- 例 (`examples`)
- 補助メタデータ (`id`, `depends_on`, `model`, `intent`)

## 3. Conformance

`.hb` を扱う処理系は、少なくとも次を満たすこと。

- 本仕様に定義された構文を受理する
- 本仕様でエラーとした入力に対し、診断を返す
- コメント規則、URL 規則、トップレベル制約を実装と一致させる

本仕様で「未規定」とした箇所は、将来バージョンで拡張可能とする。

## 4. File Model

- 1 つの `.hb` ファイルは、**0 個以上のファイルレベル設定**と、**1 個以上の goal 定義**からなる。
- goal 定義の正式構文は **ブロック構文** とする。
- ブロック外に書けるのは **ファイルレベル設定**のみとする。
- 1 ファイルに複数 goal を含めてよい。

### 4.1 Valid High-Level Shapes

有効な大枠は次のいずれかを含む。

1. ブロック構文
2. トップレベル `goal:` と `settings:` の組

ただし、**ファイル全体として goal が 1 つも得られない場合はエラー**とする。

## 5. Lexical Rules

### 5.1 Whitespace

- 行頭・行末の空白は無視する。
- 空行は無視する。
- インデントの深さ自体に意味はない。

### 5.2 Comment Rules

- 行頭の `#` は、その行全体をコメントとみなす。
- 行中の `//` 以降は、原則として行コメントとみなす。
- ただし **バッククォート `` `...` `` 内の `//` はコメント開始とみなさない。**

例:

```hb
goal: sample "説明" // これはコメント
intent: """
  `https://example.com` を参考にする
"""
```

### 5.3 URL Rule

`https://...` のように `//` を含む URL を、行コメントにしたくないテキストとして埋め込む場合は、**バッククォートで囲まなければならない。**

許可:

```hb
intent: """
  `https://example.com/path` の仕様に準拠する
"""
```

非推奨かつ意図どおりに読まれない例:

```hb
intent: """
  https://example.com/path の仕様に準拠する
"""
```

### 5.4 Delimiters

- フィールド区切りのコロンとして、ASCII の `:` または全角の `：` を受理してよい。
- `config` 内およびトップレベルフィールド間の末尾カンマは省略可能とする。

### 5.5 Identifiers

以下は識別子として扱われる。

- goal 名
- `id`
- `depends_on` の各要素
- 述語内の変数名

識別子の詳細な字句規則は現実装に従う。少なくとも、**空文字であってはならない**。

## 6. Formal Grammar

この節では `.hb` の正式文法を EBNF 風に示す。

```text
document          ::= file_setting* top_level_item*

file_setting      ::= setting_line
setting_line      ::= ("agent" | "model" | "lang" | "api_key") value_line

top_level_item    ::= outer_block
                    | top_goal_pair

outer_block       ::= "{" field* "}"
field             ::= config_field
                    | define_field
                    | goal_field
                    | settings_field

config_field      ::= "config" ":" config_obj
define_field      ::= "define" ":" define_obj
goal_field        ::= "goal" ":" goal_rhs
settings_field    ::= "settings" ":" settings_body

config_obj        ::= "{" config_pair* "}"
config_pair       ::= ("agent" | "model" | "lang" | "api_key") ":" value

define_obj        ::= "{" define_member* "}"
define_member     ::= id_field
                    | depends_on_field
                    | model_field
                    | intent_field
                    | goal_field
                    | inputs_field
                    | output_field
                    | examples_field
                    | settings_field

id_field          ::= "id" ":" identifier
depends_on_field  ::= "depends_on" ":" id_list
model_field       ::= "model" ":" value
intent_field      ::= "intent" ":" triple_quoted_text
inputs_field      ::= "inputs" ":" typed_params
output_field      ::= "output" ":" type_expr
examples_field    ::= "examples" ":" example_block

top_goal_pair     ::= goal_field settings_field

settings_body     ::= "[" setting_stmt* "]"
                    | "[" "{" setting_stmt* "}" "]"

setting_stmt      ::= require_stmt
                    | ensure_stmt
                    | invariant_stmt
                    | forbid_stmt

require_stmt      ::= "require" ":" predicate
ensure_stmt       ::= "ensure" ":" predicate_or_atom
invariant_stmt    ::= "invariant" ":" predicate_or_atom
forbid_stmt       ::= "forbid" ":" forbidden_pattern
```

注記:

- 上記は**概念文法**であり、空白・改行・コメント除去後の実際の行走査は実装に委ねる。
- `goal_field` と `settings_field` はトップレベルで対になるか、`define` 内に含まれる。
- `goal: "自然言語のみ"` は一部条件下で `settings` 省略を許す。

## 7. Top-Level Rules

### 7.1 Allowed Top-Level Keys Outside Braces

ブロック外に書けるのは、次のファイルレベル設定のみとする。

- `agent`
- `model`
- `lang`
- `api_key`

### 7.2 Disallowed Top-Level Keys Outside Braces

ブロック外で次を書くのはエラーとする。

- `define`
- `goal`
- `settings`
- その他の未知キー

ただし、最外殻ブロック内では `config`, `define`, `goal`, `settings` を認める。

## 8. File-Level Configuration

ファイルレベル設定は、`.hb` 全体に適用されるメタデータである。

### 8.1 Supported Keys

- `agent`
- `api_key`
- `model`
- `lang`

### 8.2 Locations

次の 2 箇所で指定できる。

1. ブロック外の設定行
2. 最外殻ブロック内の `config: { ... }`

### 8.3 Merge Semantics Inside `.hb`

同一 `.hb` 内で設定が複数回現れた場合は、**後に読まれた値で上書き**してよい。

### 8.4 `lang` Presence

`.hb` のパーサは内部的に既定値として `rust` 相当を持つことがあるが、**`.hb` 側で `lang` が明示されたかどうか**は別に扱わなければならない。

したがって、次を区別する。

- `lang` が明示された
- `lang` は明示されておらず、既定値が見えているだけ

この区別は設定マージ仕様で重要になる。

## 9. `config` Section

`config` は最外殻ブロック内で使うファイルレベル設定セクションである。

```hb
{
  config: {
    agent: openai
    model: gpt-4o
    api_key: $OPENAI_API_KEY
    lang: python
  }
}
```

### 9.1 Accepted Keys

- `agent`
- `api_key`
- `model`
- `lang`

### 9.2 Rejected Keys

上記以外のキーはエラーとして扱う。

## 10. `define` Section

`define` は、1 つの goal をまとめて記述する正式なブロックである。

```hb
define: {
  id: sample_v1
  goal: sample_goal "説明"
  inputs: x: i32
  output: i32
  settings: [
    require: InRange(x, 0, 10)
    ensure: result_ok
  ]
}
```

### 10.1 Supported Members

- `id`
- `depends_on`
- `model`
- `intent`
- `goal`
- `inputs`
- `output`
- `examples`
- `settings`

### 10.2 Required Members

`define` では、少なくとも次が必要である。

- `goal`

通常はさらに `settings` が必要だが、`goal: "自然言語のみ"` 形式では `settings` 省略を許し、AI 補完対象として扱ってよい。

### 10.3 Duplicate Handling

同一 `define` 内で重複フィールドが現れたときの正確な扱いは実装依存になりうるが、少なくとも**あいまいさを生む重複は診断対象**とすべきである。

## 11. `goal` Field

`goal` は契約対象の中心となる宣言である。

### 11.1 Accepted Forms

次の 3 形式を受理する。

1. 識別子のみ

```hb
goal: safe_division
```

2. 識別子 + 自然言語ラベル

```hb
goal: safe_division "ゼロ除算を防ぐ安全な除算"
```

3. 自然言語のみ

```hb
goal: "2つの整数を安全に割り算する"
```

### 11.2 Meaning

- 識別子がある場合、それを goal 名として用いる
- 自然言語のみの場合、処理系は内部的に識別子を生成してよい
- 自然言語のみの goal は AI 補完対象になりうる

## 12. `intent` Field

`intent` は設計意図や補助説明を表す任意フィールドである。

### 12.1 Syntax

`intent` は **トリプルクォート文字列**で記述する。

単行:

```hb
intent: """ゼロ除算を物理的に排除する"""
```

複数行:

```hb
intent: """
  ゼロ除算を物理的に排除し、
  システムの停止を防ぐ。
"""
```

### 12.2 Comment Interaction

`intent` の各行にもコメント規則は適用される。URL を含める場合は、`5.3 URL Rule` に従うこと。

## 13. `inputs` and `output`

### 13.1 `inputs`

`inputs` は入力パラメータ列を記述する。

例:

```hb
inputs: dividend: i64, divisor: i64
```

### 13.2 `output`

`output` は返り値の型を記述する。

例:

```hb
output: Result<i64, String>
```

型式の詳細な意味論は現状、出力先コード生成器に依存する部分がある。

## 14. `examples`

`examples` は入出力例を列挙する任意フィールドである。

```hb
examples: [
  - (10, 2) => Ok(5)
  - (7, 0)  => Err("division by zero")
]
```

### 14.1 Purpose

- コード生成時のヒント
- 将来の自動テスト生成の材料
- 意味の補助的固定

## 15. `settings`

`settings` は述語制約の集合である。

### 15.1 Allowed Statements

- `require`
- `ensure`
- `invariant`
- `forbid`

### 15.2 Example

```hb
settings: [
  require: InRange(n, 0, 10)
  ensure: n_ok
  forbid: UnprovenUnwrap
]
```

### 15.3 Meaning

- `require`: 入力側の事前条件
- `ensure`: 出力または結果側の事後条件
- `invariant`: 常に保つべき条件
- `forbid`: 生成・実装・検証上の禁止パターン

## 16. Predicates and Forbidden Patterns

述語本体の完全な意味論は別仕様へ分離可能だが、現状少なくとも次を想定する。

### 16.1 Representative Predicates

- `NonNull(var)`
- `InRange(var, min, max)`
- `Not(p)`
- `And(p, q)`
- `Or(p, q)`
- `Implies(p, q)`
- `ForAll(var, body)`
- `Exists(var, body)`
- `Equals(lhs, rhs)`
- `Regex(var, "pattern")`
- 名前付きアトム述語 (`result_is_finite`, `n_ok` など)

### 16.2 Representative Forbidden Patterns

- `RuntimeNullCheck`
- `UnprovenUnwrap`
- `ImplicitCoercion`
- `NonExhaustiveBranch`
- その他 `ForbiddenPattern` 列挙に定義されたもの

## 17. Error Conditions

少なくとも次は診断対象とする。

- ブロック外の未知キー
- `define` に `goal` がない
- `settings` が必要な形式で欠落している
- `intent` のトリプルクォートが閉じていない
- `config` の未知キー
- 解析不能な predicate
- goal が最終的に 1 つも得られない

## 18. Relationship to Configuration Precedence

`.hb` 内の `agent`, `api_key`, `model`, `lang` は、実行時設定マージの一部として扱われる。

詳細は `docs/config-spec.md` を参照。

少なくとも `hb gen` では、現行仕様上:

```text
CLI > .hb > config.hb > .env > environment variables
```

とする。

## 19. Relationship to AI

`.hb` は AI 出力の受け皿でもあり、人間の手書き仕様でもある。

したがって処理系は、AI が生成した `.hb` に対しても人間が書いた `.hb` と同じ構文規則を適用しなければならない。

## 20. Examples

### 20.1 Full Example

```hb
agent: openai
lang: rust

{
  config: {
    model: gpt-4o
  }

  define: {
    id: safe_divide_v1
    intent: """
      ゼロ除算を物理的に排除する。
      `https://example.com/spec` を参考にする。
    """
    goal: safe_division "安全な整数除算"
    inputs: dividend: i64, divisor: i64
    output: Result<i64, String>
    examples: [
      - (10, 2) => Ok(5)
      - (7, 0)  => Err("division by zero")
    ]
    settings: [
      require: NonNull(divisor)
      ensure: result_is_finite
      forbid: UnprovenUnwrap
    ]
  }
}
```

## 21. Versioning and Future Work

この文書は現行実装に基づく初版仕様である。

将来の改訂候補:

- 識別子の厳密字句規則
- 型式の正式文法
- predicate の完全意味論
- `examples` の厳密構文
- module / import 構文
- goal 間依存の正式意味