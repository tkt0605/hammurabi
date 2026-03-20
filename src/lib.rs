//! # hammurabi
//!
//! AIネイティブ言語「hammurabi」のコアライブラリ。
//! コード生成ではなく「論理検証」にリソースを全振りする設計思想を実装する。
//!
//! ## モジュール構成
//! - `lang::goal`     — [`ContractualGoal`]: AI への発注書（述語論理による仕様記述）
//! - `lang::rail`     — [`LogicRail<T>`]: 証明を封印した意味論的コンテナ
//! - `compiler::verifier` — [`Verifier`]: 憲法適合性チェックエンジン

pub mod ai_gen;
pub mod codegen;
pub mod compiler;
pub mod lang;
pub mod lsp;
pub mod proof_store;

#[cfg(feature = "wasm")]
pub mod wasm;
