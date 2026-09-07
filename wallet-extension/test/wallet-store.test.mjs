import { test } from "node:test";
import assert from "node:assert/strict";
import {
  encryptAndStoreKey,
  loadStoredWalletRecord,
  unlockStoredKey,
  clearStoredWallet
} from "../src/wallet/walletStore.ts";
import { generatePrivateKey, privateKeyToHex } from "../src/wallet/keys.ts";
import { deriveUncompressedPublicKey, deriveRevAddress, actorDidFromPublicKey, bytesToLowercaseHex } from "../src/wallet/revAddress.ts";

function createMemoryStorage() {
  const map = new Map();
  return {
    getItem: async (key) => (map.has(key) ? map.get(key) : null),
    setItem: async (key, value) => void map.set(key, value),
    removeItem: async (key) => void map.delete(key)
  };
}

function publicInfoFor(privateKey) {
  const publicKey = deriveUncompressedPublicKey(privateKey);
  return {
    publicKeyHex: bytesToLowercaseHex(publicKey),
    revAddress: deriveRevAddress(publicKey),
    actorDid: actorDidFromPublicKey(publicKey)
  };
}

test("crypto.subtle is reachable for PBKDF2/AES-GCM (Node test runner)", () => {
  assert.ok(globalThis.crypto?.subtle);
});

test("encrypt/store/unlock round-trips the original private key bytes", async () => {
  const storage = createMemoryStorage();
  const privateKey = generatePrivateKey();
  const info = publicInfoFor(privateKey);

  await encryptAndStoreKey(privateKey, "correct horse battery staple", info, storage);
  const unlocked = await unlockStoredKey("correct horse battery staple", storage);

  assert.equal(privateKeyToHex(unlocked), privateKeyToHex(privateKey));
});

test("public info is readable without unlocking", async () => {
  const storage = createMemoryStorage();
  const privateKey = generatePrivateKey();
  const info = publicInfoFor(privateKey);
  await encryptAndStoreKey(privateKey, "passphrase", info, storage);

  const record = await loadStoredWalletRecord(storage);
  assert.equal(record.revAddress, info.revAddress);
  assert.equal(record.actorDid, info.actorDid);
  assert.equal(record.publicKeyHex, info.publicKeyHex);
  // The stored record must never contain the plaintext key.
  assert.ok(!JSON.stringify(record).includes(privateKeyToHex(privateKey)));
});

test("wrong passphrase throws rather than returning garbage bytes", async () => {
  const storage = createMemoryStorage();
  const privateKey = generatePrivateKey();
  const info = publicInfoFor(privateKey);
  await encryptAndStoreKey(privateKey, "right passphrase", info, storage);

  await assert.rejects(() => unlockStoredKey("wrong passphrase", storage));
});

test("loadStoredWalletRecord returns null when nothing is stored", async () => {
  const storage = createMemoryStorage();
  assert.equal(await loadStoredWalletRecord(storage), null);
});

test("clearStoredWallet removes the record", async () => {
  const storage = createMemoryStorage();
  const privateKey = generatePrivateKey();
  await encryptAndStoreKey(privateKey, "passphrase", publicInfoFor(privateKey), storage);
  assert.ok(await loadStoredWalletRecord(storage));

  await clearStoredWallet(storage);
  assert.equal(await loadStoredWalletRecord(storage), null);
});
