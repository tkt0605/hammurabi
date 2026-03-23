//! # run_hb — .hb ファイルを読み込んで Rust コードを生成するデモ CLI
//!
//! 使い方:
//!   cargo run --bin run_hb -- <path/to/file.hb>
//!   例: cargo run --bin run_hb -- test.hb

use std::{env, fs, process};

use hammurabi::{
    codegen::CodeGenerator,
    lsp::parse_hb,
};

fn main() {
    // ─── 引数チェック ──────────────────────────────────────────
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("使い方: run_hb <path/to/file.hb>");
        process::exit(1);
    }
    let path = &args[1];

    // ─── ファイル読み込み ───────────────────────────────────────
    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("ファイルの読み込みに失敗: {path}: {e}");
        process::exit(1);
    });

    println!("═══════════════════════════════════════════════════");
    println!("  Hammurabi .hb パーサ & コード生成デモ");
    println!("  ファイル: {path}");
    println!("═══════════════════════════════════════════════════\n");

    // ─── パース ────────────────────────────────────────────────
    let result = parse_hb(&text);

    // パースエラーを報告
    if !result.errors.is_empty() {
        println!("⚠️  パースエラー ({} 件):", result.errors.len());
        for err in &result.errors {
            println!("  行 {}: {}", err.span.line + 1, err.message);
        }
        println!();
    }

    if result.goals.is_empty() {
        eprintln!("goal が 1 つも見つかりませんでした。");
        process::exit(1);
    }

    println!("✅  {} 個の goal をパースしました\n", result.goals.len());

    // ─── goal ごとにコード生成 ─────────────────────────────────
    let codegen = CodeGenerator;

    for (i, parsed_goal) in result.goals.iter().enumerate() {
        let goal = &parsed_goal.goal;

        // 統計情報を表示
        println!("─────────────────────────────────────────────────");
        println!("  Goal #{}: `{}`", i + 1, goal.name);
        println!("─────────────────────────────────────────────────");
        println!("  事前条件   (require)  : {} 個", goal.preconditions.len());
        println!("  事後条件   (ensure)   : {} 個", goal.postconditions.len());
        println!("  不変条件   (invariant): {} 個", goal.invariants.len());
        println!("  禁止パターン(forbid)  : {} 個\n", goal.forbidden.len());

        // Rust コードを生成
        let output = codegen.generate(goal);
        println!("【生成された Rust コード】\n");
        println!("{}", output.source());
        println!();
    }

    println!("═══════════════════════════════════════════════════");
    println!("  完了！生成コードを src/ に配置して実装を始めましょう。");
    println!("═══════════════════════════════════════════════════");
}
