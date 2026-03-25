# CLAUDE.md — Hammurabi プロジェクト ガイド

このファイルは AI (Claude) がこのリポジトリで作業する際の指針をまとめたものです。

---

## プロジェクト概要

**Hammurabi** は、AI を活用したアプリケーション開発を支援する「ロードマップ（LoadMaps）」システムです。
開発者が AI との対話を通じてアプリケーションの設計・実装・管理を効率的に進められることを目指しています。

---

## 現在の状態

このプロジェクトはまだ初期段階にあります。アーキテクチャ・技術スタック・実装方針は今後決定していきます。

---

## ディレクトリ構造（予定）

```
hammurabi/
├── CLAUDE.md          # AI 向けプロジェクトガイド（このファイル）
├── README.md          # プロジェクト説明
├── index.txt          # プロジェクトコンセプトメモ
└── src/               # ソースコード（今後作成）
```

---

## 開発方針

### コーディングスタイル
- 技術スタックが決まり次第、このセクションを更新してください。

### ブランチ戦略
- `main`: 安定版
- `feature/*`: 新機能開発
- `fix/*`: バグ修正

### コミットメッセージ
```
<type>: <概要>

例:
feat: ロードマップ生成機能を追加
fix: パース処理のバグを修正
docs: README を更新
```

---

## AI への作業指示

- コードを変更する前に必ず既存ファイルの内容を確認すること
- 不明点がある場合は実装を進める前に質問すること
- ファイルを新規作成する場合は、必要性を確認してから作成すること
- TODO コメントを残す場合は `// TODO(hammurabi): <内容>` の形式を使用すること

---

## ビルド・テスト

```bash
# 最小（Mock のみ・依存なし）
cargo build
cargo test

# AI 生成（OpenAI/Anthropic）と Z3 検証を両方使う
cargo build --features full
cargo test --features full

# 個別に有効化
cargo build --features ai           # AI API クライアント
cargo build --features z3-backend   # Z3 SMT（--verifier z3）
```

外部ノードへ配布する場合は `cargo install --path . --features full`、または GitHub Releases のプリビルドバイナリ（`dist` / `cargo-dist`）を推奨。

---

## 参考・メモ

- プロジェクト名「Hammurabi」は古代バビロニアの法典に由来し、「明確なルールに基づいた開発」を象徴しています。
- `index.txt` にプロジェクトの初期コンセプトが記載されています。
