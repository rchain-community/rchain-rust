import { hexToPrivateKey, privateKeyToHex } from "../wallet/keys.ts";

const SESSION_STORAGE_KEY = "rev-wallet:unlocked-key-hex";

/**
 * The unlocked-key equivalent of wallet/walletStore.ts's
 * getUnlockedPrivateKey/setUnlockedPrivateKey/lockSession, but backed by
 * chrome.storage.session instead of a plain module variable: an MV3 service
 * worker gets killed after ~30s of inactivity and restarts on the next
 * message, so a module-level variable would force re-unlocking constantly.
 * chrome.storage.session is the purpose-built primitive here - memory-only
 * (never written to disk), cleared when the browser closes, but it survives
 * service-worker eviction within a session.
 */
export async function getUnlockedPrivateKey(area: chrome.storage.StorageArea = chrome.storage.session): Promise<Uint8Array | null> {
  const result = await area.get(SESSION_STORAGE_KEY);
  const hex = result[SESSION_STORAGE_KEY];
  return typeof hex === "string" ? hexToPrivateKey(hex) : null;
}

export async function setUnlockedPrivateKey(
  key: Uint8Array,
  area: chrome.storage.StorageArea = chrome.storage.session
): Promise<void> {
  await area.set({ [SESSION_STORAGE_KEY]: privateKeyToHex(key) });
}

export async function lockSession(area: chrome.storage.StorageArea = chrome.storage.session): Promise<void> {
  await area.remove(SESSION_STORAGE_KEY);
}
