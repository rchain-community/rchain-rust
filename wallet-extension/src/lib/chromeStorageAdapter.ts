import type { WalletStorageAdapter } from "../wallet/walletStore.ts";

/** Implements wallet/walletStore.ts's WalletStorageAdapter against
 * chrome.storage.local, so encryptAndStoreKey/loadStoredWalletRecord/
 * unlockStoredKey/clearStoredWallet work here unmodified - see the async
 * WalletStorageAdapter interface's own comment for why it's Promise-based.
 * Persisted (survives browser restart), unlike sessionKey.ts's
 * unlocked-key storage. */
export function chromeStorageAdapter(area: chrome.storage.StorageArea = chrome.storage.local): WalletStorageAdapter {
  return {
    async getItem(key) {
      const result = await area.get(key);
      return typeof result[key] === "string" ? result[key] : null;
    },
    async setItem(key, value) {
      await area.set({ [key]: value });
    },
    async removeItem(key) {
      await area.remove(key);
    }
  };
}
