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
    pendingCalls.set(id, { resolve, reject });
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

window.revWallet = revWalletProvider;
