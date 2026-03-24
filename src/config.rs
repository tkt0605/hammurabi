//! # config — config.hb パーサーと HammurabiConfig
//!
//! `config.hb` ファイルから Hammurabi の実行設定（AI エージェント・言語・API キー等）を読み込む。
//!
//! ## API キーの解決順位
//!
//! ```text
//! 1. CLI 引数           --api-key sk-...
//! 2. .hb ファイル内     api_key sk-...
//! 3. config.hb 内       api_key: sk-...
//! 4. .env ファイル       OPENAI_API_KEY=sk-...   ← load_dotenv() で読み込み
//! 5. 環境変数            export OPENAI_API_KEY=sk-...
//! ```
//!
//! `.env` は `load_dotenv()` を呼ぶことで自動的に環境変数へ展開され、
//! その後 `resolve_api_key()` が通常の `std::env::var()` で取得する。
//!
//! ## config.hb フォーマット
//! ```text
//! # コメント行（# で始まる行は無視）
//! agent:   openai          # openai | anthropic | mock
//! api_key: sk-proj-...     # API キー（省略時は .env / 環境変数を参照）
//! model:   gpt-4o          # モデル名
//! lang:    rust            # 出力言語 (rust / python / go / java / javascript / typescript)
//! ```

use std::fmt;
use std::str::FromStr;
use std::path::Path;
use crate::codegen::TargetLang;

// ---------------------------------------------------------------------------
// AgentKind — 使用する AI バックエンドの種類
// ---------------------------------------------------------------------------

/// Hammurabi が使用する AI バックエンド。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum AgentKind {
    /// OpenAI Chat Completions API（GPT-4o など）
    OpenAi,
    /// Anthropic Messages API（Claude 3.5 Sonnet など）
    Anthropic,
    /// ローカルキーワード解析（API キー不要・開発/テスト用）
    #[default]
    Mock,
}

impl AgentKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::OpenAi    => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Mock      => "Mock (offline)",
        }
    }

    /// このエージェントが API キーを必要とするかどうか。
    pub fn requires_api_key(&self) -> bool {
        matches!(self, Self::OpenAi | Self::Anthropic)
    }

    /// デフォルトのモデル名を返す。
    pub fn default_model(&self) -> &'static str {
        match self {
            Self::OpenAi    => "gpt-4o",
            Self::Anthropic => "claude-3-5-sonnet-20241022",
            Self::Mock      => "mock",
        }
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl FromStr for AgentKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" | "gpt"          => Ok(Self::OpenAi),
            "anthropic" | "claude"    => Ok(Self::Anthropic),
            "mock" | "local" | "none" => Ok(Self::Mock),
            other => Err(format!(
                "未知のエージェント: `{other}` — openai / anthropic / mock のいずれかを指定"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// ConfigError
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ConfigError {
    Io(String),
    UnknownKey { line: usize, key: String },
    InvalidValue { line: usize, key: String, value: String, reason: String },
    MissingApiKey { agent: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e)                       => write!(f, "ファイル読み込みエラー: {e}"),
            Self::UnknownKey { line, key }    => write!(f, "行 {line}: 未知のキー `{key}`"),
            Self::InvalidValue { line, key, value, reason } =>
                write!(f, "行 {line}: `{key}: {value}` — {reason}"),
            Self::MissingApiKey { agent }     => write!(
                f,
                "api_key が未設定です（{agent} を使うには config.hb の api_key または \
                 環境変数 OPENAI_API_KEY / ANTHROPIC_API_KEY が必要です）"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// HammurabiConfig
// ---------------------------------------------------------------------------

/// Hammurabi の実行設定。`config.hb` から読み込む、または CLI 引数でオーバーライドできる。
#[derive(Debug, Clone)]
pub struct HammurabiConfig {
    /// 使用する AI バックエンド
    pub agent:   AgentKind,
    /// AI API キー（None の場合は環境変数から取得を試みる）
    pub api_key: Option<String>,
    /// AI モデル名（None の場合はエージェントのデフォルトを使用）
    pub model:   Option<String>,
    /// デフォルト出力言語
    pub lang:    TargetLang,
}

impl Default for HammurabiConfig {
    fn default() -> Self {
        Self {
            agent:   AgentKind::Mock,
            api_key: None,
            model:   None,
            lang:    TargetLang::Rust,
        }
    }
}

impl HammurabiConfig {
    /// ファイルパスから設定を読み込む。
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(e.to_string()))?;
        Self::from_text(&text)
    }

    /// テキストから設定をパースする（テスト・単体利用向け）。
    pub fn from_text(text: &str) -> Result<Self, ConfigError> {
        let mut cfg = Self::default();

        for (idx, raw_line) in text.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = raw_line.trim();

            // 空行・コメント行はスキップ
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // `key: value` に分割（コメント部分を除去）
            let (kv, _comment) = trimmed.split_once('#').unwrap_or((trimmed, ""));
            let kv = kv.trim();

            let (key, val) = match kv.split_once(':') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => {
                    return Err(ConfigError::UnknownKey {
                        line: line_no,
                        key:  trimmed.to_owned(),
                    });
                }
            };

            match key.to_lowercase().as_str() {
                "agent" => {
                    cfg.agent = val.parse::<AgentKind>().map_err(|reason| {
                        ConfigError::InvalidValue {
                            line: line_no,
                            key: "agent".into(),
                            value: val.to_owned(),
                            reason,
                        }
                    })?;
                }
                "api_key" | "apikey" | "api-key" => {
                    if !val.is_empty() {
                        cfg.api_key = Some(val.to_owned());
                    }
                }
                "model" => {
                    if !val.is_empty() {
                        cfg.model = Some(val.to_owned());
                    }
                }
                "lang" | "language" => {
                    cfg.lang = val.parse::<TargetLang>().map_err(|reason| {
                        ConfigError::InvalidValue {
                            line: line_no,
                            key: "lang".into(),
                            value: val.to_owned(),
                            reason,
                        }
                    })?;
                }
                other => {
                    return Err(ConfigError::UnknownKey {
                        line: line_no,
                        key: other.to_owned(),
                    });
                }
            }
        }

        Ok(cfg)
    }

    /// API キーを解決する。
    ///
    /// 解決順位:
    /// 1. `self.api_key`（CLI 引数 / `.hb` / `config.hb` から設定済み）
    /// 2. 環境変数（`OPENAI_API_KEY` / `ANTHROPIC_API_KEY`）
    ///    → `load_dotenv()` を事前に呼んでおくと `.env` の値も含まれる
    ///
    /// `Mock` の場合は不要なので `Ok(None)` を返す。
    pub fn resolve_api_key(&self) -> Result<Option<String>, ConfigError> {
        if !self.agent.requires_api_key() {
            return Ok(None);
        }
        // CLI 引数 / .hb / config.hb に直接書かれている場合
        if let Some(ref k) = self.api_key {
            return Ok(Some(k.clone()));
        }
        // 環境変数にフォールバック（.env 読み込み済みなら .env の値も参照される）
        let env_key = match self.agent {
            AgentKind::OpenAi    => "OPENAI_API_KEY",
            AgentKind::Anthropic => "ANTHROPIC_API_KEY",
            AgentKind::Mock      => unreachable!(),
        };
        match std::env::var(env_key) {
            Ok(v) if !v.is_empty() => Ok(Some(v)),
            _ => Err(ConfigError::MissingApiKey {
                agent: self.agent.display_name().to_owned(),
            }),
        }
    }

    /// モデル名を解決する（config → エージェントのデフォルトの順）。
    pub fn resolve_model(&self) -> &str {
        self.model.as_deref().unwrap_or_else(|| self.agent.default_model())
    }

    /// CLI 引数で上書き適用する（`None` のフィールドは上書きしない）。
    pub fn apply_overrides(
        &mut self,
        agent:   Option<AgentKind>,
        api_key: Option<String>,
        model:   Option<String>,
        lang:    Option<TargetLang>,
    ) {
        if let Some(a) = agent   { self.agent   = a; }
        if let Some(k) = api_key { self.api_key = Some(k); }
        if let Some(m) = model   { self.model   = Some(m); }
        if let Some(l) = lang    { self.lang     = l; }
    }
}

// ---------------------------------------------------------------------------
// .env ローダー
// ---------------------------------------------------------------------------

/// `.env` ファイルを読み込んで環境変数へ展開する。
///
/// ## 探索順序
/// 1. カレントディレクトリの `.env`
/// 2. 親ディレクトリへ順に遡って最初に見つかった `.env`（dotenvy の標準動作）
///
/// ## ファイルが存在しない場合
/// エラーにはならず静かに無視する（`ok()` で握り潰す）。
/// 既に環境変数が設定されている場合は上書きしない（`.env` の値が優先されない）。
///
/// ## 使い方
/// ```rust,ignore
/// use hammurabi::config::load_dotenv;
///
/// fn main() {
///     load_dotenv(); // ← main 先頭で必ず呼ぶ
///     // 以降 std::env::var("OPENAI_API_KEY") で .env の値が取得できる
/// }
/// ```
///
/// ## .env の書き方
/// ```text
/// # .env
/// OPENAI_API_KEY=sk-proj-xxxxxxxxxxxxxxxxxxxx
/// ANTHROPIC_API_KEY=sk-ant-xxxxxxxxxxxxxxxxxxxx
/// ```
#[cfg(not(target_arch = "wasm32"))]
pub fn load_dotenv() -> DotenvResult {
    match dotenvy::dotenv() {
        Ok(path) => DotenvResult::Loaded(path),
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            DotenvResult::NotFound
        }
        Err(e) => DotenvResult::Error(e.to_string()),
    }
}

/// WASM 環境では `.env` ロードは何もしない。
#[cfg(target_arch = "wasm32")]
pub fn load_dotenv() -> DotenvResult {
    DotenvResult::NotFound
}

/// `load_dotenv()` の結果。
#[derive(Debug)]
pub enum DotenvResult {
    /// `.env` を正常に読み込んだ。パスを含む。
    Loaded(std::path::PathBuf),
    /// `.env` ファイルが見つからなかった（正常）。
    NotFound,
    /// `.env` の読み込みに失敗した（パースエラー等）。
    Error(String),
}

impl DotenvResult {
    /// ロードに成功したかどうかを返す（`NotFound` は成功扱い）。
    pub fn is_ok(&self) -> bool {
        !matches!(self, Self::Error(_))
    }

    /// ロードしたパスを返す（`Loaded` の場合のみ）。
    pub fn path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Loaded(p) => Some(p),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let text = r#"
# Hammurabi config
agent:   openai
api_key: sk-proj-test1234
model:   gpt-4o
lang:    python
"#;
        let cfg = HammurabiConfig::from_text(text).unwrap();
        assert_eq!(cfg.agent,              AgentKind::OpenAi);
        assert_eq!(cfg.api_key.as_deref(), Some("sk-proj-test1234"));
        assert_eq!(cfg.model.as_deref(),   Some("gpt-4o"));
        assert_eq!(cfg.lang,               TargetLang::Python);
    }

    #[test]
    fn parse_anthropic_config() {
        let text = "agent: anthropic\napi_key: sk-ant-test\nmodel: claude-3-5-sonnet-20241022";
        let cfg = HammurabiConfig::from_text(text).unwrap();
        assert_eq!(cfg.agent, AgentKind::Anthropic);
    }

    #[test]
    fn parse_mock_config() {
        let text = "agent: mock";
        let cfg = HammurabiConfig::from_text(text).unwrap();
        assert_eq!(cfg.agent, AgentKind::Mock);
    }

    #[test]
    fn default_config_is_mock_rust() {
        let cfg = HammurabiConfig::default();
        assert_eq!(cfg.agent, AgentKind::Mock);
        assert_eq!(cfg.lang,  TargetLang::Rust);
        assert!(cfg.api_key.is_none());
    }

    #[test]
    fn resolve_model_uses_default_when_none() {
        let cfg = HammurabiConfig { agent: AgentKind::OpenAi, model: None, ..Default::default() };
        assert_eq!(cfg.resolve_model(), "gpt-4o");
    }

    #[test]
    fn resolve_model_uses_config_value() {
        let cfg = HammurabiConfig {
            agent: AgentKind::OpenAi,
            model: Some("gpt-4-turbo".into()),
            ..Default::default()
        };
        assert_eq!(cfg.resolve_model(), "gpt-4-turbo");
    }

    #[test]
    fn apply_overrides_overwrites_fields() {
        let mut cfg = HammurabiConfig::default();
        cfg.apply_overrides(
            Some(AgentKind::Anthropic),
            Some("sk-override".into()),
            Some("claude-opus".into()),
            Some(TargetLang::TypeScript),
        );
        assert_eq!(cfg.agent,              AgentKind::Anthropic);
        assert_eq!(cfg.api_key.as_deref(), Some("sk-override"));
        assert_eq!(cfg.model.as_deref(),   Some("claude-opus"));
        assert_eq!(cfg.lang,               TargetLang::TypeScript);
    }

    #[test]
    fn invalid_agent_returns_error() {
        let text = "agent: cobol";
        let err = HammurabiConfig::from_text(text).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidValue { .. }));
    }

    #[test]
    fn unknown_key_returns_error() {
        let text = "temperature: 0.9";
        let err = HammurabiConfig::from_text(text).unwrap_err();
        assert!(matches!(err, ConfigError::UnknownKey { .. }));
    }

    #[test]
    fn inline_comment_is_stripped() {
        let text = "agent: openai   # GPT-4 を使用";
        let cfg = HammurabiConfig::from_text(text).unwrap();
        assert_eq!(cfg.agent, AgentKind::OpenAi);
    }

    #[test]
    fn agent_kind_from_str_aliases() {
        assert_eq!("gpt".parse::<AgentKind>().unwrap(),    AgentKind::OpenAi);
        assert_eq!("claude".parse::<AgentKind>().unwrap(), AgentKind::Anthropic);
        assert_eq!("local".parse::<AgentKind>().unwrap(),  AgentKind::Mock);
    }

    #[test]
    fn resolve_api_key_uses_env_var_when_not_set_in_config() {
        // 環境変数をテスト用に一時設定
        std::env::set_var("OPENAI_API_KEY", "sk-test-from-env");
        let cfg = HammurabiConfig {
            agent:   AgentKind::OpenAi,
            api_key: None,  // config には設定なし
            ..Default::default()
        };
        let key = cfg.resolve_api_key().unwrap();
        assert_eq!(key, Some("sk-test-from-env".to_string()));
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn resolve_api_key_prefers_config_over_env_var() {
        // 環境変数も設定されているが config の値が優先される
        std::env::set_var("OPENAI_API_KEY", "sk-from-env");
        let cfg = HammurabiConfig {
            agent:   AgentKind::OpenAi,
            api_key: Some("sk-from-config".into()),
            ..Default::default()
        };
        let key = cfg.resolve_api_key().unwrap();
        assert_eq!(key, Some("sk-from-config".to_string()));
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn resolve_api_key_returns_error_when_missing() {
        std::env::remove_var("OPENAI_API_KEY");
        let cfg = HammurabiConfig {
            agent:   AgentKind::OpenAi,
            api_key: None,
            ..Default::default()
        };
        let err = cfg.resolve_api_key().unwrap_err();
        assert!(matches!(err, ConfigError::MissingApiKey { .. }));
    }

    #[test]
    fn resolve_api_key_returns_none_for_mock() {
        let cfg = HammurabiConfig::default(); // Mock
        let key = cfg.resolve_api_key().unwrap();
        assert!(key.is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn load_dotenv_not_found_is_ok() {
        // テスト実行時に .env がなくてもエラーにならないことを確認
        // （既に .env がある環境でも NotFound 以外の Ok になる）
        let result = load_dotenv();
        assert!(result.is_ok(), "load_dotenv が Error を返した: {:?}", result);
    }
}
