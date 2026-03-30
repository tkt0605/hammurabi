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
    compiler::verifier::{MockVerifier, Verifier, ConstitutionalReport},
    config::{AgentKind, DotenvResult, HammurabiConfig, load_dotenv},
    lang::goal::ContractualGoal,
    lsp::{parse_hb, ErrorSeverity, ParsedGoal},
};

#[cfg(feature = "z3-backend")]
use hammurabi::compiler::verifier::z3_backend::Z3Verifier;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BIN_NAME: &str = "hb";

// ---------------------------------------------------------------------------
// サブコマンド
// ---------------------------------------------------------------------------

enum Subcommand {
    Gen   { file: String,   opts: CommonOpts },
    Ai    { prompt: String, opts: CommonOpts },
    Init  { force: bool },
    Check { file: String,   verifier: VerifierKind },
}

/// 検証バックエンドの選択
#[derive(Debug, Clone, PartialEq, Default)]
enum VerifierKind {
    #[default]
    Mock,
    Z3,
}

struct CommonOpts {
    config:   Option<String>,
    agent:    Option<AgentKind>,
    api_key:  Option<String>,
    model:    Option<String>,
    lang:     Option<TargetLang>,
    verifier: VerifierKind,
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
            let (file, opts) = parse_file_and_opts(&raw[1..], "check");
            Subcommand::Check { file, verifier: opts.verifier }
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
    let mut file:     Option<String>    = None;
    let mut config:   Option<String>    = None;
    let mut agent:    Option<AgentKind> = None;
    let mut api_key:  Option<String>    = None;
    let mut model:    Option<String>    = None;
    let mut lang:     Option<TargetLang>= None;
    let mut verifier: VerifierKind      = VerifierKind::Mock;

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
            "--verifier" => {
                let v = next_val(args, &mut i, "--verifier");
                verifier = parse_verifier_kind(&v);
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

    (file, CommonOpts { config, agent, api_key, model, lang, verifier })
}

fn parse_prompt_and_opts(args: &[String]) -> (String, CommonOpts) {
    let mut prompt:   Option<String>    = None;
    let mut config:   Option<String>    = None;
    let mut agent:    Option<AgentKind> = None;
    let mut api_key:  Option<String>    = None;
    let mut model:    Option<String>    = None;
    let mut lang:     Option<TargetLang>= None;
    let mut verifier: VerifierKind      = VerifierKind::Mock;

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
            "--verifier" => {
                let v = next_val(args, &mut i, "--verifier");
                verifier = parse_verifier_kind(&v);
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

    (prompt, CommonOpts { config, agent, api_key, model, lang, verifier })
}

fn next_val(args: &[String], i: &mut usize, flag: &str) -> String {
    *i += 1;
    if *i >= args.len() {
        eprintln!("エラー: `{flag}` の後に値が必要です");
        process::exit(1);
    }
    args[*i].clone()
}

fn parse_verifier_kind(s: &str) -> VerifierKind {
    match s.to_lowercase().as_str() {
        "z3" | "z3-smt" | "smt" => {
            #[cfg(not(feature = "z3-backend"))]
            {
                eprintln!("エラー: --verifier z3 を使うには z3-backend feature が必要です。");
                eprintln!("  cargo build --features z3-backend でビルドしてください。");
                process::exit(1);
            }
            #[cfg(feature = "z3-backend")]
            VerifierKind::Z3
        }
        "mock" | "default" => VerifierKind::Mock,
        other => {
            eprintln!("エラー: 不明な verifier `{other}` — z3 / mock のいずれかを指定");
            process::exit(1);
        }
    }
}

/// goal を指定された検証バックエンドで検証し、結果を表示する。
/// 戻り値: (compliant件数, violation件数)
fn verify_goals_with_backend(
    goals: &[hammurabi::lsp::ParsedGoal],
    verifier_kind: &VerifierKind,
) -> (usize, usize) {
    let mut ok_count  = 0usize;
    let mut err_count = 0usize;

    match verifier_kind {
        VerifierKind::Mock => {
            let v = MockVerifier::default();
            for pg in goals {
                run_goal_verification(&pg.goal, &v, &mut ok_count, &mut err_count);
            }
        }
        VerifierKind::Z3 => {
            #[cfg(feature = "z3-backend")]
            {
                let v = Z3Verifier::new();
                for pg in goals {
                    run_goal_verification(&pg.goal, &v, &mut ok_count, &mut err_count);
                }
            }
            #[cfg(not(feature = "z3-backend"))]
            {
                eprintln!("z3-backend feature が無効です。");
                process::exit(1);
            }
        }
    }

    (ok_count, err_count)
}

fn run_goal_verification<V: Verifier>(
    goal:      &ContractualGoal,
    verifier:  &V,
    ok_count:  &mut usize,
    err_count: &mut usize,
) {
    match verifier.verify_goal(goal) {
        Ok(report) => {
            print_verification_report(&report);
            if report.is_compliant() { *ok_count  += 1; }
            else                     { *err_count += 1; }
        }
        Err(e) => {
            println!("  ❌  検証エラー: {e}");
            *err_count += 1;
        }
    }
}

fn print_verification_report(report: &ConstitutionalReport) {
    let icon = if report.is_compliant() { "✅" } else { "❌" };
    println!("  {icon} [{:?}] {}", report.proof_backend, report.goal_name);
    if !report.is_compliant() {
        for v in &report.violations {
            println!("       ✗ {v}");
        }
    }
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
        Subcommand::Gen   { file, opts }            => cmd_gen(&file, &opts),
        Subcommand::Ai    { prompt, opts }           => cmd_ai(&prompt, &opts),
        Subcommand::Init  { force }                  => cmd_init(force),
        Subcommand::Check { file, verifier }         => cmd_check(&file, &verifier),
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
    let has_nl_goals = result.goals.iter().any(|pg| pg.needs_ai);

    println!("✅  {} 個の goal をパースしました", result.goals.len());
    if has_nl_goals {
        let nl_count = result.goals.iter().filter(|pg| pg.needs_ai).count();
        println!("⚡  {} 個の goal は自然言語のみ（AI が制約を自動生成します）", nl_count);
    }
    println!("🌐  出力言語: {}", lang.display_name());
    println!("🤖  エージェント: {}", effective_cfg.agent.display_name());
    if effective_cfg.agent.requires_api_key() {
        println!("📐  モデル: {}", effective_cfg.resolve_model());
    }

    // ── Z3 が有効な場合、コード生成前に仕様を証明 ─────────────────────
    let verifier_kind = &opts.verifier;
    if *verifier_kind == VerifierKind::Z3 {
        println!();
        println!("🔬  Z3 SMT で仕様を証明中…");
        let (ok, err) = verify_goals_with_backend(&result.goals, verifier_kind);
        println!("    合格: {ok} 個 / 違反: {err} 個");
        if err > 0 {
            eprintln!("❌  仕様に違反があるためコード生成を中止しました。");
            process::exit(1);
        }
        println!("    ✅  全ての仕様が証明されました。コード生成を開始します。");
    }

    if use_ai || has_nl_goals {
        println!("✨  AI が契約を満たす実装コードを生成します…");
    }
    println!();

    // needs_ai ゴールがある場合は generator も初期化する
    let generator = if has_nl_goals {
        Some(build_generator(&effective_cfg).unwrap_or_else(|e| {
            eprintln!("AI ジェネレーター初期化エラー: {e}"); process::exit(1);
        }))
    } else {
        None
    };

    let writer = build_code_writer(&effective_cfg).unwrap_or_else(|e| {
        eprintln!("コードライター初期化エラー: {e}"); process::exit(1);
    });

    for (i, pg) in result.goals.iter().enumerate() {
        println!("─────────────────────────────────────────────────");

        // needs_ai な goal は自然言語から制約を AI で生成する
        let expanded: ContractualGoal;
        let goal: &ContractualGoal = if pg.needs_ai {
            // depends_on で参照された goal の契約を収集
            let dep_goals: Vec<&ContractualGoal> = pg.depends_on.iter()
                .filter_map(|dep_id| {
                    result.goals.iter().find(|g| g.id.as_deref() == Some(dep_id.as_str()))
                        .map(|g| &g.goal)
                })
                .collect();
            // プロンプトに依存先契約・型・例を統合
            let desc = hammurabi::ai_gen::build_goal_description(
                &pg.goal.name,
                pg.label.as_deref(),
                pg.intent.as_deref(),
                &pg.goal.inputs,
                pg.goal.output.as_deref(),
                &pg.goal.examples,
                &dep_goals,
            );
            if !dep_goals.is_empty() {
                let dep_ids = pg.depends_on.join(", ");
                println!("  🔗  依存: [{dep_ids}]");
            }
            let display_name = if let Some(lbl) = &pg.label {
                format!("`{}` (\"{}\")", pg.goal.name, lbl)
            } else {
                format!("`{}`", pg.goal.name)
            };
            println!("  Goal #{}: {} ⚡ 自然言語から AI が制約を生成", i + 1, display_name);
            println!("─────────────────────────────────────────────────");
            println!("  説明: {}", pg.label.as_deref().unwrap_or(&pg.goal.name));
            println!("⏳  制約を生成中…\n");
            // goal レベルの model: が指定されていれば専用 generator を生成してモデルを上書き
            let goal_generator;
            let active_generator: &dyn hammurabi::ai_gen::AiGoalGenerator = if let Some(ref mp) = pg.model_pin {
                let mut pinned_cfg = effective_cfg.clone();
                // `name@version` → `name-version`（OpenAI / Anthropic の API 形式に正規化）
                let api_model = mp.replacen('@', "-", 1);
                pinned_cfg.model = Some(api_model.clone());
                println!("  📌  model PIN: {mp}  →  API モデル: {api_model}");
                goal_generator = hammurabi::ai_gen::build_generator(&pinned_cfg).unwrap_or_else(|e| {
                    eprintln!("AI ジェネレーター (PIN) 初期化エラー: {e}"); process::exit(1);
                });
                goal_generator.as_ref()
            } else {
                generator.as_ref().unwrap().as_ref()
            };

            match active_generator.generate(&desc) {
                Ok(output) => {
                    if let Some(mut gen_goal) = output.goals.into_iter().next() {
                        // .hb で指定された inputs/output/examples/id を AI 生成ゴールに引き継ぐ
                        if !pg.goal.inputs.is_empty() {
                            gen_goal.inputs = pg.goal.inputs.clone();
                        }
                        if pg.goal.output.is_some() {
                            gen_goal.output = pg.goal.output.clone();
                        }
                        if !pg.goal.examples.is_empty() {
                            gen_goal.examples = pg.goal.examples.clone();
                        }
                        gen_goal.id        = pg.id.clone();
                        gen_goal.model_pin = pg.model_pin.clone();
                        println!("  ✅  生成された制約: require {} / ensure {} / forbid {}\n",
                            gen_goal.preconditions.len(),
                            gen_goal.postconditions.len(),
                            gen_goal.forbidden.len());
                        expanded = gen_goal;
                        &expanded
                    } else {
                        eprintln!("  ⚠️  AI が制約を生成できませんでした。スキップします。");
                        println!();
                        continue;
                    }
                }
                Err(e) => {
                    eprintln!("  ❌  AI 生成エラー: {e}");
                    println!();
                    continue;
                }
            }
        } else {
            println!("  Goal #{}: `{}`", i + 1, pg.goal.name);
            println!("─────────────────────────────────────────────────");
            println!("  require  : {} 個 / ensure: {} 個 / forbid: {} 個\n",
                pg.goal.preconditions.len(), pg.goal.postconditions.len(), pg.goal.forbidden.len());
            if use_ai { println!("⏳  `{}` を生成中…", pg.goal.name); }
            &pg.goal
        };

        match writer.write_code(goal, &lang) {
            Ok(out) => {
                println!("【生成コード ({})】\n", out.lang.display_name());
                println!("{}", out.source);
                for w in &out.warnings { println!("  💡 {w}"); }
            }
            Err(e) => {
                eprintln!("❌  生成エラー (`{}`): {e}", goal.name);
                if !use_ai && !pg.needs_ai { process::exit(1); }
                println!("  ⚠️  スキップしました。");
            }
        }
        println!();
    }

    println!("═══════════════════════════════════════════════════");
    if use_ai || has_nl_goals {
        println!("  完了！AI が生成したコードを確認してください。");
    } else {
        println!("  完了！TODO を実装に置き換えてください。");
    }
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

fn cmd_check(path: &str, verifier_kind: &VerifierKind) {
    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("ファイル読み込みエラー: {path}: {e}");
        process::exit(1);
    });

    let backend_label = match verifier_kind {
        VerifierKind::Mock => "Mock",
        VerifierKind::Z3   => "Z3 SMT",
    };
    println!("🔍  {BIN_NAME} check [{backend_label}]: {path}\n");

    let result = parse_hb(&text);

    // パース警告・エラーを表示
    let warnings: Vec<_> = result.errors.iter().filter(|e| e.severity == ErrorSeverity::Warning).collect();
    let errors:   Vec<_> = result.errors.iter().filter(|e| e.severity == ErrorSeverity::Error).collect();

    for w in &warnings {
        println!("⚠️  行 {}: {}", w.span.line + 1, w.message);
    }
    for e in &errors {
        println!("❌  行 {}: {}", e.span.line + 1, e.message);
    }
    if errors.is_empty() && warnings.is_empty() {
        println!("✅  構文エラーなし");
    }

    println!();
    println!("  goal    : {} 個", result.goals.len());
    println!("  言語    : {}", result.lang.display_name());
    if let Some(ref a) = result.agent  { println!("  agent   : {}", a.display_name()); }
    if let Some(ref m) = result.model  { println!("  model   : {m}"); }

    // needs_ai ゴールのヒントを表示 + id / model_pin のサマリ
    for ParsedGoal { goal, needs_ai, label, id, model_pin, .. } in &result.goals {
        if *needs_ai {
            let desc = label.as_deref().unwrap_or(&goal.name);
            println!("  ⚡  `{}`: 自然言語のみ定義 — `hb gen {}` で制約を自動生成できます", goal.name, path);
            println!("       説明: {desc}");
        }
        if let Some(ref id_str) = id {
            let model_str = model_pin.as_deref().map(|m| format!("  (model: {m})")).unwrap_or_default();
            println!("  🔖  `{}` → id: {id_str}{model_str}", goal.name);
        }
    }

    // ID 重複検証
    {
        let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        let mut has_dup = false;
        for pg in &result.goals {
            if let Some(ref id_str) = pg.id {
                if let Some(prev_name) = seen.get(id_str.as_str()) {
                    println!("❌  ID 重複: `{id_str}` は `{prev_name}` と `{}` の両方に使われています", pg.goal.name);
                    has_dup = true;
                } else {
                    seen.insert(id_str.as_str(), pg.goal.name.as_str());
                }
            }
        }
        if has_dup {
            process::exit(1);
        }
    }

    // depends_on 検証 + 循環検出 + グラフ表示
    {
        use std::collections::{HashMap, HashSet};

        // id → goal_name の逆引きマップ
        let id_map: HashMap<&str, &str> = result.goals.iter()
            .filter_map(|pg| pg.id.as_deref().map(|id| (id, pg.goal.name.as_str())))
            .collect();

        let has_any_deps = result.goals.iter().any(|pg| !pg.depends_on.is_empty());

        // ── 存在検証 ────────────────────────────────────────────────
        let mut dep_error = false;
        for pg in &result.goals {
            for dep_id in &pg.depends_on {
                if !id_map.contains_key(dep_id.as_str()) {
                    println!("❌  依存解決失敗: `{}` が参照する `{dep_id}` は存在しません（id: が未定義か typo）",
                        pg.id.as_deref().unwrap_or(&pg.goal.name));
                    dep_error = true;
                }
            }
        }
        if dep_error { process::exit(1); }

        // ── 循環検出（DFS）───────────────────────────────────────────
        // id → depends_on ids の隣接リスト
        let adj: HashMap<&str, Vec<&str>> = result.goals.iter()
            .filter_map(|pg| {
                pg.id.as_deref().map(|id| {
                    let deps: Vec<&str> = pg.depends_on.iter()
                        .map(|s| s.as_str())
                        .collect();
                    (id, deps)
                })
            })
            .collect();

        fn dfs_cycle<'a>(
            node:     &'a str,
            adj:      &HashMap<&'a str, Vec<&'a str>>,
            visited:  &mut HashSet<&'a str>,
            in_stack: &mut Vec<&'a str>,
        ) -> Option<Vec<String>> {
            visited.insert(node);
            in_stack.push(node);
            if let Some(deps) = adj.get(node) {
                for &dep in deps {
                    if in_stack.contains(&dep) {
                        let mut cycle = in_stack.iter().map(|s| s.to_string()).collect::<Vec<_>>();
                        cycle.push(dep.to_string());
                        return Some(cycle);
                    }
                    if !visited.contains(dep) {
                        if let Some(cycle) = dfs_cycle(dep, adj, visited, in_stack) {
                            return Some(cycle);
                        }
                    }
                }
            }
            in_stack.pop();
            None
        }

        let mut visited: HashSet<&str> = HashSet::new();
        for id in adj.keys() {
            if !visited.contains(id) {
                let mut stack: Vec<&str> = Vec::new();
                if let Some(cycle) = dfs_cycle(id, &adj, &mut visited, &mut stack) {
                    println!("❌  依存グラフに循環があります: {}", cycle.join(" → "));
                    process::exit(1);
                }
            }
        }

        // ── 依存グラフ表示 ─────────────────────────────────────────
        if has_any_deps {
            println!();
            println!("── 依存グラフ ─────────────────────────────────────────────────");
            for pg in &result.goals {
                if pg.depends_on.is_empty() { continue; }
                let node_label = pg.id.as_deref().unwrap_or(&pg.goal.name);
                println!("  {node_label}");
                for (i, dep_id) in pg.depends_on.iter().enumerate() {
                    let dep_name = id_map.get(dep_id.as_str())
                        .map(|n| format!(" (`{n}`)"))
                        .unwrap_or_default();
                    let branch = if i + 1 == pg.depends_on.len() { "└─" } else { "├─" };
                    println!("  {branch} {dep_id}{dep_name}");
                }
            }
            println!("  ✅  循環なし");
        }
    }

    if result.goals.is_empty() {
        if !errors.is_empty() { process::exit(1); }
        return;
    }

    // needs_ai ゴールのみの場合は論理検証をスキップ
    let verifiable_goals: Vec<_> = result.goals.iter().filter(|pg| !pg.needs_ai).collect();

    // ── 契約の論理検証（Verifier による証明）──────────────────────────
    println!();
    println!("── ContractualGoal 検証 ({backend_label}) ──────────────────────────");

    if verifiable_goals.is_empty() {
        println!("  (全ての goal が自然言語のみです。`hb gen` で制約を自動生成してください)");
        if !errors.is_empty() { process::exit(1); }
        return;
    }

    let (ok, err) = verify_goals_with_backend(
        &verifiable_goals.iter().map(|pg| (*pg).clone()).collect::<Vec<_>>(),
        verifier_kind,
    );

    println!();
    println!("  合格: {ok} 個 / 違反: {err} 個");

    if !errors.is_empty() || err > 0 {
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
  --config    <path>     config.hb のパス
  --agent     <name>     openai | anthropic | mock
  --api-key   <key>      API キー（省略時は .env を参照）
  --model     <name>     gpt-4o / claude-3-5-sonnet-20241022 など
  --lang      <lang>     rust | python | go | java | javascript | typescript
  --verifier  <backend>  mock（デフォルト）| z3（要 z3-backend feature）

OPTIONS（check）:
  --verifier  <backend>  mock（デフォルト）| z3（要 z3-backend feature）

設定優先順位:
  CLI 引数 > .hb ファイル内の指定 > config.hb > .env > 環境変数

例:
  hb gen  test.hb
  hb gen   test.hb --lang python
  hb gen   test.hb --verifier z3 --lang rust   # Z3 で仕様を証明してからコード生成
  hb gen   test3.hb              # agent/lang を .hb から自動読み込み
  hb ai    "Safely divide two integers." --agent mock
  hb ai    "Validate an email." --agent openai --lang typescript
  hb init
  hb check test.hb
  hb check test.hb --verifier z3  # Z3 SMT で契約の整合性を厳密証明

ビルド（Cargo features）:
  cargo build                    # 最小（Mock のみ・Z3/AI なし）
  cargo build --features ai      # OpenAI/Anthropic 連携
  cargo build --features z3-backend   # Z3 SMT（--verifier z3）
  cargo build --features full    # ai + z3-backend まとめて

インストール:
  cargo install --path . --features full
  cargo install hammurabi --features full   # crates.io 公開後
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
