use aes_gcm::aead::{Aead, KeyInit, generic_array::GenericArray};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, bail};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use pbkdf2::pbkdf2_hmac;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

const PBKDF2_ITERATIONS: u32 = 600_000;
const KEY_LENGTH: usize = 32;
const IV_LENGTH: usize = 12;
const JWT_EXPIRY_DAYS: i64 = 7;

// ── Password hashing (bcrypt) ──────────────────────────

pub fn hash_password(password: &str) -> Result<String> {
    bcrypt::hash(password, 12).context("hashing password")
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

// ── Key derivation (PBKDF2-SHA256) ─────────────────────

pub fn derive_key(password: &str, salt_b64: &str) -> Result<[u8; KEY_LENGTH]> {
    let salt = base64_decode(salt_b64).context("decoding salt")?;
    let mut key = [0u8; KEY_LENGTH];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ITERATIONS, &mut key);
    Ok(key)
}

pub fn generate_salt() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    base64_encode(&bytes)
}

// ── AES-256-GCM encrypt/decrypt ────────────────────────
// Format: base64(iv[12] || ciphertext || auth_tag[16])
// Matches the Node.js implementation in ui/lib/server/crypto.ts

pub fn encrypt_pk(plaintext: &str, derived_key: &[u8; KEY_LENGTH]) -> Result<String> {
    let key = GenericArray::from_slice(derived_key);
    let cipher = Aes256Gcm::new(key);

    use rand::Rng;
    let iv_bytes: [u8; IV_LENGTH] = rand::rng().random();
    let nonce = Nonce::from_slice(&iv_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;

    // aes-gcm appends auth tag to ciphertext already
    let mut result = Vec::with_capacity(IV_LENGTH + ciphertext.len());
    result.extend_from_slice(&iv_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(base64_encode(&result))
}

pub fn decrypt_pk(encrypted_b64: &str, derived_key: &[u8; KEY_LENGTH]) -> Result<String> {
    let data = base64_decode(encrypted_b64).context("decoding encrypted pk")?;
    if data.len() < IV_LENGTH + 16 {
        bail!("encrypted data too short");
    }

    let key = GenericArray::from_slice(derived_key);
    let cipher = Aes256Gcm::new(key);

    let nonce = Nonce::from_slice(&data[..IV_LENGTH]);
    let ciphertext_with_tag = &data[IV_LENGTH..];

    let plaintext = cipher
        .decrypt(nonce, ciphertext_with_tag)
        .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))?;

    String::from_utf8(plaintext).context("decrypted pk is not valid utf8")
}

// ── JWT ─────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user_id
    pub exp: usize,
}

pub fn create_jwt(user_id: &str, secret: &str) -> Result<String> {
    let exp = (chrono::Utc::now() + chrono::Duration::days(JWT_EXPIRY_DAYS)).timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .context("creating jwt")
}

pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .context("invalid token")?;
    Ok(data.claims)
}

// ── Base64 helpers ──────────────────────────────────────

fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        let _ = write!(s, "{}", alphabet[((n >> 18) & 63) as usize] as char);
        let _ = write!(s, "{}", alphabet[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            let _ = write!(s, "{}", alphabet[((n >> 6) & 63) as usize] as char);
        } else {
            s.push('=');
        }
        if chunk.len() > 2 {
            let _ = write!(s, "{}", alphabet[(n & 63) as usize] as char);
        } else {
            s.push('=');
        }
    }
    s
}

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    let input = input.trim_end_matches('=');
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &c) in alphabet.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }

    let mut result = Vec::new();
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| lookup[*b as usize] != 255)
        .collect();

    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &b) in chunk.iter().enumerate() {
            n |= (lookup[b as usize] as u32) << (18 - 6 * i);
        }
        result.push((n >> 16) as u8);
        if chunk.len() > 2 {
            result.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            result.push(n as u8);
        }
    }

    Ok(result)
}
