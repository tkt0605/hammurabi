//! # client — 実 AI API クライアント（`ai` feature が必要）
//!
//! ## 使い方
//! ```rust,ignore
//! use hammurabi::ai_gen::{AiGoalGenerator, OpenAiGenerator, AnthropicGenerator};
//!
//! // OpenAI (GPT-4o)
//! let gen = OpenAiGenerator::from_env()?;
//! let out = gen.generate("Safely divide two integers.")?;
//!
//! // Anthropic (Claude)
//! let gen = AnthropicGenerator::from_env()?;
//! let out = gen.generate("Validate an email address.")?;
//! ```

use serde::{Deserialize, Serialize};
use super::{AiGenError, AiGenOutput, AiGoalGenerator, PromptBuilder, hb_to_output};

// ---------------------------------------------------------------------------
// OpenAiGenerator
// ---------------------------------------------------------------------------

/// OpenAI Chat Completions API（GPT-4o など）を使って ContractualGoal を生成する。
///
/// 環境変数 `OPENAI_API_KEY` が必要。
pub struct OpenAiGenerator {
    api_key: String,
    model:   String,
}

impl OpenAiGenerator {
    /// 環境変数 `OPENAI_API_KEY` から初期化する。
    pub fn from_env() -> Result<Self, AiGenError> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| AiGenError::Auth)?;
        Ok(Self::new(api_key, "gpt-4o"))
    }

    /// API キーとモデル名を直接指定して初期化する。
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), model: model.into() }
    }
}

impl AiGoalGenerator for OpenAiGenerator {
    fn generate(&self, description: &str) -> Result<AiGenOutput, AiGenError> {
        let raw_hb = call_openai(
            &self.api_key,
            &self.model,
            PromptBuilder::system_prompt(),
            &PromptBuilder::user_prompt(description),
        )?;
        hb_to_output(raw_hb, description)
    }
}

// ---------------------------------------------------------------------------
// AnthropicGenerator
// ---------------------------------------------------------------------------

/// Anthropic Messages API（Claude 3.5 Sonnet など）を使って ContractualGoal を生成する。
///
/// 環境変数 `ANTHROPIC_API_KEY` が必要。
pub struct AnthropicGenerator {
    api_key: String,
    model:   String,
}

impl AnthropicGenerator {
    /// 環境変数 `ANTHROPIC_API_KEY` から初期化する。
    pub fn from_env() -> Result<Self, AiGenError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| AiGenError::Auth)?;
        Ok(Self::new(api_key, "claude-3-5-sonnet-20241022"))
    }

    /// API キーとモデル名を直接指定して初期化する。
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), model: model.into() }
    }
}

impl AiGoalGenerator for AnthropicGenerator {
    fn generate(&self, description: &str) -> Result<AiGenOutput, AiGenError> {
        let raw_hb = call_anthropic(
            &self.api_key,
            &self.model,
            PromptBuilder::system_prompt(),
            &PromptBuilder::user_prompt(description),
        )?;
        hb_to_output(raw_hb, description)
    }
}

// ---------------------------------------------------------------------------
// OpenAI API 呼び出し
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model:       &'a str,
    messages:    Vec<OpenAiMessage<'a>>,
    temperature: f32,
    max_tokens:  u32,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role:    &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessageContent,
}

#[derive(Deserialize)]
struct OpenAiMessageContent {
    content: String,
}

fn call_openai(
    api_key:     &str,
    model:       &str,
    system:      &str,
    user:        &str,
) -> Result<String, AiGenError> {
    let body = OpenAiRequest {
        model,
        messages: vec![
            OpenAiMessage { role: "system", content: system },
            OpenAiMessage { role: "user",   content: user   },
        ],
        temperature: 0.2,
        max_tokens:  1024,
    };

    let resp = ureq::post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send_json(serde_json::to_value(&body).unwrap())
        .map_err(|e| AiGenError::Http {
            status: 0,
            body:   e.to_string(),
        })?;

    if resp.status() == 401 {
        return Err(AiGenError::Auth);
    }
    if !resp.status().is_success() {
        let status = resp.status().into();
        let body   = resp.body_mut().read_to_string().unwrap_or_default();
        return Err(AiGenError::Http { status, body });
    }

    let parsed: OpenAiResponse = resp
        .into_body()
        .read_json()
        .map_err(|e| AiGenError::Api { message: e.to_string() })?;

    parsed.choices.into_iter()
        .next()
        .map(|c| c.message.content.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or(AiGenError::Empty)
}

// ---------------------------------------------------------------------------
// Anthropic API 呼び出し
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model:       &'a str,
    system:      &'a str,
    messages:    Vec<AnthropicMessage<'a>>,
    max_tokens:  u32,
}

#[derive(Serialize)]
struct AnthropicMessage<'a> {
    role:    &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

fn call_anthropic(
    api_key: &str,
    model:   &str,
    system:  &str,
    user:    &str,
) -> Result<String, AiGenError> {
    let body = AnthropicRequest {
        model,
        system,
        messages: vec![AnthropicMessage { role: "user", content: user }],
        max_tokens: 1024,
    };

    let resp = ureq::post("https://api.anthropic.com/v1/messages")
        .header("x-api-key",         api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type",      "application/json")
        .send_json(serde_json::to_value(&body).unwrap())
        .map_err(|e| AiGenError::Http {
            status: 0,
            body:   e.to_string(),
        })?;

    if resp.status() == 401 {
        return Err(AiGenError::Auth);
    }
    if !resp.status().is_success() {
        let status = resp.status().into();
        let body   = resp.body_mut().read_to_string().unwrap_or_default();
        return Err(AiGenError::Http { status, body });
    }

    let parsed: AnthropicResponse = resp
        .into_body()
        .read_json()
        .map_err(|e| AiGenError::Api { message: e.to_string() })?;

    parsed.content.into_iter()
        .find(|c| c.kind == "text")
        .and_then(|c| c.text)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or(AiGenError::Empty)
}
