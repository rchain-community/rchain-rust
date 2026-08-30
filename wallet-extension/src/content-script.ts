import type { PageRequestMessage } from "./lib/messages.ts";

// Isolated world: relays window.postMessage (from inject.ts, running in the
// MAIN world on the same page) to the background service worker, and posts
// the response back the same way. Never touches key material itself - it's
// a dumb pipe with one piece of context the page can't be trusted to
// provide honestly: the real origin, read from this document, not from the
// message payload.
//
// A Chrome content script declared via manifest.json's "js" is always
// loaded as a classic (non-module) script, never "type": "module" (that's
// only available to the background service worker) - so this file cannot
// have a real runtime import from ./lib/messages.ts (only the type-only
// import above, erased at compile time). The two source-tag constants are
// duplicated here and in inject.ts rather than shared, specifically so
// vite.config.content-script.ts and vite.config.inject.ts can each build a
// fully independent IIFE bundle (Rollup rejects code-split IIFE output).
const EXTENSION_MESSAGE_SOURCE = "rev-wallet-extension";
const PAGE_MESSAGE_SOURCE = "rev-wallet-page";

type IncomingPageRequest = Pick<PageRequestMessage, "id" | "method" | "params">;

function asPageRequest(value: unknown): IncomingPageRequest | null {
  const candidate = value as { source?: unknown; id?: unknown; method?: unknown; params?: unknown } | null;
  if (
    !candidate ||
    candidate.source !== PAGE_MESSAGE_SOURCE ||
    typeof candidate.id !== "string" ||
    typeof candidate.method !== "string"
  ) {
    return null;
  }
  return { id: candidate.id, method: candidate.method as IncomingPageRequest["method"], params: candidate.params as IncomingPageRequest["params"] };
}

window.addEventListener("message", (event) => {
  if (event.source !== window) return;
  const incoming = asPageRequest(event.data);
  if (!incoming) return;

  const { id, method, params } = incoming;
  const request: PageRequestMessage = { channel: "page-request", origin: window.location.origin, id, method, params };

  chrome.runtime.sendMessage(request, (response: { result?: unknown; error?: string } | undefined) => {
    if (chrome.runtime.lastError) {
      window.postMessage(
        { source: EXTENSION_MESSAGE_SOURCE, id, error: chrome.runtime.lastError.message ?? "Extension error." },
        window.location.origin
      );
      return;
    }
    if (response && "error" in response && response.error) {
      window.postMessage({ source: EXTENSION_MESSAGE_SOURCE, id, error: response.error }, window.location.origin);
    } else {
      window.postMessage({ source: EXTENSION_MESSAGE_SOURCE, id, result: response?.result }, window.location.origin);
    }
  });
});
