import { createRoot } from "react-dom/client";
import { useEffect, useState, type FormEvent } from "react";
import type { WalletBackgroundState } from "../lib/messages.ts";
import "./popup.css";

function sendMessage<T = WalletBackgroundState>(message: Record<string, unknown>): Promise<T> {
  return new Promise((resolve) => chrome.runtime.sendMessage(message, resolve));
}

function App() {
  const [state, setState] = useState<WalletBackgroundState | null>(null);
  const [passphrase, setPassphrase] = useState("");
  const [importHex, setImportHex] = useState("");
  const [mode, setMode] = useState<"generate" | "import">("generate");
  const [busy, setBusy] = useState(false);

  const refresh = () => {
    sendMessage<WalletBackgroundState>({ channel: "popup-get-state" }).then(setState);
  };

  useEffect(() => {
    refresh();
    // Picks up a pending request that arrives while the popup is already
    // open (e.g. the user reopened it manually just before a page called
    // signDeploy), and any decision made from elsewhere.
    const interval = setInterval(refresh, 1000);
    return () => clearInterval(interval);
  }, []);

  if (!state) {
    return (
      <main className="popup">
        <p className="muted">Loading...</p>
      </main>
    );
  }

  const runGenerate = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setBusy(true);
    await sendMessage({ channel: "popup-generate", passphrase });
    setPassphrase("");
    setBusy(false);
    refresh();
  };

  const runImport = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setBusy(true);
    await sendMessage({ channel: "popup-import", privateKeyHex: importHex, passphrase });
    setPassphrase("");
    setImportHex("");
    setBusy(false);
    refresh();
  };

  const runUnlock = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setBusy(true);
    await sendMessage({ channel: "popup-unlock", passphrase });
    setPassphrase("");
    setBusy(false);
    refresh();
  };

  const runLock = async () => {
    await sendMessage({ channel: "popup-lock" });
    refresh();
  };

  const runDisconnect = async (origin: string) => {
    await sendMessage({ channel: "popup-disconnect", origin });
    refresh();
  };

  const decide = async (approved: boolean) => {
    if (!state.pendingRequest) return;
    await sendMessage({ channel: "popup-decision", requestId: state.pendingRequest.id, approved });
    refresh();
  };

  if (state.status === "no-wallet") {
    return (
      <main className="popup">
        <h1>REV Wallet</h1>
        <p className="muted">Generate a key or import one. It never leaves this extension.</p>
        <form onSubmit={mode === "generate" ? runGenerate : runImport}>
          {mode === "import" ? (
            <label>
              <span>Private key (hex)</span>
              <input type="password" value={importHex} onChange={(event) => setImportHex(event.target.value)} />
            </label>
          ) : null}
          <label>
            <span>Passphrase (encrypts the key at rest)</span>
            <input type="password" value={passphrase} onChange={(event) => setPassphrase(event.target.value)} />
          </label>
          <button type="submit" disabled={busy || !passphrase || (mode === "import" && !importHex)}>
            {mode === "generate" ? "Generate wallet" : "Import key"}
          </button>
        </form>
        <button className="link" type="button" onClick={() => setMode(mode === "generate" ? "import" : "generate")}>
          {mode === "generate" ? "Import instead" : "Generate instead"}
        </button>
      </main>
    );
  }

  const pending = state.pendingRequest;

  // A signing request needs the key, so it needs the wallet unlocked first -
  // that's a UI-flow step, not a separate thing background.ts tracks.
  if (pending && pending.kind === "sign" && state.status === "locked") {
    return (
      <main className="popup">
        <h1>Unlock to continue</h1>
        <p className="origin">{pending.origin}</p>
        <p className="muted">This site is asking for a signature. Unlock to review it.</p>
        <form onSubmit={runUnlock}>
          <label>
            <span>Passphrase</span>
            <input type="password" value={passphrase} onChange={(event) => setPassphrase(event.target.value)} />
          </label>
          <button type="submit" disabled={busy || !passphrase}>
            Unlock
          </button>
        </form>
        {state.error ? <p className="error">{state.error}</p> : null}
      </main>
    );
  }

  if (pending) {
    return (
      <main className="popup">
        <h1>{pending.kind === "connect" ? "Connection request" : "Signature request"}</h1>
        <p className="origin">{pending.origin}</p>
        {pending.kind === "sign" ? (
          <>
            <label>
              <span>Term to sign</span>
              <textarea readOnly rows={6} value={pending.fields.term} />
            </label>
            <dl className="fields">
              <div>
                <dt>Phlo limit</dt>
                <dd>{pending.fields.phloLimit}</dd>
              </div>
              <div>
                <dt>Phlo price</dt>
                <dd>{pending.fields.phloPrice}</dd>
              </div>
              <div>
                <dt>Shard</dt>
                <dd>{pending.fields.shardId}</dd>
              </div>
            </dl>
          </>
        ) : (
          <p className="muted">
            Grants this site your address and the ability to request signatures - every signature still needs its
            own separate approval here.
          </p>
        )}
        <div className="actions">
          <button className="approve" type="button" onClick={() => decide(true)}>
            Approve
          </button>
          <button className="reject" type="button" onClick={() => decide(false)}>
            Reject
          </button>
        </div>
      </main>
    );
  }

  if (state.status === "locked") {
    return (
      <main className="popup">
        <h1>Wallet locked</h1>
        <form onSubmit={runUnlock}>
          <label>
            <span>Passphrase</span>
            <input type="password" value={passphrase} onChange={(event) => setPassphrase(event.target.value)} />
          </label>
          <button type="submit" disabled={busy || !passphrase}>
            Unlock
          </button>
        </form>
        {state.error ? <p className="error">{state.error}</p> : null}
      </main>
    );
  }

  return (
    <main className="popup">
      <h1>REV Wallet</h1>
      <p className="address">{state.account?.revAddress}</p>
      <p className="muted">{state.account?.actorDid}</p>
      <button type="button" onClick={runLock}>
        Lock
      </button>
      <h2>Connected sites</h2>
      {state.connections.length ? (
        <ul className="connections">
          {state.connections.map((origin) => (
            <li key={origin}>
              <span>{origin}</span>
              <button type="button" onClick={() => runDisconnect(origin)}>
                Disconnect
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">No sites connected yet.</p>
      )}
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
