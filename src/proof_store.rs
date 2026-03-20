//! # ProofStore — ProofToken の永続化レイヤー
//!
//! ## 設計方針
//! - `ProofRecord` = `ProofToken` + メタデータ（目標名・制約・タイムスタンプ）
//! - JSON にシリアライズしてファイルへ保存・復元
//! - **二段構えの改竄防止**
//!   1. `constraint_hash` 再計算（内部整合性）
//!   2. HMAC-SHA256 署名（外部からのファイル改竄防止）
//!
//! ## データフロー
//! ```text
//! ProofToken
//!   ↓ ProofRecord::from_token(...)
//! ProofRecord ──sign(key)──→ signature フィールドに HMAC をセット
//!   ↓ save_signed(path, key)
//! *.proof.json（署名付き）
//!   ↓ load_and_verify_signed(path, key)
//! verify_signature(key)  →  HMAC が一致しなければ Err(SignatureInvalid)
//!   ↓
//! verify_integrity()     →  constraint_hash が不一致なら Err(Tampered)
//!   ↓
//! Ok(ProofToken)
//! ```
//!
//! ## HMAC の仕組み
//! ```text
//! 署名対象バイト列 = JSON(SignablePayload)
//!                    ┌─ goal_name, rail_id, issued_at
//!                    ├─ backend, constraint_hash
//!                    └─ constraints（制約本体）
//!
//! signature = hex( HMAC-SHA256(key, 署名対象バイト列) )
//! ```
//! `signature` フィールド自体は署名対象に含めない（鶏卵問題を回避）。

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use crate::lang::rail::{Constraint, ProofToken, VerifierBackend, hash_constraints};

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// StoreError
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("ファイル I/O エラー: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON シリアライズ/デシリアライズ エラー: {0}")]
    Json(#[from] serde_json::Error),

    #[error("制約ハッシュの改竄を検出: 保存時={stored:016x}, 再計算={recomputed:016x}")]
    Tampered { stored: u64, recomputed: u64 },

    #[error("HMAC 署名の検証に失敗: ファイルが改竄されている可能性があります")]
    SignatureInvalid,

    #[error("未署名のレコードに対して verify_signature が呼ばれた（先に sign() を呼ぶこと）")]
    Unsigned,

    #[error("制約が空のため整合性を検証できない")]
    EmptyConstraints,
}

// ---------------------------------------------------------------------------
// SignablePayload — HMAC の署名対象
// ---------------------------------------------------------------------------

/// HMAC 計算時に JSON へシリアライズする「署名ペイロード」。
/// `signature` フィールドを意図的に含めないことで鶏卵問題を回避する。
/// フィールドの順序変更は署名を無効化するため、追加する場合は末尾のみ可。
#[derive(Serialize)]
struct SignablePayload<'a> {
    goal_name:       &'a str,
    rail_id:         &'a str,
    issued_at:       u64,
    backend:         &'a VerifierBackend,
    constraint_hash: u64,
    constraints:     &'a [Constraint],
}

// ---------------------------------------------------------------------------
// ProofRecord — 保存単位
// ---------------------------------------------------------------------------

/// ファイルに書き出す 1 レコード。
///
/// | フィールド | 役割 |
/// |------------|------|
/// | `constraint_hash` | 制約セットの内部整合性チェック（`verify_integrity`） |
/// | `signature` | ファイル全体の HMAC-SHA256（`verify_signature`） |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofRecord {
    /// この証明が属する目標名（ContractualGoal::name に対応）
    pub goal_name: String,

    /// 証明対象のレール識別子
    pub rail_id: String,

    /// 発行時の UNIX タイムスタンプ（秒）
    pub issued_at: u64,

    /// 発行バックエンド（Z3Smt or Mock）
    pub backend: VerifierBackend,

    /// 制約セットのハッシュ（ProofToken が保持するものと同一であるべき）
    pub constraint_hash: u64,

    /// 制約セット本体（整合性再計算に使用）
    pub constraints: Vec<Constraint>,

    /// HMAC-SHA256 署名（hex エンコード）。`sign()` を呼ぶまで `None`。
    pub signature: Option<String>,
}

impl ProofRecord {
    // -----------------------------------------------------------------------
    // 構築
    // -----------------------------------------------------------------------

    /// `ProofToken` と関連メタデータから未署名の `ProofRecord` を生成する。
    ///
    /// # Errors
    /// - `constraints` が空の場合は `StoreError::EmptyConstraints`
    pub fn from_token(
        token: &ProofToken,
        goal_name: impl Into<String>,
        rail_id:   impl Into<String>,
        constraints: Vec<Constraint>,
    ) -> Result<Self, StoreError> {
        if constraints.is_empty() {
            return Err(StoreError::EmptyConstraints);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let issued_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        #[cfg(target_arch = "wasm32")]
        let issued_at: u64 = 0;

        Ok(Self {
            goal_name:       goal_name.into(),
            rail_id:         rail_id.into(),
            issued_at,
            backend:         token.backend.clone(),
            constraint_hash: token.constraint_hash,
            constraints,
            signature:       None,
        })
    }

    // -----------------------------------------------------------------------
    // 内部整合性検証（constraint_hash の再計算）
    // -----------------------------------------------------------------------

    /// 制約を再ハッシュして `constraint_hash` と照合する。
    ///
    /// # Errors
    /// - ハッシュが一致しない場合は `StoreError::Tampered`
    pub fn verify_integrity(&self) -> Result<ProofToken, StoreError> {
        let recomputed = hash_constraints(&self.constraints);
        if recomputed != self.constraint_hash {
            return Err(StoreError::Tampered {
                stored:     self.constraint_hash,
                recomputed,
            });
        }
        Ok(ProofToken::new(self.constraint_hash, self.backend.clone()))
    }

    // -----------------------------------------------------------------------
    // HMAC-SHA256 署名・検証
    // -----------------------------------------------------------------------

    /// 署名対象バイト列を生成する（`SignablePayload` の JSON）。
    fn signable_bytes(&self) -> Result<Vec<u8>, StoreError> {
        let payload = SignablePayload {
            goal_name:       &self.goal_name,
            rail_id:         &self.rail_id,
            issued_at:       self.issued_at,
            backend:         &self.backend,
            constraint_hash: self.constraint_hash,
            constraints:     &self.constraints,
        };
        Ok(serde_json::to_vec(&payload)?)
    }

    /// `key` を使って HMAC-SHA256 署名を計算し、`self.signature` にセットする。
    ///
    /// 同じレコードに対して再度呼ぶと署名が上書きされる（冪等）。
    pub fn sign(&mut self, key: &[u8]) -> Result<(), StoreError> {
        let bytes = self.signable_bytes()?;
        let mut mac = HmacSha256::new_from_slice(key)
            .expect("HMAC は任意長のキーを受け付ける");
        mac.update(&bytes);
        let result = mac.finalize().into_bytes();
        self.signature = Some(hex::encode(result));
        Ok(())
    }

    /// 保存された `signature` を `key` で検証する。
    ///
    /// # Errors
    /// - `signature` が `None` の場合は `StoreError::Unsigned`
    /// - HMAC が一致しない場合は `StoreError::SignatureInvalid`
    pub fn verify_signature(&self, key: &[u8]) -> Result<(), StoreError> {
        let stored_hex = self.signature.as_deref().ok_or(StoreError::Unsigned)?;
        let stored_bytes = hex::decode(stored_hex).map_err(|_| StoreError::SignatureInvalid)?;

        let bytes = self.signable_bytes()?;
        let mut mac = HmacSha256::new_from_slice(key)
            .expect("HMAC は任意長のキーを受け付ける");
        mac.update(&bytes);

        // constant-time 比較（タイミング攻撃対策）
        mac.verify_slice(&stored_bytes).map_err(|_| StoreError::SignatureInvalid)
    }

    // -----------------------------------------------------------------------
    // ファイル I/O（wasm32 では利用不可）
    // -----------------------------------------------------------------------

    /// JSON ファイルへ書き出す（署名なし）。
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<(), StoreError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// 署名してから JSON ファイルへ書き出す。
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_signed(&mut self, path: impl AsRef<Path>, key: &[u8]) -> Result<(), StoreError> {
        self.sign(key)?;
        self.save_to_file(path)
    }

    /// JSON ファイルから読み込む（検証は呼び出し元が行う）。
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let raw = std::fs::read_to_string(path)?;
        let record: Self = serde_json::from_str(&raw)?;
        Ok(record)
    }

    /// ファイルから読み込み、整合性を検証して `ProofToken` を返す（署名なし版）。
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_and_verify(path: impl AsRef<Path>) -> Result<ProofToken, StoreError> {
        let record = Self::load_from_file(path)?;
        record.verify_integrity()
    }

    /// ファイルから読み込み、**HMAC 署名** と **constraint_hash** の両方を検証して
    /// `ProofToken` を返す。二段構えの完全検証。
    ///
    /// # Errors
    /// - HMAC 不一致 → `StoreError::SignatureInvalid`
    /// - 制約ハッシュ不一致 → `StoreError::Tampered`
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_and_verify_signed(
        path: impl AsRef<Path>,
        key:  &[u8],
    ) -> Result<ProofToken, StoreError> {
        let record = Self::load_from_file(path)?;
        record.verify_signature(key)?;   // まず署名を検証
        record.verify_integrity()        // 次に内部整合性を検証
    }
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::verifier::{MockVerifier, Verifier};
    use std::fs;

    const TEST_KEY: &[u8] = b"hammurabi-test-secret-key-32byte";

    fn tmp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    fn make_constraints() -> Vec<Constraint> {
        vec![
            Constraint::NonNull,
            Constraint::InRange { min: 1, max: 100 },
        ]
    }

    fn make_record() -> (ProofToken, ProofRecord) {
        let verifier    = MockVerifier::default();
        let constraints = make_constraints();
        let token       = verifier.verify_constraints(&42_i64, &constraints).unwrap();
        let record      = ProofRecord::from_token(&token, "safe_division", "divisor", constraints).unwrap();
        (token, record)
    }

    // --- 既存テスト（署名なし）---------------------------------------------------

    #[test]
    fn proof_record_roundtrip_via_file() {
        let (token, record) = make_record();
        let path = tmp_path("hammurabi_test_roundtrip.proof.json");
        record.save_to_file(&path).unwrap();

        let restored = ProofRecord::load_and_verify(&path).unwrap();
        assert_eq!(restored.constraint_hash, token.constraint_hash);
        assert_eq!(restored.backend,         token.backend);
        fs::remove_file(path).ok();
    }

    #[test]
    fn proof_record_json_is_human_readable() {
        let (_, record) = make_record();
        let json = serde_json::to_string_pretty(&record).unwrap();

        assert!(json.contains("\"goal_name\""));
        assert!(json.contains("\"rail_id\""));
        assert!(json.contains("\"backend\""));
        assert!(json.contains("\"constraint_hash\""));
        assert!(json.contains("\"constraints\""));
        assert!(json.contains("\"NonNull\""));
        assert!(json.contains("\"InRange\""));
    }

    #[test]
    fn tampered_hash_is_detected() {
        let (_, mut record) = make_record();
        record.constraint_hash = record.constraint_hash.wrapping_add(1);
        assert!(matches!(record.verify_integrity(), Err(StoreError::Tampered { .. })));
    }

    #[test]
    fn tampered_constraints_are_detected() {
        let (_, mut record) = make_record();
        record.constraints = vec![Constraint::NonEmpty];
        assert!(matches!(record.verify_integrity(), Err(StoreError::Tampered { .. })));
    }

    #[test]
    fn empty_constraints_returns_error() {
        let verifier    = MockVerifier::default();
        let constraints = make_constraints();
        let token       = verifier.verify_constraints(&1_i64, &constraints).unwrap();
        let result      = ProofRecord::from_token(&token, "goal", "rail", vec![]);
        assert!(matches!(result, Err(StoreError::EmptyConstraints)));
    }

    // --- HMAC 署名テスト --------------------------------------------------------

    #[test]
    fn signed_record_roundtrip_via_file() {
        let (token, mut record) = make_record();
        let path = tmp_path("hammurabi_test_signed.proof.json");
        record.save_signed(&path, TEST_KEY).unwrap();

        // 署名が JSON に書き込まれていること
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"signature\""));
        assert!(!raw.contains("null")); // Some(hex) のはず

        // 署名 + 整合性の両方を検証してトークンが復元できること
        let restored = ProofRecord::load_and_verify_signed(&path, TEST_KEY).unwrap();
        assert_eq!(restored.constraint_hash, token.constraint_hash);
        fs::remove_file(path).ok();
    }

    #[test]
    fn wrong_key_is_rejected() {
        let (_, mut record) = make_record();
        let path = tmp_path("hammurabi_test_wrong_key.proof.json");
        record.save_signed(&path, TEST_KEY).unwrap();

        let result = ProofRecord::load_and_verify_signed(&path, b"wrong-key");
        assert!(matches!(result, Err(StoreError::SignatureInvalid)));
        fs::remove_file(path).ok();
    }

    #[test]
    fn file_tampering_after_sign_is_rejected() {
        let (_, mut record) = make_record();
        let path = tmp_path("hammurabi_test_file_tamper.proof.json");
        record.save_signed(&path, TEST_KEY).unwrap();

        // ファイルを直接書き換えて goal_name を改竄
        let raw = fs::read_to_string(&path).unwrap();
        let tampered = raw.replace("\"safe_division\"", "\"evil_goal\"");
        fs::write(&path, tampered).unwrap();

        let result = ProofRecord::load_and_verify_signed(&path, TEST_KEY);
        assert!(matches!(result, Err(StoreError::SignatureInvalid)));
        fs::remove_file(path).ok();
    }

    #[test]
    fn verify_signature_on_unsigned_record_returns_error() {
        let (_, record) = make_record(); // sign() 未呼び出し
        assert!(matches!(record.verify_signature(TEST_KEY), Err(StoreError::Unsigned)));
    }
}
