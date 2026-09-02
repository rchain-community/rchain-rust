# REV Wallet browser extension

A Manifest V3 Chrome extension for signing RChain deploys with least
authority: the extension's background service worker is the only thing
that ever touches a private key. A page can request a signature and
nothing more, and every request needs its own explicit, informed approval
in the extension's popup - connecting a site at all, and each individual
`signDeploy` call.

This is the same model MetaMask/Phantom use for Ethereum/Solana, applied to
RChain. It's general-purpose: `signDeploy` signs whatever Rholang term it's
handed and has no dependency on any particular dApp's event schemas.

## What a page sees

Once a page's origin is listed in `manifest.json`'s `content_scripts.matches`
and the extension is installed, it gets a `window.revWallet` provider:

```js
const { revAddress, actorDid } = await window.revWallet.requestAccounts(); // prompts for connection approval the first time
const envelope = await window.revWallet.signDeploy({
  term, timestamp, phloPrice, phloLimit, validAfterBlockNumber, shardId
}); // prompts for a per-request signing approval, showing the term
```

No key material, storage access, or extension API is ever exposed to the
page - only these three async methods (`requestAccounts`, `getAddress`,
`signDeploy`).

## How it's built

- `src/background.ts` - the service worker; the only file with key access.
  Holds per-origin connection grants (`src/lib/connections.ts`,
  `chrome.storage.local`) and the encrypted wallet record
  (`src/wallet/walletStore.ts`, via `src/lib/chromeStorageAdapter.ts`). The
  transient *unlocked* key lives in `chrome.storage.session`
  (`src/lib/sessionKey.ts`) rather than a plain variable, since an MV3
  service worker is killed after ~30s idle and a plain variable wouldn't
  survive that - `chrome.storage.session` is memory-only and cleared on
  browser close, but does survive worker eviction within a session.
- `src/inject.ts` (MAIN world) / `src/content-script.ts` (ISOLATED world) -
  the provider-injection relay. A content script's isolated JS world can't
  define `window.revWallet` visibly to the page itself, so `inject.ts`
  defines the provider and talks to `content-script.ts` via
  `window.postMessage`; `content-script.ts` relays to the background via
  `chrome.runtime.sendMessage` and is the one place that reads the page's
  *real* origin (never trusted from the message payload).
- `src/popup/` - a small React UI: generate/import/lock/unlock, a
  "Connected sites" list with per-origin disconnect, and the
  connection/signing approval screens (the signing one shows the actual
  term text before Approve/Reject).
- `src/wallet/` - vendored copies of the secp256k1/blake2b signing and
  REV-address-derivation logic (ports of this repo's own
  `rholang/src/util/rev_address.rs` and the deploy-signing protobuf
  encoding), kept dependency-free from any particular frontend.

`content-script.js`/`inject.js` must be classic scripts, never ES modules
(only the background service worker and the popup page support
`"type": "module"`), and Rollup's IIFE output rejects multiple inputs in
one build - hence three separate Vite configs
(`vite.config.ts` for background+popup, `vite.config.content-script.ts`,
`vite.config.inject.ts`), chained in `npm run build`.

## Try it

```sh
npm install
npm run build      # writes dist/
npm run typecheck
```

Load `dist/` as an unpacked extension (`chrome://extensions` -> Developer
mode -> Load unpacked). Open `test-page.html` (served from an origin listed
in `manifest.json`'s `content_scripts.matches` - the defaults are common
local Vite dev ports on `127.0.0.1`/`localhost`) to exercise
`requestAccounts`/`getAddress`/`signDeploy` against a real devnet.

## Status

Standalone and verified (a real Playwright session covering generate ->
fund -> connect-with-approval -> sign-with-approval -> real on-chain
`ProcessedWithSuccess` -> disconnect). Not yet wired into any specific
dApp's frontend - that's expected to be a separate integration per
consumer, using the `window.revWallet` provider above.
