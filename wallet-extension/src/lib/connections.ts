const STORAGE_KEY = "rev-wallet:connections:v1";

interface ConnectionsRecord {
  [origin: string]: { connectedAt: string };
}

/**
 * Per-origin connection grants: the "minimum authority from the start"
 * control surface. An origin has zero access (getAddress/signDeploy both
 * reject) until explicitly connected via a "connect" approval in the popup,
 * and can be disconnected at any time - see the popup's "Connected origins"
 * screen. Storage is injectable (defaults to chrome.storage.local) so this
 * is testable with a plain in-memory fake, the same DI pattern
 * src/wallet/walletStore.ts's WalletStorageAdapter already uses.
 */
async function readRecord(area: chrome.storage.StorageArea): Promise<ConnectionsRecord> {
  const result = await area.get(STORAGE_KEY);
  const raw = result[STORAGE_KEY];
  return raw && typeof raw === "object" ? (raw as ConnectionsRecord) : {};
}

async function writeRecord(record: ConnectionsRecord, area: chrome.storage.StorageArea): Promise<void> {
  await area.set({ [STORAGE_KEY]: record });
}

export async function isConnected(
  origin: string,
  area: chrome.storage.StorageArea = chrome.storage.local
): Promise<boolean> {
  const record = await readRecord(area);
  return Object.prototype.hasOwnProperty.call(record, origin);
}

export async function connect(origin: string, area: chrome.storage.StorageArea = chrome.storage.local): Promise<void> {
  const record = await readRecord(area);
  record[origin] = { connectedAt: new Date().toISOString() };
  await writeRecord(record, area);
}

export async function disconnect(origin: string, area: chrome.storage.StorageArea = chrome.storage.local): Promise<void> {
  const record = await readRecord(area);
  delete record[origin];
  await writeRecord(record, area);
}

export async function listConnections(area: chrome.storage.StorageArea = chrome.storage.local): Promise<string[]> {
  const record = await readRecord(area);
  return Object.keys(record).sort();
}
