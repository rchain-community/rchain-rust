import { deriveRevAddress, deriveUncompressedPublicKey, actorDidFromPublicKey, bytesToLowercaseHex } from "./wallet/revAddress.ts";
import { generatePrivateKey, importPrivateKey } from "./wallet/keys.ts";
import { signDeploy } from "./wallet/deploySigning.ts";
import { encryptAndStoreKey, loadStoredWalletRecord, unlockStoredKey } from "./wallet/walletStore.ts";
import { chromeStorageAdapter } from "./lib/chromeStorageAdapter.ts";
import { getUnlockedPrivateKey, setUnlockedPrivateKey, lockSession } from "./lib/sessionKey.ts";
import { connect, disconnect, isConnected, listConnections } from "./lib/connections.ts";
import type {
  BackgroundInboundMessage,
  PageRequestMessage,
  PendingRequestInfo,
  WalletAccountInfo,
  WalletBackgroundState,
  WalletStatus
} from "./lib/messages.ts";

// The only module in this extension that ever imports a signing function
// with real key access. Everything else (content script, injected page
// provider, popup) only ever sees this file's message-passing surface.

const storage = chromeStorageAdapter();

interface PendingEntry {
  info: PendingRequestInfo;
  resolve: (approved: boolean) => void;
}

let pending: PendingEntry | null = null;
let popupWindowId: number | null = null;

async function currentStatus(): Promise<WalletStatus> {
  const record = await loadStoredWalletRecord(storage);
  if (!record) return "no-wallet";
  const key = await getUnlockedPrivateKey();
  return key ? "unlocked" : "locked";
}

async function accountInfo(): Promise<WalletAccountInfo | null> {
  const record = await loadStoredWalletRecord(storage);
  if (!record) return null;
  return { revAddress: record.revAddress, actorDid: record.actorDid, publicKeyHex: record.publicKeyHex };
}

async function publicInfoFor(privateKey: Uint8Array) {
  const publicKey = deriveUncompressedPublicKey(privateKey);
  return {
    publicKeyHex: bytesToLowercaseHex(publicKey),
    revAddress: deriveRevAddress(publicKey),
    actorDid: actorDidFromPublicKey(publicKey)
  };
}

async function openApprovalPopup(): Promise<void> {
  if (popupWindowId !== null) {
    try {
      await chrome.windows.update(popupWindowId, { focused: true });
      return;
    } catch {
      popupWindowId = null;
    }
  }
  const win = await chrome.windows.create({
    url: chrome.runtime.getURL("popup.html"),
    type: "popup",
    width: 380,
    height: 580
  });
  popupWindowId = win?.id ?? null;
}

chrome.windows.onRemoved.addListener((windowId) => {
  if (windowId === popupWindowId) {
    popupWindowId = null;
    // Only resolve here - requestApproval's own continuation clears
    // `pending`, so there is exactly one place that does it.
    if (pending) {
      pending.resolve(false);
    }
  }
});

/** Sets the single in-flight approval request and opens the popup for it.
 * Only one request is ever pending at a time - a second concurrent request
 * from another tab/origin waits its turn rather than racing the popup UI.
 * Clears `pending` itself once resolved, so callers never need to (and
 * can't forget to) do it themselves - that used to be duplicated at each
 * call site in handlePageRequest. */
async function requestApproval(info: PendingRequestInfo): Promise<boolean> {
  while (pending) {
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  const approved = await new Promise<boolean>((resolve) => {
    pending = { info, resolve };
    void openApprovalPopup();
  });
  pending = null;
  return approved;
}

async function handlePageRequest(message: PageRequestMessage): Promise<{ result: unknown } | { error: string }> {
  const { origin, method, params } = message;
  try {
    if (method === "requestAccounts") {
      const status = await currentStatus();
      if (status === "no-wallet") throw new Error("No wallet configured in the extension yet.");
      if (!(await isConnected(origin))) {
        const id = crypto.randomUUID();
        const approved = await requestApproval({ id, kind: "connect", origin });
        if (!approved) throw new Error("Connection request rejected.");
        await connect(origin);
      }
      const account = await accountInfo();
      if (!account) throw new Error("No wallet configured in the extension yet.");
      return { result: account };
    }

    if (method === "getAddress") {
      if (!(await isConnected(origin))) throw new Error("Not connected - call requestAccounts() first.");
      const account = await accountInfo();
      if (!account) throw new Error("No wallet configured in the extension yet.");
      return { result: account };
    }

    if (method === "signDeploy") {
      if (!(await isConnected(origin))) throw new Error("Not connected - call requestAccounts() first.");
      if (!params) throw new Error("Missing deploy fields.");
      const id = crypto.randomUUID();
      const approved = await requestApproval({ id, kind: "sign", origin, fields: params });
      if (!approved) throw new Error("Signing request rejected.");
      const privateKey = await getUnlockedPrivateKey();
      if (!privateKey) throw new Error("Wallet is locked.");
      const envelope = signDeploy(params, privateKey);
      return { result: envelope };
    }

    throw new Error(`Unknown method: ${method}`);
  } catch (error) {
    return { error: error instanceof Error ? error.message : String(error) };
  }
}

async function getState(): Promise<WalletBackgroundState> {
  return {
    status: await currentStatus(),
    account: await accountInfo(),
    connections: await listConnections(),
    pendingRequest: pending?.info ?? null,
    error: null
  };
}

const POPUP_URL = chrome.runtime.getURL("popup.html");

/** Only page-request may originate from a content script (any tab); every
 * other channel is popup-only. Since only extension contexts can call
 * chrome.runtime.sendMessage against this listener in the first place, a
 * page itself cannot reach any of this today - but a compromised or
 * mis-wired content script could otherwise forward an arbitrary channel
 * (e.g. a spoofed "popup-decision": approved: true), so this is checked
 * explicitly rather than relied on implicitly. */
function isFromPopup(sender: chrome.runtime.MessageSender): boolean {
  return sender.id === chrome.runtime.id && sender.url === POPUP_URL;
}

chrome.runtime.onMessage.addListener((message: BackgroundInboundMessage, sender, sendResponse) => {
  (async () => {
    if (message.channel === "page-request") {
      if (sender.id !== chrome.runtime.id) return;
      sendResponse(await handlePageRequest(message));
      return;
    }

    if (!isFromPopup(sender)) return;

    if (message.channel === "popup-decision") {
      if (pending && pending.info.id === message.requestId) {
        pending.resolve(message.approved);
      }
      sendResponse(await getState());
      return;
    }

    if (message.channel === "popup-generate") {
      const privateKey = generatePrivateKey();
      await encryptAndStoreKey(privateKey, message.passphrase, await publicInfoFor(privateKey), storage);
      await setUnlockedPrivateKey(privateKey);
      sendResponse(await getState());
      return;
    }

    if (message.channel === "popup-import") {
      const privateKey = importPrivateKey(message.privateKeyHex);
      await encryptAndStoreKey(privateKey, message.passphrase, await publicInfoFor(privateKey), storage);
      await setUnlockedPrivateKey(privateKey);
      sendResponse(await getState());
      return;
    }

    if (message.channel === "popup-unlock") {
      try {
        const privateKey = await unlockStoredKey(message.passphrase, storage);
        await setUnlockedPrivateKey(privateKey);
        sendResponse(await getState());
      } catch (error) {
        sendResponse({ ...(await getState()), error: error instanceof Error ? error.message : "Incorrect passphrase." });
      }
      return;
    }

    if (message.channel === "popup-lock") {
      await lockSession();
      sendResponse(await getState());
      return;
    }

    if (message.channel === "popup-disconnect") {
      await disconnect(message.origin);
      sendResponse(await getState());
      return;
    }

    if (message.channel === "popup-get-state") {
      sendResponse(await getState());
      return;
    }
  })();
  return true;
});
