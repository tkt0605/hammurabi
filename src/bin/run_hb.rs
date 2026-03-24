//! # run_hb — Hammurabi コード生成 / AI ゴール生成 CLI
//!
//! ## 使い方
//!
//! ### 1. .hb ファイルからコード生成（既存機能）
//! ```sh
//! cargo run --bin run_hb -- test.hb
//! cargo run --bin run_hb -- test.hb --lang python
//! ```
//!
//! ### 2. config.hb から AI エージェント設定を読み込んでゴール生成
//! ```sh
//! cargo run --features ai --bin run_hb -- \
//!     --config config.hb \
//!     --ai "Safely divide two integers. Divisor must not be zero."
//! ```
//!
//! ### 3. CLI 引数で直接 AI エージェントを指定
//! ```sh
//! cargo run --features ai --bin run_hb -- \
//!     --agent openai \
//!     --api-key sk-proj-... \
//!     --model gpt-4o \
//!     --lang typescript \
//!     --ai "Validate an email address."
//! ```
//!
//! ### 4. Mock エージェント（API キー不要・開発用）
//! ```sh
//! cargo run --bin run_hb -- \
//!     --agent mock \
//!     --ai "Compute the GCD of two non-negative integers."
//! ```
//!
//! ## 優先順位
//! CLI 引数  >  config.hb の設定  >  デフォルト値

use std::{env, fs, process};

use hammurabi::{
    ai_gen::{build_code_writer, build_generator},
    codegen::TargetLang,
    config::{AgentKind, DotenvResult, HammurabiConfig, load_dotenv},
    lsp::parse_hb,
};

// ---------------------------------------------------------------------------
// 引数構造体
// ---------------------------------------------------------------------------

struct Args {
    /// .hb ファイルパス（--ai が無い場合は必須）
    hb_file:    Option<String>,
    /// config.hb のパス
    config:     Option<String>,
    /// AI エージェント（CLI オーバーライド）
    agent:      Option<AgentKind>,
    /// API キー（CLI オーバーライド）
    api_key:    Option<String>,
    /// モデル名（CLI オーバーライド）
    model:      Option<String>,
    /// 出力言語（CLI オーバーライド）
    lang:       Option<TargetLang>,
    /// AI への自然言語プロンプト（指定時は AI モードで動作）
    ai_prompt:  Option<String>,
}

fn parse_args() -> Args {
    let raw: Vec<String> = env::args().skip(1).collect();

    if raw.is_empty() || raw[0] == "--help" || raw[0] == "-h" {
        print_help();
        process::exit(if raw.is_empty() { 1 } else { 0 });
    }

    let mut hb_file:   Option<String>    = None;
    let mut config:    Option<String>    = None;
    let mut agent:     Option<AgentKind> = None;
    let mut api_key:   Option<String>    = None;
    let mut model:     Option<String>    = None;
    let mut lang:      Option<TargetLang>= None;
    let mut ai_prompt: Option<String>    = None;

    let mut i = 0usize;
    while i < raw.len() {
        match raw[i].as_str() {
            "--config" => {
                config = Some(take_next(&raw, &mut i, "--config"));
            }
            "--agent" => {
                let v = take_next(&raw, &mut i, "--agent");
                agent = Some(v.parse::<AgentKind>().unwrap_or_else(|e| {
                    eprintln!("--agent エラー: {e}"); process::exit(1)
                }));
            }
            "--api-key" | "--api_key" | "--apikey" => {
                api_key = Some(take_next(&raw, &mut i, "--api-key"));
            }
            "--model" => {
                model = Some(take_next(&raw, &mut i, "--model"));
            }
            "--lang" => {
                let v = take_next(&raw, &mut i, "--lang");
                lang = Some(v.parse::<TargetLang>().unwrap_or_else(|e| {
                    eprintln!("--lang エラー: {e}"); process::exit(1)
                }));
            }
            "--ai" => {
                ai_prompt = Some(take_next(&raw, &mut i, "--ai"));
            }
            flag if flag.starts_with('-') => {
                eprintln!("未知のフラグ: {flag}\n");
                print_help();
                process::exit(1);
            }
            // 最初の位置引数は .hb ファイルパス
            positional => {
                if hb_file.is_none() {
                    hb_file = Some(positional.to_owned());
                }
            }
        }
        i += 1;
    }

    Args { hb_file, config, agent, api_key, model, lang, ai_prompt }
}

fn take_next(args: &[String], i: &mut usize, flag: &str) -> String {
    *i += 1;
    if *i >= args.len() {
        eprintln!("{flag} の後に値が必要です"); process::exit(1);
    }
    args[*i].clone()
}

fn print_help() {
    eprintln!(
r#"使い方: run_hb [オプション] [<file.hb>]

モード:
  <file.hb>                  .hb ファイルをパースしてコード生成（既存）
  --ai "<description>"       AI でゴールを生成してコード生成

AI エージェント設定（優先順位: CLI > .hb内 > config.hb > .env > 環境変数）:
  --config  <path>           config.hb のパス（例: --config config.hb）
  --agent   <name>           openai | anthropic | mock
  --api-key <key>            AI API キー
                             省略時は .env の OPENAI_API_KEY / ANTHROPIC_API_KEY を参照
  --model   <name>           モデル名（例: gpt-4o, claude-3-5-sonnet-20241022）

出力オプション:
  --lang    <lang>           rust | python | go | java | javascript | typescript

例:
  # .hb ファイルから Rust コード生成
  run_hb test.hb

  # .hb ファイルから Python コード生成
  run_hb test.hb --lang python

  # config.hb で設定した OpenAI エージェントで AI ゴール生成
  run_hb --config config.hb --ai "Safely divide two integers."

  # CLI 直接指定（ai feature 必要）
  run_hb --agent openai --api-key sk-... --model gpt-4o --ai "Validate email."

  # Mock エージェント（API 不要）
  run_hb --agent mock --ai "Compute factorial of n."
"#
    );
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    // ─── .env ファイルを環境変数に展開（最優先で実行）────────────────────
    // config.hb や .hb ファイルの api_key 省略時に環境変数を参照するため、
    // 引数パースよりも前にロードする必要がある。
    match load_dotenv() {
        DotenvResult::Loaded(path) => {
            eprintln!("🔑  .env を読み込みました: {}", path.display());
        }
        DotenvResult::NotFound => {
            // .env が無いのは正常（環境変数で直接設定している場合など）
        }
        DotenvResult::Error(e) => {
            eprintln!("⚠️  .env の読み込みに失敗しました: {e}");
        }
    }

    let args = parse_args();

    // ─── 設定の構築（config.hb → CLI オーバーライド）─────────────────
    let mut cfg = if let Some(ref config_path) = args.config {
        HammurabiConfig::from_file(config_path).unwrap_or_else(|e| {
            eprintln!("config.hb 読み込みエラー: {e}");
            process::exit(1);
        })
    } else if std::path::Path::new("config.hb").exists() {
        // カレントディレクトリに config.hb があれば自動読み込み
        HammurabiConfig::from_file("config.hb").unwrap_or_else(|e| {
            eprintln!("config.hb 読み込みエラー（自動検出）: {e}");
            process::exit(1);
        })
    } else {
        HammurabiConfig::default()
    };

    // CLI 引数でオーバーライド
    let cli_lang = args.lang.clone();
    let ai_mode  = args.ai_prompt.is_some();
    cfg.apply_overrides(args.agent.clone(), args.api_key.clone(), args.model.clone(), args.lang.clone());

    print_banner(&cfg, ai_mode);

    // ─── AI モード ────────────────────────────────────────────────────
    if let Some(ref prompt) = args.ai_prompt {
        run_ai_mode(&cfg, prompt);
        return;
    }

    // ─── .hb ファイルモード ───────────────────────────────────────────
    let hb_path = args.hb_file.unwrap_or_else(|| {
        eprintln!("エラー: .hb ファイルパスか --ai オプションが必要です\n");
        print_help();
        process::exit(1);
    });

    run_file_mode(&cfg, &hb_path, cli_lang);
}

// ---------------------------------------------------------------------------
// AI モード
// ---------------------------------------------------------------------------

fn run_ai_mode(cfg: &HammurabiConfig, prompt: &str) {
    println!("🤖  AI エージェント: {}", cfg.agent.display_name());
    println!("📐  モデル: {}", cfg.resolve_model());
    println!("💬  プロンプト: {prompt}\n");

    let generator = build_generator(cfg).unwrap_or_else(|e| {
        eprintln!("エージェント初期化エラー: {e}");
        process::exit(1);
    });

    println!("⏳  AI に ContractualGoal を生成中…\n");

    let output = generator.generate(prompt).unwrap_or_else(|e| {
        eprintln!("AI 生成エラー: {e}");
        process::exit(1);
    });

    println!("【生成された .hb テキスト】\n");
    println!("{}\n", output.raw_hb);

    if !output.warnings.is_empty() {
        println!("⚠️  警告:");
        for w in &output.warnings { println!("  - {w}"); }
        println!();
    }

    // 生成されたゴールからコードを出力
    println!("═══════════════════════════════════════════════════");
    println!("  生成コード（{}）", cfg.lang.display_name());
    println!("═══════════════════════════════════════════════════\n");

    let writer = build_code_writer(cfg).unwrap_or_else(|e| {
        eprintln!("コードライター初期化エラー: {e}");
        process::exit(1);
    });

    for (i, goal) in output.goals.iter().enumerate() {
        println!("─── Goal #{}: `{}` ───", i + 1, goal.name);
        match writer.write_code(goal, &cfg.lang) {
            Ok(code_output) => println!("{}", code_output.source),
            Err(e)          => eprintln!("❌  コード生成エラー: {e}"),
        }
    }

    println!("✅  完了");
}

// ---------------------------------------------------------------------------
// ファイルモード
// ---------------------------------------------------------------------------

fn run_file_mode(cfg: &HammurabiConfig, path: &str, cli_lang: Option<TargetLang>) {
    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("ファイルの読み込みに失敗: {path}: {e}");
        process::exit(1);
    });

    println!("  ファイル: {path}\n");

    let result = parse_hb(&text);

    // エラー / 警告を表示（api_key の security warning も含む）
    let errors: Vec<_> = result.errors.iter()
        .filter(|e| e.severity == hammurabi::lsp::ErrorSeverity::Error)
        .collect();
    let warnings: Vec<_> = result.errors.iter()
        .filter(|e| e.severity == hammurabi::lsp::ErrorSeverity::Warning)
        .collect();

    if !warnings.is_empty() {
        println!("⚠️  警告 ({} 件):", warnings.len());
        for w in &warnings {
            println!("  行 {}: {}", w.span.line + 1, w.message);
        }
        println!();
    }
    if !errors.is_empty() {
        println!("❌  パースエラー ({} 件):", errors.len());
        for e in &errors {
            println!("  行 {}: {}", e.span.line + 1, e.message);
        }
        println!();
    }

    if result.goals.is_empty() {
        eprintln!("goal が 1 つも見つかりませんでした。");
        process::exit(1);
    }

    // ── 設定の優先順位 ──────────────────────────────────────────────────
    // CLI 引数  >  .hb ファイルの agent/api_key/model/lang  >  config.hb  >  デフォルト
    let mut effective_cfg = cfg.clone();

    if effective_cfg.agent == AgentKind::Mock {
        if let Some(a) = result.agent {
            effective_cfg.agent = a;
        }
    }
    if effective_cfg.api_key.is_none() {
        if let Some(k) = result.api_key {
            effective_cfg.api_key = Some(k);
        }
    }
    if effective_cfg.model.is_none() {
        if let Some(m) = result.model {
            effective_cfg.model = Some(m);
        }
    }

    // 言語の優先順位: CLI > config.hb（非デフォルト） > .hb lang 行
    let lang = cli_lang
        .or_else(|| if cfg.lang != TargetLang::Rust { Some(cfg.lang.clone()) } else { None })
        .unwrap_or(result.lang);

    let use_ai = effective_cfg.agent != AgentKind::Mock;

    println!("✅  {} 個の goal をパースしました", result.goals.len());
    println!("🌐  出力言語: {}", lang.display_name());
    println!("🤖  AI エージェント: {}", effective_cfg.agent.display_name());
    if effective_cfg.agent.requires_api_key() {
        println!("📐  モデル: {}", effective_cfg.resolve_model());
    }
    if use_ai {
        println!("✨  AI が契約を満たす実装コードを生成します…");
    }
    println!();

    // ── コードライターを構築 ─────────────────────────────────────────────
    // Mock → スケルトン、OpenAI/Anthropic → AI が実装コードを生成
    let writer = build_code_writer(&effective_cfg).unwrap_or_else(|e| {
        eprintln!("コードライター初期化エラー: {e}");
        process::exit(1);
    });

    for (i, parsed_goal) in result.goals.iter().enumerate() {
        let goal = &parsed_goal.goal;
        println!("─────────────────────────────────────────────────");
        println!("  Goal #{}: `{}`", i + 1, goal.name);
        println!("─────────────────────────────────────────────────");
        println!("  事前条件   (require)  : {} 個", goal.preconditions.len());
        println!("  事後条件   (ensure)   : {} 個", goal.postconditions.len());
        println!("  不変条件   (invariant): {} 個", goal.invariants.len());
        println!("  禁止パターン(forbid)  : {} 個\n", goal.forbidden.len());

        if use_ai {
            println!("⏳  AI に `{}` の実装を生成中…", goal.name);
        }

        match writer.write_code(goal, &lang) {
            Ok(output) => {
                println!("【生成コード ({})】\n", output.lang.display_name());
                println!("{}", output.source);
                if !output.warnings.is_empty() {
                    println!();
                    for w in &output.warnings {
                        println!("  💡 {w}");
                    }
                }
            }
            Err(e) => {
                eprintln!("❌  コード生成エラー (goal `{}`): {e}", goal.name);
                if !use_ai {
                    process::exit(1);
                }
                // AI エラーの場合はスキップして次のゴールへ
                println!("  ⚠️  このゴールはスキップされました。");
            }
        }
        println!();
    }

    println!("═══════════════════════════════════════════════════");
    if use_ai {
        println!("  完了！AI が生成したコードを確認してください。");
    } else {
        println!("  完了！TODO コメントを実装に置き換えてください。");
    }
    println!("═══════════════════════════════════════════════════");
}

// ---------------------------------------------------------------------------
// バナー表示
// ---------------------------------------------------------------------------

fn print_banner(cfg: &HammurabiConfig, ai_mode: bool) {
    println!("═══════════════════════════════════════════════════");
    println!("  Hammurabi — Logic-First Code Generator");
    if ai_mode {
        println!("  モード: AI ゴール生成  ({})", cfg.agent.display_name());
    } else {
        println!("  モード: .hb ファイルパース");
    }
    println!("═══════════════════════════════════════════════════\n");
}
