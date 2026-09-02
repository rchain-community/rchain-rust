import { secp256k1 } from "@noble/curves/secp256k1.js";

const HEX_64 = /^[0-9a-f]{64}$/;

export function generatePrivateKey(): Uint8Array {
  return secp256k1.utils.randomSecretKey();
}

export function importPrivateKey(hex: string): Uint8Array {
  const normalized = hex.trim().toLowerCase().replace(/^0x/, "");
  if (!HEX_64.test(normalized)) {
    throw new Error("Private key must be 32 bytes of hex (64 hex characters).");
  }
  const key = hexToPrivateKey(normalized);
  if (!secp256k1.utils.isValidSecretKey(key)) {
    throw new Error("Private key is not a valid secp256k1 scalar.");
  }
  return key;
}

export function privateKeyToHex(key: Uint8Array): string {
  return Array.from(key)
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

export function hexToPrivateKey(hex: string): Uint8Array {
  const normalized = hex.trim().toLowerCase().replace(/^0x/, "");
  if (!HEX_64.test(normalized)) {
    throw new Error("Expected 32 bytes of hex (64 hex characters).");
  }
  const bytes = new Uint8Array(32);
  for (let i = 0; i < 32; i += 1) {
    bytes[i] = parseInt(normalized.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}
