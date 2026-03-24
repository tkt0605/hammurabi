//! # run_hb — .hb ファイルを読み込んでコードを生成する CLI
//!
//! 使い方:
//!   cargo run --bin run_hb -- <path/to/file.hb> [--lang <language>]
//!
//! オプション:
//!   --lang rust|python|go|java|javascript|typescript
//!       出力言語を指定（省略時は .hb ファイルの `lang` 指定、
//!       それもなければ rust が使われる）
//!
//! 例:
//!   cargo run --bin run_hb -- test.hb
//!   cargo run --bin run_hb -- test.hb --lang python
//!   cargo run --bin run_hb -- test.hb --lang typescript

use std::{env, fs, process};

use hammurabi::{
    codegen::{CodeGenerator, TargetLang},
    lsp::parse_hb,
};

fn main() {
    // ─── 引数パース ────────────────────────────────────────────
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        eprintln!("使い方: run_hb <path/to/file.hb> [--lang rust|python|go|java|javascript|typescript]");
        process::exit(if args.is_empty() { 1 } else { 0 });
    }

    let path = &args[0];

    // --lang フラグを探す（優先度: CLI > .hb ファイルの lang 指定）
    let cli_lang: Option<TargetLang> = {
        let mut found = None;
        let mut i = 1;
        while i < args.len() {
            if args[i] == "--lang" {
                if let Some(val) = args.get(i + 1) {
                    match val.parse::<TargetLang>() {
                        Ok(l)  => { found = Some(l); i += 2; }
                        Err(e) => {
                            eprintln!("--lang エラー: {e}");
                            process::exit(1);
                        }
                    }
                } else {
                    eprintln!("--lang の後に言語名が必要です");
                    process::exit(1);
                }
            } else {
                i += 1;
            }
        }
        found
    };

    // ─── ファイル読み込み ───────────────────────────────────────
    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("ファイルの読み込みに失敗: {path}: {e}");
        process::exit(1);
    });

    println!("═══════════════════════════════════════════════════");
    println!("  Hammurabi .hb パーサ & コード生成");
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

    // 最終的な言語を決定: CLI 引数 > .hb の lang 指定 > Rust（デフォルト）
    let lang = cli_lang.unwrap_or(result.lang);
    println!("✅  {} 個の goal をパースしました", result.goals.len());
    println!("🌐  出力言語: {}\n", lang.display_name());

    // ─── goal ごとにコード生成 ─────────────────────────────────
    let codegen = CodeGenerator::for_lang(lang);

    for (i, parsed_goal) in result.goals.iter().enumerate() {
        let goal = &parsed_goal.goal;

        println!("─────────────────────────────────────────────────");
        println!("  Goal #{}: `{}`", i + 1, goal.name);
        println!("─────────────────────────────────────────────────");
        println!("  事前条件   (require)  : {} 個", goal.preconditions.len());
        println!("  事後条件   (ensure)   : {} 個", goal.postconditions.len());
        println!("  不変条件   (invariant): {} 個", goal.invariants.len());
        println!("  禁止パターン(forbid)  : {} 個\n", goal.forbidden.len());

        let output = codegen.generate(goal);
        println!("【生成コード ({})】\n", codegen.lang.display_name());
        println!("{}", output.source());
        println!();
    }

    println!("═══════════════════════════════════════════════════");
    println!("  完了！生成コードを実装して Hammurabi の証明を始めましょう。");
    println!("═══════════════════════════════════════════════════");
}
