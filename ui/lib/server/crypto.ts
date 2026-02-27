import crypto from "crypto";

const PBKDF2_ITERATIONS = 600_000;
const KEY_LENGTH = 32;
const IV_LENGTH = 12;
const AUTH_TAG_LENGTH = 16;

export function deriveKey(password: string, salt: string): Buffer {
  return crypto.pbkdf2Sync(
    password,
    Buffer.from(salt, "base64"),
    PBKDF2_ITERATIONS,
    KEY_LENGTH,
    "sha256"
  );
}

export function encryptPK(plaintext: string, derivedKey: Buffer): string {
  const iv = crypto.randomBytes(IV_LENGTH);
  const cipher = crypto.createCipheriv("aes-256-gcm", derivedKey, iv);

  const encrypted = Buffer.concat([cipher.update(plaintext, "utf8"), cipher.final()]);
  const authTag = cipher.getAuthTag();

  // iv (12) + ciphertext + authTag (16)
  return Buffer.concat([iv, encrypted, authTag]).toString("base64");
}

export function decryptPK(encrypted: string, derivedKey: Buffer): string {
  const buf = Buffer.from(encrypted, "base64");

  const iv = buf.subarray(0, IV_LENGTH);
  const authTag = buf.subarray(buf.length - AUTH_TAG_LENGTH);
  const ciphertext = buf.subarray(IV_LENGTH, buf.length - AUTH_TAG_LENGTH);

  const decipher = crypto.createDecipheriv("aes-256-gcm", derivedKey, iv);
  decipher.setAuthTag(authTag);

  return decipher.update(ciphertext) + decipher.final("utf8");
}

export function generateSalt(): string {
  return crypto.randomBytes(32).toString("base64");
}
