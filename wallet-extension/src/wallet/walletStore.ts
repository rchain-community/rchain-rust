const STORAGE_KEY = "rev-wallet:v1";
const PBKDF2_ITERATIONS = 210_000;

export interface StoredWalletRecord {
  version: 1;
  salt: string;
  iv: string;
  ciphertext: string;
  publicKeyHex: string;
  revAddress: string;
  actorDid: string;
}

export interface WalletPublicInfo {
  publicKeyHex: string;
  revAddress: string;
  actorDid: string;
}

/**
 * Async because the real backing store (chrome.storage.local, wired up via
 * ../lib/chromeStorageAdapter.ts) is inherently async. A synchronous
 * localStorage-style adapter is also wrapped below for use outside the
 * extension (tests, or a future non-extension embedding of this module).
 */
export interface WalletStorageAdapter {
  getItem(key: string): Promise<string | null>;
  setItem(key: string, value: string): Promise<void>;
  removeItem(key: string): Promise<void>;
}

function defaultStorage(): WalletStorageAdapter {
  if (typeof globalThis.localStorage !== "undefined") {
    const storage = globalThis.localStorage;
    return {
      getItem: async (key) => storage.getItem(key),
      setItem: async (key, value) => storage.setItem(key, value),
      removeItem: async (key) => storage.removeItem(key)
    };
  }
  throw new Error("No storage adapter available; pass one explicitly.");
}

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i += 1) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

/** TS 5.7+ made Uint8Array generic over its buffer type, and DOM's
 * BufferSource union requires the narrower Uint8Array<ArrayBuffer>. The byte
 * arrays passed through this module are always freshly allocated, never
 * SharedArrayBuffer-backed, so this reflects a real invariant the type
 * system can't express here, not a runtime risk. */
function asBufferSource(bytes: Uint8Array): BufferSource {
  return bytes as unknown as BufferSource;
}

async function deriveAesKey(passphrase: string, salt: Uint8Array): Promise<CryptoKey> {
  const passphraseKey = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(passphrase),
    "PBKDF2",
    false,
    ["deriveKey"]
  );
  return crypto.subtle.deriveKey(
    { name: "PBKDF2", salt: asBufferSource(salt), iterations: PBKDF2_ITERATIONS, hash: "SHA-256" },
    passphraseKey,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"]
  );
}

/** Encrypts only the raw private key bytes; public info (address/pubkey/DID)
 * is stored in the clear since it isn't secret. The plaintext key is never
 * written to storage. */
export async function encryptAndStoreKey(
  privateKey: Uint8Array,
  passphrase: string,
  publicInfo: WalletPublicInfo,
  storage: WalletStorageAdapter = defaultStorage()
): Promise<StoredWalletRecord> {
  const salt = crypto.getRandomValues(new Uint8Array(16));
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const aesKey = await deriveAesKey(passphrase, salt);
  const ciphertext = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: asBufferSource(iv) },
    aesKey,
    asBufferSource(privateKey)
  );
  const record: StoredWalletRecord = {
    version: 1,
    salt: bytesToHex(salt),
    iv: bytesToHex(iv),
    ciphertext: bytesToHex(new Uint8Array(ciphertext)),
    publicKeyHex: publicInfo.publicKeyHex,
    revAddress: publicInfo.revAddress,
    actorDid: publicInfo.actorDid
  };
  await storage.setItem(STORAGE_KEY, JSON.stringify(record));
  return record;
}

export async function loadStoredWalletRecord(
  storage: WalletStorageAdapter = defaultStorage()
): Promise<StoredWalletRecord | null> {
  const raw = await storage.getItem(STORAGE_KEY);
  if (!raw) {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as StoredWalletRecord;
    return parsed.version === 1 ? parsed : null;
  } catch {
    return null;
  }
}

/** Decrypts and returns the private key. Throws (does not silently return
 * garbage) on a wrong passphrase, since AES-GCM's auth tag fails to verify. */
export async function unlockStoredKey(
  passphrase: string,
  storage: WalletStorageAdapter = defaultStorage()
): Promise<Uint8Array> {
  const record = await loadStoredWalletRecord(storage);
  if (!record) {
    throw new Error("No stored wallet found.");
  }
  const aesKey = await deriveAesKey(passphrase, hexToBytes(record.salt));
  try {
    const plaintext = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: asBufferSource(hexToBytes(record.iv)) },
      aesKey,
      asBufferSource(hexToBytes(record.ciphertext))
    );
    return new Uint8Array(plaintext);
  } catch {
    throw new Error("Incorrect passphrase.");
  }
}

export async function clearStoredWallet(storage: WalletStorageAdapter = defaultStorage()): Promise<void> {
  await storage.removeItem(STORAGE_KEY);
}

// Not used by the extension itself (a plain module variable doesn't survive
// MV3 service-worker eviction - see ../lib/sessionKey.ts, which uses
// chrome.storage.session instead). Kept here for non-extension embeddings
// of this module, where the decrypted key can safely live in memory for a
// page's lifetime.
let unlockedPrivateKey: Uint8Array | null = null;

export function getUnlockedPrivateKey(): Uint8Array | null {
  return unlockedPrivateKey;
}

export function setUnlockedPrivateKey(key: Uint8Array | null): void {
  unlockedPrivateKey = key;
}

export function lockSession(): void {
  unlockedPrivateKey = null;
}
