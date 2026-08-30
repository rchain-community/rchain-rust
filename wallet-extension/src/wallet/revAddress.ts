import { secp256k1 } from "@noble/curves/secp256k1.js";
import { keccak_256 } from "@noble/hashes/sha3.js";
import { blake2b } from "@noble/hashes/blake2.js";
import { base58 } from "@scure/base";

const ACTOR_DID_PREFIX = "did:rchain:";
const REV_ADDRESS_PREFIX = new Uint8Array([0x00, 0x00, 0x00, 0x00]);

function bytesToLowercaseHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function concatBytes(...chunks: Uint8Array[]): Uint8Array {
  const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

export function deriveUncompressedPublicKey(privateKey: Uint8Array): Uint8Array {
  return secp256k1.getPublicKey(privateKey, false);
}

/**
 * Port of this repo's rholang/src/util/rev_address.rs: eth-style address
 * from the raw (uncompressed, prefix stripped) public key, then a second
 * hash + blake2b checksum, base58-encoded.
 */
export function deriveRevAddress(uncompressedPublicKey: Uint8Array): string {
  if (uncompressedPublicKey.length !== 65 || uncompressedPublicKey[0] !== 0x04) {
    throw new Error("Expected a 65-byte uncompressed public key with a 0x04 prefix.");
  }
  const rawKey = uncompressedPublicKey.subarray(1);
  const ethAddress = keccak_256(rawKey).subarray(-20);
  const keyHash = keccak_256(ethAddress);
  const checksum = blake2b(concatBytes(REV_ADDRESS_PREFIX, keyHash), { dkLen: 32 }).subarray(0, 4);
  return base58.encode(concatBytes(REV_ADDRESS_PREFIX, keyHash, checksum));
}

export function actorDidFromPublicKey(uncompressedPublicKey: Uint8Array): string {
  return `${ACTOR_DID_PREFIX}${bytesToLowercaseHex(uncompressedPublicKey)}`;
}

export { bytesToLowercaseHex };
