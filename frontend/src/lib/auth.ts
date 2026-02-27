import { cookies } from "next/headers";

// Use dynamic import for crypto to avoid webpack bundling issues
let createHash: typeof import("crypto").createHash;
let createHmac: typeof import("crypto").createHmac;
let timingSafeEqual: typeof import("crypto").timingSafeEqual;

// Initialize crypto functions lazily
function getCrypto() {
  if (!createHash) {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const crypto = require("crypto");
    createHash = crypto.createHash;
    createHmac = crypto.createHmac;
    timingSafeEqual = crypto.timingSafeEqual;
  }
}

// ─── Configuration from environment ─────────────────────────────────────────
// SECURITY: All credentials MUST come from environment variables in production.
// The server will refuse to start if these are not set (see validateAuthConfig).

const SESSION_COOKIE = "apex_session";

/**
 * Validate that all required auth environment variables are set.
 * Call this at startup to fail fast with clear error messages.
 */
export function validateAuthConfig(): string[] {
  const errors: string[] = [];
  
  if (!process.env.APEX_ADMIN_USERNAME) {
    errors.push("APEX_ADMIN_USERNAME is not set");
  }
  if (!process.env.APEX_ADMIN_PASSWORD_HASH) {
    errors.push("APEX_ADMIN_PASSWORD_HASH is not set (use: echo -n 'yourpassword' | sha256sum)");
  }
  if (!process.env.SESSION_SECRET) {
    errors.push("SESSION_SECRET is not set (use: openssl rand -hex 32)");
  } else if (process.env.SESSION_SECRET.length < 32) {
    errors.push("SESSION_SECRET must be at least 32 characters");
  }
  
  return errors;
}

/**
 * Get the configured admin username.
 * Returns empty string if not configured (validation will catch this).
 */
function getValidUsername(): string {
  return process.env.APEX_ADMIN_USERNAME || "";
}

/**
 * Get the preconfigured password hash (SHA-256 hex).
 * The password should be hashed externally: echo -n 'password' | sha256sum
 */
function getValidPasswordHash(): string {
  return process.env.APEX_ADMIN_PASSWORD_HASH || "";
}

/**
 * Get the session secret for HMAC signing.
 * Throws in production if not set (enforced by validateAuthConfig at startup).
 */
function getSessionSecret(): string {
  const secret = process.env.SESSION_SECRET;
  if (!secret) {
    // In development, provide a clear error. In production, this should never
    // be reached because validateAuthConfig() should be called at startup.
    throw new Error("SESSION_SECRET environment variable is not set");
  }
  return secret;
}

/** Generate a signed session token */
export function createSessionToken(username: string): string {
  getCrypto();
  const secret = getSessionSecret();
  const payload = JSON.stringify({ sub: username, iat: Date.now() });
  const hmac = createHmac("sha256", secret).update(payload).digest("hex");
  const token = Buffer.from(payload).toString("base64url") + "." + hmac;
  return token;
}

/** Verify a session token, return payload or null */
export function verifySessionToken(token: string): { sub: string; iat: number } | null {
  try {
    getCrypto();
    const secret = getSessionSecret();
    const [payloadB64, sig] = token.split(".");
    if (!payloadB64 || !sig) return null;
    const payload = Buffer.from(payloadB64, "base64url").toString();
    const expected = createHmac("sha256", secret).update(payload).digest("hex");
    // Constant-time comparison
    if (!timingSafeEqual(Buffer.from(sig), Buffer.from(expected))) return null;
    const data = JSON.parse(payload);
    // Sessions expire after 24 hours
    if (Date.now() - data.iat > 24 * 60 * 60 * 1000) return null;
    if (!data.sub) return null;
    return { sub: data.sub, iat: data.iat };
  } catch {
    return null;
  }
}

/** Validate username + password, return true if valid */
export function validateCredentials(username: string, password: string): boolean {
  const validUsername = getValidUsername();
  const validPasswordHash = getValidPasswordHash();
  
  // Fail closed if credentials aren't configured
  if (!validUsername || !validPasswordHash) {
    return false;
  }
  
  if (username !== validUsername) return false;
  getCrypto();
  const hash = createHash("sha256").update(password).digest("hex");
  
  // Constant-time comparison to prevent timing attacks
  try {
    return timingSafeEqual(Buffer.from(hash), Buffer.from(validPasswordHash));
  } catch {
    // This can happen if the hash lengths don't match (misconfiguration)
    return false;
  }
}

/** Get the session cookie name */
export function getSessionCookieName(): string {
  return SESSION_COOKIE;
}
