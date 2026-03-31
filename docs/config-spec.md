# Hammurabi Configuration Specification

## 1. Scope

この文書は、`Hammurabi` の実行設定モデルを定義する。

対象は次のとおり。

- `config.hb`
- `.hb` ファイル内のファイルレベル設定
- CLI 引数
- `.env`
- 環境変数

この文書の目的は、**どこに設定を書けるか**、**どのコマンドでどう解決されるか**、**何が何を上書きするか**を固定することである。

主な実装参照先は `src/config.rs` と `src/bin/hb.rs` だが、**規範としては本仕様を優先**する。

## 2. Purpose

`Hammurabi` は、AI バックエンド、API キー、モデル、出力言語などの実行設定を複数の入力ソースから受け取る。

設定仕様は次を保証する。

- プロジェクト既定値を `config.hb` に置ける
- 個別の `.hb` ファイルが、必要に応じてその既定値を上書きできる
- CLI 引数が最終的な一時上書きとして働く
- `.env` / 環境変数で秘密情報を安全に注入できる

## 3. Configuration Model

設定の中心データ構造は `HammurabiConfig` である。

現行仕様で対象となる論理フィールドは次の 4 つ。

- `agent`
- `api_key`
- `model`
- `lang`

これらは、設定ソースごとに異なるタイミングで構築・上書き・解決される。

## 4. Supported Configuration Fields

### 4.1 `agent`

AI バックエンドの種類を指定する。

受理値:

- `openai`
- `anthropic`
- `mock`

実装上は別名も受理する。

- `gpt` -> `openai`
- `claude` -> `anthropic`
- `local`, `none` -> `mock`

### 4.2 `api_key`

AI API キーを指定する。

`api_key` は次のいずれかで与えられる。

- 直接文字列
- `$ENV_VAR` 形式の環境変数参照
- 解決時の `.env` / 環境変数フォールバック

### 4.3 `model`

AI モデル名を指定する。

指定されない場合、`agent` に応じた既定モデルを用いる。

### 4.4 `lang`

出力言語を指定する。

現行実装が想定する主な値:

- `rust`
- `python`
- `go`
- `java`
- `javascript`
- `typescript`

## 5. Configuration Sources

設定ソースは次の 5 層である。

1. CLI 引数
2. `.hb` ファイル内のファイルレベル設定
3. `config.hb`
4. `.env`
5. 環境変数

ただし `.env` と環境変数は、**主に `api_key` の最終解決時に効く補助ソース**であり、`agent` / `model` / `lang` を直接供給するものではない。

## 6. Default Values

設定ソースが一切与えられないときの既定値は次のとおり。

- `agent = mock`
- `api_key = None`
- `model = None`
- `lang = rust`

`model = None` は「未指定」を意味し、利用時に `agent.default_model()` で解決する。

## 7. `config.hb`

### 7.1 Purpose

`config.hb` は、**プロジェクト既定値**を表す設定ファイルである。

### 7.2 Discovery

`config.hb` の読み込みは次の順で行う。

1. CLI の `--config <path>` が与えられていれば、そのファイルを読む
2. そうでなければ、カレントディレクトリの `config.hb` を自動検出する
3. 見つからなければ、既定値を使う

### 7.3 Accepted Keys

`config.hb` で受理するキーは次のとおり。

- `agent`
- `api_key`
- `model`
- `lang`

### 7.4 Format

```text
# comment
agent: openai
api_key: $OPENAI_API_KEY
model: gpt-4o
lang: rust
```

### 7.5 Parsing Rules

- 行頭 `#` はコメント
- `key: value` 形式を用いる
- 未知キーはエラー
- `lang` は `TargetLang` として解釈する
- `api_key: $NAME` は環境変数参照として解釈する

## 8. `.hb` File-Level Configuration

### 8.1 Purpose

`.hb` 内のファイルレベル設定は、**その `.hb` ファイルだけに適用される上書き層**である。

### 8.2 Locations

次の 2 箇所で指定できる。

1. ブロック外の設定行
2. 最外殻ブロック内の `config: { ... }`

### 8.3 Accepted Keys

- `agent`
- `api_key`
- `model`
- `lang`

### 8.4 Meaning

`.hb` に書かれた設定は、`config.hb` より強く、CLI より弱い。

したがって `hb gen` においては、**ファイル単位の設定上書き**として扱う。

## 9. CLI Options

### 9.1 Supported Flags

現行仕様で設定に影響する主な CLI フラグは次のとおり。

- `--config <path>`
- `--agent <name>`
- `--api-key <key>`
- `--model <name>`
- `--lang <lang>`

### 9.2 Meaning

- `--config` は設定ファイルの探索先を変更する
- その他の設定フラグは、解決済み設定を**最後に上書き**する

## 10. `.env` and Environment Variables

### 10.1 Purpose

`.env` および環境変数は、主に `api_key` 解決のための補助ソースである。

### 10.2 Loading

CLI 起動時、`main` の先頭で `load_dotenv()` を呼んで `.env` 読み込みを試みる。

### 10.3 Search Rule

`.env` は `dotenvy` の規則に従い、次を探索する。

1. カレントディレクトリ
2. 親ディレクトリへ遡って最初に見つかったもの

### 10.4 Overwrite Behavior

既に環境変数が設定されている場合、`.env` はそれを上書きしない。

### 10.5 Relevant Variable Names

`agent` に応じて参照する主な環境変数は次のとおり。

- `OPENAI_API_KEY`
- `ANTHROPIC_API_KEY`

## 11. Command-Specific Resolution

設定解決はサブコマンドごとに少し異なる。

### 11.1 `hb gen`

`hb gen` は次の順で設定を構築する。

1. `config.hb` または `--config` の内容を読む
2. `.hb` のファイルレベル設定を読む
3. `.hb` の設定を `config.hb` にマージする
4. CLI 引数を最後に適用する

結果として、優先順位は次のとおり。

```text
CLI > .hb > config.hb > .env > environment variables
```

ここで `.env` / 環境変数は、主として `api_key` 解決時に効く。

### 11.2 `hb ai`

`hb ai` は `.hb` ファイルを入力に取らないため、設定解決は次の順になる。

1. `config.hb` または `--config`
2. CLI
3. `.env` / 環境変数による API キー解決

結果として、優先順位は次のとおり。

```text
CLI > config.hb > .env > environment variables
```

### 11.3 `hb check`

`hb check` は主に構文・検証バックエンド選択に関わる。現時点では `agent` / `model` / `lang` は中心ではない。

ただし将来、`.hb` メタ設定を診断表示する拡張はありうる。

## 12. Merge Semantics by Field

### 12.1 `agent`

`hb gen` では、`.hb` に `agent` が明示されていれば `config.hb.agent` を上書きする。  
その後、CLI に `--agent` があればさらに上書きする。

### 12.2 `api_key`

`hb gen` では、`.hb` に `api_key` が明示されていれば `config.hb.api_key` を上書きする。  
その後、CLI に `--api-key` があればさらに上書きする。

API キーが最終設定に存在しない場合、`resolve_api_key()` で `.env` / 環境変数を参照する。

### 12.3 `model`

`hb gen` では、`.hb` に `model` が明示されていれば `config.hb.model` を上書きする。  
その後、CLI に `--model` があればさらに上書きする。

モデルが最終設定に存在しない場合、`resolve_model()` は `agent` に対応する既定モデルを返す。

### 12.4 `lang`

`hb gen` では、`.hb` に `lang` が**明示された場合のみ** `config.hb.lang` を上書きする。  
その後、CLI に `--lang` があればさらに上書きする。

これは、`.hb` パーサ内部の既定値 `rust` が、**未指定なのに `config.hb.lang` を潰すことを防ぐため**である。

## 13. `lang` Presence Rule

`lang` は他の設定項目と異なり、「見えている値」と「明示的に書かれた値」を区別する必要がある。

### 13.1 Problem

`.hb` パーサは、内部的に `lang` の既定値を `rust` として保持しうる。

しかし、次の 2 ケースは意味が異なる。

1. `.hb` に `lang` が書かれていない
2. `.hb` に `lang: rust` が明示されている

### 13.2 Required Behavior

処理系は少なくとも、`.hb` 内で `lang` が明示されたかどうかを保持しなければならない。

`hb gen` では、

- `lang` 明示あり -> `.hb` が `config.hb.lang` を上書きしてよい
- `lang` 明示なし -> `.hb` の既定値で `config.hb.lang` を上書きしてはならない

## 14. API Key Resolution

### 14.1 When Required

`api_key` は、`agent` が API キーを要求する場合のみ必要である。

- `openai` -> 必要
- `anthropic` -> 必要
- `mock` -> 不要

### 14.2 Resolution Order

`resolve_api_key()` の論理順は次のとおり。

1. 最終設定に入っている `api_key`
2. `agent` に対応する環境変数

ここで「最終設定に入っている `api_key`」は、CLI / `.hb` / `config.hb` のマージ結果である。

### 14.3 Missing Key

`openai` / `anthropic` で API キーが得られなければエラーとする。

`mock` では `Ok(None)` を返してよい。

## 15. Model Resolution

`model` は最終設定に存在すればそれを使い、存在しなければ `agent.default_model()` を使う。

例:

- `agent = openai`, `model = None` -> `gpt-4o`
- `agent = anthropic`, `model = None` -> `claude-3-5-sonnet-20241022`
- `agent = mock`, `model = None` -> `mock`

## 16. Error Conditions

少なくとも次は診断対象とする。

- `config.hb` の未知キー
- `config.hb` の不正値
- API キーが必要なのに解決できない
- `--config` で指定したファイルが読めない
- `.env` 読み込み失敗

## 17. Recommended Usage

推奨配置は次のとおり。

- プロジェクト共通既定値 -> `config.hb`
- 個別ファイルの一時上書き -> `.hb`
- 実行時だけの一時変更 -> CLI
- 秘密情報 -> `.env` または環境変数

例:

```text
config.hb        = lang: rust, agent: mock
target.hb        = agent: openai
CLI              = --lang python
```

このとき `hb gen target.hb` の最終設定は:

- `agent = openai` (`.hb`)
- `lang = python` (CLI)

となる。

## 18. Examples

### 18.1 `hb gen` with all layers

前提:

```text
config.hb:   agent=mock, lang=rust
.hb:         agent=openai, model=gpt-4o
CLI:         --lang python
ENV/.env:    OPENAI_API_KEY=sk-...
```

最終結果:

- `agent = openai`
- `model = gpt-4o`
- `lang = python`
- `api_key = OPENAI_API_KEY`

### 18.2 `lang` not specified in `.hb`

前提:

```text
config.hb: lang=typescript
.hb:       lang 未指定
CLI:       なし
```

最終結果:

- `lang = typescript`

`.hb` パーサ内部の既定値 `rust` によって上書きしてはならない。

### 18.3 `hb ai`

前提:

```text
config.hb: agent=anthropic
CLI:       --model claude-3-5-sonnet-20241022
```

最終結果:

- `agent = anthropic`
- `model = claude-3-5-sonnet-20241022`

`.hb` 層は存在しない。

## 19. Relationship to `.hb` Specification

`.hb` における設定の書き方そのものは `docs/hb-spec.md` が規定する。  
本仕様は、それらの設定値が**実行時にどうマージされるか**を規定する。

## 20. Versioning and Future Work

今後の改訂候補:

- コマンド別設定表の追加
- `hb check` の設定解決の厳密化
- goal 単位 `model` PIN と全体設定の関係の明文化
- 将来の profile / workspace 構成対応
