//! # hb — Hammurabi CLI
//!
//! ```
//! cargo install hammurabi        # crates.io から
//! cargo install --path .         # ローカル開発
//! ```
//!
//! ## サブコマンド
//!
//! ```
//! hb gen  <file.hb>   [OPTIONS]   .hb の契約から実装コードを生成
//! hb ai   "<prompt>"  [OPTIONS]   AI でゴールを生成してコードを生成
//! hb init [--force]               config.hb / .env.example を作成
//! hb check <file.hb>              .hb ファイルの構文チェック（コード生成なし）
//! ```
//!
//! ## OPTIONS（gen / ai 共通）
//! ```
//! --config  <path>    config.hb のパス（省略時はカレントディレクトリを自動検索）
//! --agent   <name>    openai | anthropic | mock
//! --api-key <key>     API キー（省略時は .env / 環境変数から自動解決）
//! --model   <name>    gpt-4o / claude-3-5-sonnet-20241022 など
//! --lang    <lang>    rust | python | go | java | javascript | typescript
//! ```

use std::{env, fs, process};

use hammurabi::{
    ai_gen::{build_code_writer, build_generator},
    codegen::TargetLang,
    config::{AgentKind, DotenvResult, HammurabiConfig, load_dotenv},
    lsp::{parse_hb, ErrorSeverity},
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BIN_NAME: &str = "hb";

// ---------------------------------------------------------------------------
// サブコマンド
// ---------------------------------------------------------------------------

enum Subcommand {
    Gen   { file: String,   opts: CommonOpts },
    Ai    { prompt: String, opts: CommonOpts },
    Init  { force: bool },
    Check { file: String },
}

struct CommonOpts {
    config:  Option<String>,
    agent:   Option<AgentKind>,
    api_key: Option<String>,
    model:   Option<String>,
    lang:    Option<TargetLang>,
}

// ---------------------------------------------------------------------------
// 引数パーサ
// ---------------------------------------------------------------------------

fn parse_args() -> Subcommand {
    let raw: Vec<String> = env::args().skip(1).collect();

    if raw.is_empty() {
        print_help();
        process::exit(0);
    }

    match raw[0].as_str() {
        "-V" | "--version" | "version" => {
            println!("{BIN_NAME} {VERSION}");
            process::exit(0);
        }
        "-h" | "--help" | "help" => {
            print_help();
            process::exit(0);
        }
        "gen" => {
            let (file, opts) = parse_file_and_opts(&raw[1..], "gen");
            Subcommand::Gen { file, opts }
        }
        "ai" => {
            let (prompt, opts) = parse_prompt_and_opts(&raw[1..]);
            Subcommand::Ai { prompt, opts }
        }
        "init" => {
            let force = raw.get(1).map(|s| s == "--force").unwrap_or(false);
            Subcommand::Init { force }
        }
        "check" => {
            let (file, _) = parse_file_and_opts(&raw[1..], "check");
            Subcommand::Check { file }
        }
        unknown => {
            // 後方互換: `hb <file.hb>` は `hb gen <file.hb>` と同じ扱い
            if unknown.ends_with(".hb") || std::path::Path::new(unknown).exists() {
                let (file, opts) = parse_file_and_opts(&raw, "gen");
                Subcommand::Gen { file, opts }
            } else {
                eprintln!("エラー: 不明なサブコマンド `{unknown}`\n");
                print_help();
                process::exit(1);
            }
        }
    }
}

fn parse_file_and_opts(args: &[String], subcmd: &str) -> (String, CommonOpts) {
    let mut file:    Option<String>    = None;
    let mut config:  Option<String>    = None;
    let mut agent:   Option<AgentKind> = None;
    let mut api_key: Option<String>    = None;
    let mut model:   Option<String>    = None;
    let mut lang:    Option<TargetLang>= None;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => { config  = Some(next_val(args, &mut i, "--config")); }
            "--agent"  => {
                let v = next_val(args, &mut i, "--agent");
                agent = Some(v.parse::<AgentKind>().unwrap_or_else(|e| {
                    eprintln!("--agent エラー: {e}"); process::exit(1);
                }));
            }
            "--api-key" | "--api_key" | "--apikey" => {
                api_key = Some(next_val(args, &mut i, "--api-key"));
            }
            "--model" => { model = Some(next_val(args, &mut i, "--model")); }
            "--lang"  => {
                let v = next_val(args, &mut i, "--lang");
                lang = Some(v.parse::<TargetLang>().unwrap_or_else(|e| {
                    eprintln!("--lang エラー: {e}"); process::exit(1);
                }));
            }
            flag if flag.starts_with('-') => {
                eprintln!("エラー: 不明なフラグ `{flag}` (subcommand: {subcmd})\n");
                print_help();
                process::exit(1);
            }
            positional => {
                if file.is_none() { file = Some(positional.to_owned()); }
            }
        }
        i += 1;
    }

    let file = file.unwrap_or_else(|| {
        eprintln!("エラー: `hb {subcmd}` には <file.hb> が必要です\n");
        print_help();
        process::exit(1);
    });

    (file, CommonOpts { config, agent, api_key, model, lang })
}

fn parse_prompt_and_opts(args: &[String]) -> (String, CommonOpts) {
    let mut prompt:  Option<String>    = None;
    let mut config:  Option<String>    = None;
    let mut agent:   Option<AgentKind> = None;
    let mut api_key: Option<String>    = None;
    let mut model:   Option<String>    = None;
    let mut lang:    Option<TargetLang>= None;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => { config  = Some(next_val(args, &mut i, "--config")); }
            "--agent"  => {
                let v = next_val(args, &mut i, "--agent");
                agent = Some(v.parse::<AgentKind>().unwrap_or_else(|e| {
                    eprintln!("--agent エラー: {e}"); process::exit(1);
                }));
            }
            "--api-key" | "--api_key" | "--apikey" => {
                api_key = Some(next_val(args, &mut i, "--api-key"));
            }
            "--model" => { model = Some(next_val(args, &mut i, "--model")); }
            "--lang"  => {
                let v = next_val(args, &mut i, "--lang");
                lang = Some(v.parse::<TargetLang>().unwrap_or_else(|e| {
                    eprintln!("--lang エラー: {e}"); process::exit(1);
                }));
            }
            flag if flag.starts_with('-') => {
                eprintln!("エラー: 不明なフラグ `{flag}` (subcommand: ai)\n");
                print_help();
                process::exit(1);
            }
            positional => {
                if prompt.is_none() { prompt = Some(positional.to_owned()); }
            }
        }
        i += 1;
    }

    let prompt = prompt.unwrap_or_else(|| {
        eprintln!("エラー: `hb ai` にはプロンプト文字列が必要です\n");
        eprintln!("  例: hb ai \"Safely divide two integers. Divisor must not be zero.\"\n");
        process::exit(1);
    });

    (prompt, CommonOpts { config, agent, api_key, model, lang })
}

fn next_val(args: &[String], i: &mut usize, flag: &str) -> String {
    *i += 1;
    if *i >= args.len() {
        eprintln!("エラー: `{flag}` の後に値が必要です");
        process::exit(1);
    }
    args[*i].clone()
}

// ---------------------------------------------------------------------------
// 設定ロード
// ---------------------------------------------------------------------------

fn load_config(opts: &CommonOpts) -> HammurabiConfig {
    let mut cfg = if let Some(ref path) = opts.config {
        HammurabiConfig::from_file(path).unwrap_or_else(|e| {
            eprintln!("config ファイル読み込みエラー: {e}");
            process::exit(1);
        })
    } else if std::path::Path::new("config.hb").exists() {
        HammurabiConfig::from_file("config.hb").unwrap_or_else(|e| {
            eprintln!("config.hb 読み込みエラー（自動検出）: {e}");
            process::exit(1);
        })
    } else {
        HammurabiConfig::default()
    };

    cfg.apply_overrides(
        opts.agent.clone(),
        opts.api_key.clone(),
        opts.model.clone(),
        opts.lang.clone(),
    );
    cfg
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    // .env を最優先でロード
    match load_dotenv() {
        DotenvResult::Loaded(path) => {
            eprintln!("🔑  .env を読み込みました: {}", path.display());
        }
        DotenvResult::NotFound => {}
        DotenvResult::Error(e) => {
            eprintln!("⚠️  .env の読み込みに失敗しました: {e}");
        }
    }

    match parse_args() {
        Subcommand::Gen   { file, opts }   => cmd_gen(&file, &opts),
        Subcommand::Ai    { prompt, opts } => cmd_ai(&prompt, &opts),
        Subcommand::Init  { force }        => cmd_init(force),
        Subcommand::Check { file }         => cmd_check(&file),
    }
}

// ---------------------------------------------------------------------------
// hb gen <file.hb>
// ---------------------------------------------------------------------------

fn cmd_gen(path: &str, opts: &CommonOpts) {
    let cfg = load_config(opts);

    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("ファイル読み込みエラー: {path}: {e}");
        process::exit(1);
    });

    print_banner("gen", &cfg);
    println!("  ファイル: {path}\n");

    let result = parse_hb(&text);

    // 警告・エラーを表示
    let warnings: Vec<_> = result.errors.iter().filter(|e| e.severity == ErrorSeverity::Warning).collect();
    let errors:   Vec<_> = result.errors.iter().filter(|e| e.severity == ErrorSeverity::Error).collect();

    for w in &warnings {
        println!("⚠️  行 {}: {}", w.span.line + 1, w.message);
    }
    if !warnings.is_empty() { println!(); }
    for e in &errors {
        println!("❌  行 {}: {}", e.span.line + 1, e.message);
    }
    if !errors.is_empty() { println!(); }

    if result.goals.is_empty() {
        eprintln!("goal が 1 つも見つかりませんでした。");
        process::exit(1);
    }

    // .hb の設定をマージ（CLI > .hb > config.hb の優先順位）
    let mut effective_cfg = cfg.clone();
    if effective_cfg.agent == AgentKind::Mock {
        if let Some(a) = result.agent { effective_cfg.agent = a; }
    }
    if effective_cfg.api_key.is_none() {
        if let Some(k) = result.api_key { effective_cfg.api_key = Some(k); }
    }
    if effective_cfg.model.is_none() {
        if let Some(m) = result.model { effective_cfg.model = Some(m); }
    }

    let lang = opts.lang.clone()
        .or_else(|| if cfg.lang != TargetLang::Rust { Some(cfg.lang.clone()) } else { None })
        .unwrap_or(result.lang);

    let use_ai = effective_cfg.agent != AgentKind::Mock;

    println!("✅  {} 個の goal をパースしました", result.goals.len());
    println!("🌐  出力言語: {}", lang.display_name());
    println!("🤖  エージェント: {}", effective_cfg.agent.display_name());
    if effective_cfg.agent.requires_api_key() {
        println!("📐  モデル: {}", effective_cfg.resolve_model());
    }
    if use_ai { println!("✨  AI が契約を満たす実装コードを生成します…"); }
    println!();

    let writer = build_code_writer(&effective_cfg).unwrap_or_else(|e| {
        eprintln!("コードライター初期化エラー: {e}"); process::exit(1);
    });

    for (i, pg) in result.goals.iter().enumerate() {
        let goal = &pg.goal;
        println!("─────────────────────────────────────────────────");
        println!("  Goal #{}: `{}`", i + 1, goal.name);
        println!("─────────────────────────────────────────────────");
        println!("  require  : {} 個 / ensure: {} 個 / forbid: {} 個\n",
            goal.preconditions.len(), goal.postconditions.len(), goal.forbidden.len());

        if use_ai { println!("⏳  `{}` を生成中…", goal.name); }

        match writer.write_code(goal, &lang) {
            Ok(out) => {
                println!("【生成コード ({})】\n", out.lang.display_name());
                println!("{}", out.source);
                for w in &out.warnings { println!("  💡 {w}"); }
            }
            Err(e) => {
                eprintln!("❌  生成エラー (`{}`): {e}", goal.name);
                if !use_ai { process::exit(1); }
                println!("  ⚠️  スキップしました。");
            }
        }
        println!();
    }

    println!("═══════════════════════════════════════════════════");
    if use_ai { println!("  完了！AI が生成したコードを確認してください。"); }
    else       { println!("  完了！TODO を実装に置き換えてください。"); }
    println!("═══════════════════════════════════════════════════");
}

// ---------------------------------------------------------------------------
// hb ai "<prompt>"
// ---------------------------------------------------------------------------

fn cmd_ai(prompt: &str, opts: &CommonOpts) {
    let cfg = load_config(opts);

    // .hb ファイルパスが誤って渡された場合に案内する
    if prompt.ends_with(".hb") && std::path::Path::new(prompt).exists() {
        eprintln!("⚠️  `{prompt}` は .hb ファイルのように見えます。");
        eprintln!("    既存の .hb からコードを生成するには `hb gen` を使ってください:");
        eprintln!("      hb gen {prompt}");
        eprintln!("    AI でゴールを生成する場合は自然言語のプロンプトを渡してください:");
        eprintln!("      hb ai \"関数の説明を日本語か英語で記述\"");
        process::exit(1);
    }

    print_banner("ai", &cfg);

    println!("🤖  エージェント: {}", cfg.agent.display_name());
    println!("📐  モデル: {}", cfg.resolve_model());
    println!("🌐  出力言語: {}", cfg.lang.display_name());
    println!("💬  プロンプト: {prompt}\n");

    let generator = build_generator(&cfg).unwrap_or_else(|e| {
        eprintln!("エージェント初期化エラー: {e}"); process::exit(1);
    });

    println!("⏳  ContractualGoal を生成中…\n");

    let output = generator.generate(prompt).unwrap_or_else(|e| {
        eprintln!("AI 生成エラー: {e}"); process::exit(1);
    });

    println!("【生成された .hb テキスト】\n");
    println!("{}\n", output.raw_hb);

    if !output.warnings.is_empty() {
        for w in &output.warnings { println!("⚠️  {w}"); }
        println!();
    }

    println!("═══════════════════════════════════════════════════");
    println!("  生成コード（{}）", cfg.lang.display_name());
    println!("═══════════════════════════════════════════════════\n");

    let writer = build_code_writer(&cfg).unwrap_or_else(|e| {
        eprintln!("コードライター初期化エラー: {e}"); process::exit(1);
    });

    for (i, goal) in output.goals.iter().enumerate() {
        println!("─── Goal #{}: `{}` ───", i + 1, goal.name);
        match writer.write_code(goal, &cfg.lang) {
            Ok(out) => println!("{}", out.source),
            Err(e)  => eprintln!("❌  コード生成エラー: {e}"),
        }
        println!();
    }

    println!("✅  完了");
}

// ---------------------------------------------------------------------------
// hb init
// ---------------------------------------------------------------------------

fn cmd_init(force: bool) {
    println!("🔧  Hammurabi プロジェクトを初期化します…\n");

    create_file(
        "config.hb",
        force,
        r#"# config.hb — Hammurabi デフォルト設定
# CLI 引数 > .hb ファイル > ここの設定 の優先順位

agent: mock          # openai | anthropic | mock
# api_key: $OPENAI_API_KEY
# model: gpt-4o
lang: rust           # rust | python | go | java | javascript | typescript
"#,
    );

    create_file(
        ".env.example",
        force,
        r#"# .env.example — API キー設定テンプレート
# cp .env.example .env して実際のキーを記入してください
# .env は .gitignore に追加済みです

OPENAI_API_KEY=sk-proj-...
# ANTHROPIC_API_KEY=sk-ant-...
"#,
    );

    // .gitignore に .env を追加（なければ作成）
    let gitignore = std::fs::read_to_string(".gitignore").unwrap_or_default();
    if !gitignore.contains(".env") {
        let new_content = if gitignore.is_empty() {
            ".env\n".to_owned()
        } else {
            format!("{gitignore}\n.env\n")
        };
        fs::write(".gitignore", new_content).ok();
        println!("✅  .gitignore に .env を追加しました");
    }

    println!("\n📋  次のステップ:");
    println!("  1. cp .env.example .env");
    println!("  2. .env に API キーを記入");
    println!("  3. hb gen <file.hb>   でコード生成");
    println!("  4. hb ai \"<説明>\"    で AI ゴール生成\n");
}

fn create_file(path: &str, force: bool, content: &str) {
    if std::path::Path::new(path).exists() && !force {
        println!("⏭️  スキップ: `{path}` は既に存在します（--force で上書き）");
        return;
    }
    fs::write(path, content).unwrap_or_else(|e| {
        eprintln!("❌  `{path}` の作成に失敗しました: {e}");
        process::exit(1);
    });
    println!("✅  作成: `{path}`");
}

// ---------------------------------------------------------------------------
// hb check <file.hb>
// ---------------------------------------------------------------------------

fn cmd_check(path: &str) {
    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("ファイル読み込みエラー: {path}: {e}");
        process::exit(1);
    });

    println!("🔍  {BIN_NAME} check: {path}\n");

    let result = parse_hb(&text);

    let warnings: Vec<_> = result.errors.iter().filter(|e| e.severity == ErrorSeverity::Warning).collect();
    let errors:   Vec<_> = result.errors.iter().filter(|e| e.severity == ErrorSeverity::Error).collect();

    for w in &warnings {
        println!("⚠️  行 {}: {}", w.span.line + 1, w.message);
    }
    for e in &errors {
        println!("❌  行 {}: {}", e.span.line + 1, e.message);
    }

    if errors.is_empty() && warnings.is_empty() {
        println!("✅  エラーなし");
    }

    println!();
    println!("  goal    : {} 個", result.goals.len());
    println!("  言語    : {}", result.lang.display_name());
    if let Some(ref a) = result.agent  { println!("  agent   : {}", a.display_name()); }
    if let Some(ref m) = result.model  { println!("  model   : {m}"); }

    if !errors.is_empty() {
        process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// ヘルプ / バナー
// ---------------------------------------------------------------------------

fn print_help() {
    println!(
r#"hb {VERSION} — Hammurabi Logic-First Code Generator

使い方:
  hb gen  <file.hb>  [OPTIONS]   .hb の契約から実装コードを生成
  hb ai   "<prompt>" [OPTIONS]   AI でゴールを生成 → コード生成
  hb init [--force]              config.hb / .env.example を作成
  hb check <file.hb>             .hb ファイルの構文チェック

OPTIONS（gen / ai 共通）:
  --config  <path>   config.hb のパス
  --agent   <name>   openai | anthropic | mock
  --api-key <key>    API キー（省略時は .env を参照）
  --model   <name>   gpt-4o / claude-3-5-sonnet-20241022 など
  --lang    <lang>   rust | python | go | java | javascript | typescript

設定優先順位:
  CLI 引数 > .hb ファイル内の指定 > config.hb > .env > 環境変数

例:
  hb gen  test.hb
  hb gen  test.hb --lang python
  hb gen  test3.hb              # agent/lang を .hb から自動読み込み
  hb ai   "Safely divide two integers." --agent mock
  hb ai   "Validate an email." --agent openai --lang typescript
  hb init
  hb check test.hb

インストール:
  cargo install hammurabi        # crates.io
  cargo install --path .         # ローカル開発
"#
    );
}

fn print_banner(subcmd: &str, cfg: &HammurabiConfig) {
    println!("═══════════════════════════════════════════════════");
    println!("  hb {VERSION} — Logic-First Code Generator");
    match subcmd {
        "gen" => println!("  hb gen  (エージェント: {})", cfg.agent.display_name()),
        "ai"  => println!("  hb ai   (エージェント: {})", cfg.agent.display_name()),
        _     => {}
    }
    println!("═══════════════════════════════════════════════════\n");
}
