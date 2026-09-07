import type {
  PageResponseEvent,
  SignDeployFields,
  SignedDeployEnvelope,
  WalletAccountInfo,
  WalletMethod
} from "./lib/messages.ts";

// Runs in the page's own JS world (manifest.json's "world": "MAIN" content
// script) - this file, and only this file, is what a page ever sees. No key
// material, no storage access, no extension API: just three methods that
// each round-trip through content-script.ts to the background service
// worker and back, gated by that page's own explicit connection + the
// user's per-request approval in the extension popup.
//
// Only a type-only import from ./lib/messages.ts above (erased at compile
// time) - a content script (this file included, per manifest.json's
// "world": "MAIN" entry) is always loaded as a classic script, never as an
// ES module, so a real runtime import isn't available here. See
// content-script.ts's matching comment; vite.config.content-script.ts and
// vite.config.inject.ts build each as its own independent IIFE bundle.
const EXTENSION_MESSAGE_SOURCE = "rev-wallet-extension";
const PAGE_MESSAGE_SOURCE = "rev-wallet-page";

// Generous enough not to interrupt a real human approving in the popup, but
// bounded so a page's promise can never hang forever - e.g. if the popup is
// closed in a way background.ts doesn't observe, or the extension context
// goes away mid-request (content-script.ts handles the latter directly and
// responds with an error immediately, but this is the backstop either way).
const REQUEST_TIMEOUT_MS = 120_000;

const pendingCalls = new Map<string, { resolve: (value: unknown) => void; reject: (reason: Error) => void }>();

window.addEventListener("message", (event) => {
  if (event.source !== window) return;
  const data = event.data as Partial<PageResponseEvent> | undefined;
  if (!data || data.source !== EXTENSION_MESSAGE_SOURCE || typeof data.id !== "string") return;

  const call = pendingCalls.get(data.id);
  if (!call) return;
  pendingCalls.delete(data.id);

  if ("error" in data && data.error) {
    call.reject(new Error(data.error));
  } else if ("result" in data) {
    call.resolve(data.result);
  }
});

function call(method: WalletMethod, params?: SignDeployFields): Promise<unknown> {
  const id = crypto.randomUUID();
  return new Promise((resolve, reject) => {
    const timeoutId = setTimeout(() => {
      pendingCalls.delete(id);
      reject(new Error(`REV Wallet request "${method}" timed out after ${REQUEST_TIMEOUT_MS / 1000}s with no response from the extension.`));
    }, REQUEST_TIMEOUT_MS);
    pendingCalls.set(id, {
      resolve: (value) => {
        clearTimeout(timeoutId);
        resolve(value);
      },
      reject: (reason) => {
        clearTimeout(timeoutId);
        reject(reason);
      }
    });
    window.postMessage({ source: PAGE_MESSAGE_SOURCE, id, method, params }, window.location.origin);
  });
}

const revWalletProvider = {
  requestAccounts: (): Promise<WalletAccountInfo> => call("requestAccounts") as Promise<WalletAccountInfo>,
  getAddress: (): Promise<WalletAccountInfo> => call("getAddress") as Promise<WalletAccountInfo>,
  signDeploy: (fields: SignDeployFields): Promise<SignedDeployEnvelope> =>
    call("signDeploy", fields) as Promise<SignedDeployEnvelope>
};

declare global {
  interface Window {
    revWallet?: typeof revWalletProvider;
  }
}

// Never clobber another provider that got here first - e.g. a second copy
// of this same extension injected twice, or (in principle) an unrelated
// provider that happens to use this name. A real multi-provider pattern
// (each wallet announcing itself, à la EIP-6963) is the fuller fix if this
// ever needs to coexist with other RChain wallets, but isn't needed yet -
// at minimum, don't silently overwrite what's already there.
if (!window.revWallet) {
  window.revWallet = revWalletProvider;
} else {
  console.warn("[REV Wallet] window.revWallet is already defined; not overwriting it.");
}
