/**
 * Shared request/response protocol for the three-piece relay:
 * page (inject.ts) <-> content script (content-script.ts) <-> background
 * (background.ts). Kept dependency-free (no chrome.* types here) so it can
 * be imported from inject.ts, which runs in the page's own JS world and
 * must never touch extension APIs directly.
 */

export type WalletMethod = "requestAccounts" | "getAddress" | "signDeploy";

/** The same flat fields src/wallet/deploySigning.ts's signDeploy already
 * takes - the wallet signs whatever Rholang term it's handed and doesn't
 * know or care about any particular dApp's event schemas. */
export interface SignDeployFields {
  term: string;
  timestamp: number;
  phloPrice: number;
  phloLimit: number;
  validAfterBlockNumber: number;
  shardId: string;
}

export interface WalletAccountInfo {
  revAddress: string;
  actorDid: string;
  publicKeyHex: string;
}

export interface SignedDeployEnvelope extends SignDeployFields {
  deployer: string;
  signature: string;
  sigAlgorithm: "secp256k1";
}

export const PAGE_MESSAGE_SOURCE = "rev-wallet-page" as const;
export const EXTENSION_MESSAGE_SOURCE = "rev-wallet-extension" as const;

/** content script -> page, via window.postMessage */
export type PageResponseEvent =
  | { source: typeof EXTENSION_MESSAGE_SOURCE; id: string; result: WalletAccountInfo | SignedDeployEnvelope }
  | { source: typeof EXTENSION_MESSAGE_SOURCE; id: string; error: string };

/** content script -> background, via chrome.runtime.sendMessage. Carries the
 * page's origin, which the content script reads from its own document
 * (never trusted if it came from the page itself). */
export interface PageRequestMessage {
  channel: "page-request";
  origin: string;
  id: string;
  method: WalletMethod;
  params?: SignDeployFields;
}

/** popup -> background messages. */
export interface PopupDecisionMessage {
  channel: "popup-decision";
  requestId: string;
  approved: boolean;
}
export interface PopupGenerateMessage {
  channel: "popup-generate";
  passphrase: string;
}
export interface PopupImportMessage {
  channel: "popup-import";
  privateKeyHex: string;
  passphrase: string;
}
export interface PopupUnlockMessage {
  channel: "popup-unlock";
  passphrase: string;
}
export interface PopupLockMessage {
  channel: "popup-lock";
}
export interface PopupDisconnectMessage {
  channel: "popup-disconnect";
  origin: string;
}
export interface PopupGetStateMessage {
  channel: "popup-get-state";
}

export type BackgroundInboundMessage =
  | PageRequestMessage
  | PopupDecisionMessage
  | PopupGenerateMessage
  | PopupImportMessage
  | PopupUnlockMessage
  | PopupLockMessage
  | PopupDisconnectMessage
  | PopupGetStateMessage;

export type PendingRequestInfo =
  | { id: string; kind: "connect"; origin: string }
  | { id: string; kind: "sign"; origin: string; fields: SignDeployFields };

export type WalletStatus = "no-wallet" | "locked" | "unlocked";

/** What the popup renders itself from - fetched via a "popup-get-state"
 * message and re-fetched after every action. */
export interface WalletBackgroundState {
  status: WalletStatus;
  account: WalletAccountInfo | null;
  connections: string[];
  pendingRequest: PendingRequestInfo | null;
  error: string | null;
}
