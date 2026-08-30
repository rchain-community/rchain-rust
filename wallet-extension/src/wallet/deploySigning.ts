import { secp256k1 } from "@noble/curves/secp256k1.js";
import { blake2b } from "@noble/hashes/blake2.js";
import { deriveUncompressedPublicKey, bytesToLowercaseHex } from "./revAddress.ts";

export interface DeployDataFields {
  term: string;
  timestamp: number;
  phloPrice: number;
  phloLimit: number;
  validAfterBlockNumber: number;
  shardId: string;
}

/** Vendored copy of Glidegraph's src/contracts.ts RholangDeployEnvelope -
 * this extension has no dependency on that (or any) dApp's codebase, so the
 * one type it needs from there is inlined here instead of imported. */
export interface RholangDeployEnvelope extends DeployDataFields {
  deployer: string;
  signature: string;
  sigAlgorithm: "secp256k1" | "ed25519";
}

export interface SignedDeployRequestBody {
  data: DeployDataFields;
  deployer: string;
  signature: string;
  sigAlgorithm: "secp256k1";
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

/** Standard base-128 protobuf varint. `value` must already be non-negative
 * (encode signed fields via two's-complement first, see encodeInt64Field). */
function varint(value: bigint): Uint8Array {
  if (value < 0n) {
    throw new Error("varint requires a non-negative bigint");
  }
  const bytes: number[] = [];
  let remaining = value;
  for (;;) {
    const byte = Number(remaining & 0x7fn);
    remaining >>= 7n;
    if (remaining === 0n) {
      bytes.push(byte);
      break;
    }
    bytes.push(byte | 0x80);
  }
  return Uint8Array.from(bytes);
}

function fieldTag(fieldNumber: number, wireType: number): Uint8Array {
  return varint(BigInt((fieldNumber << 3) | wireType));
}

/** prost (the RChain Rust protobuf codec) omits fields at their default
 * value entirely rather than encoding a zero-length/zero-value field, so an
 * empty string must produce zero bytes here, not a zero-length string field. */
function encodeStringField(fieldNumber: number, value: string): Uint8Array {
  if (value === "") {
    return new Uint8Array(0);
  }
  const bytes = new TextEncoder().encode(value);
  return concatBytes(fieldTag(fieldNumber, 2), varint(BigInt(bytes.length)), bytes);
}

/**
 * protobuf int64 (not sint64) encodes a negative value as the varint of its
 * 64-bit two's-complement representation cast to unsigned - always a full
 * 10-byte varint, not a short encoding of the magnitude. Getting this wrong
 * for validAfterBlockNumber = -1 is the most likely source of a "signature
 * invalid" bug.
 */
function encodeInt64Field(fieldNumber: number, value: number): Uint8Array {
  if (value === 0) {
    return new Uint8Array(0);
  }
  if (!Number.isInteger(value)) {
    throw new Error(`int64 field ${fieldNumber} must be an integer, got ${value}`);
  }
  const unsigned64 = BigInt.asUintN(64, BigInt(value));
  return concatBytes(fieldTag(fieldNumber, 0), varint(unsigned64));
}

/**
 * Serializes DeployDataProto with only the fields that are part of the
 * signed bytes (deployer/sig/sigAlgorithm are excluded), in tag order.
 */
export function encodeDeployDataProto(fields: DeployDataFields): Uint8Array {
  return concatBytes(
    encodeStringField(2, fields.term),
    encodeInt64Field(3, fields.timestamp),
    encodeInt64Field(7, fields.phloPrice),
    encodeInt64Field(8, fields.phloLimit),
    encodeInt64Field(10, fields.validAfterBlockNumber),
    encodeStringField(11, fields.shardId)
  );
}

export function hashDeployData(serializedBytes: Uint8Array): Uint8Array {
  return blake2b(serializedBytes, { dkLen: 32 });
}

/** Signs an already-computed blake2b256 hash directly - `prehash: false` is
 * required, since @noble/curves' secp256k1.sign() otherwise sha256-hashes
 * its input by default. `format: "der"` + the default `lowS: true` match
 * what SignedDeployData::verify_signature expects. */
export function signDeployHash(hash: Uint8Array, privateKey: Uint8Array): Uint8Array {
  return secp256k1.sign(hash, privateKey, { prehash: false, format: "der", lowS: true });
}

/** Builds a fully signed RholangDeployEnvelope from deploy fields and a raw
 * private key. Does not touch the network or storage. */
export function signDeploy(fields: DeployDataFields, privateKey: Uint8Array): RholangDeployEnvelope {
  const serialized = encodeDeployDataProto(fields);
  const hash = hashDeployData(serialized);
  const signature = signDeployHash(hash, privateKey);
  const publicKey = deriveUncompressedPublicKey(privateKey);
  return {
    ...fields,
    deployer: bytesToLowercaseHex(publicKey),
    signature: bytesToLowercaseHex(signature),
    sigAlgorithm: "secp256k1"
  };
}

/** Maps the flat RholangDeployEnvelope to the nested JSON body
 * POST /api/v1/deploy expects. */
export function toDeployRequestBody(envelope: RholangDeployEnvelope): SignedDeployRequestBody {
  return {
    data: {
      term: envelope.term,
      timestamp: envelope.timestamp,
      phloPrice: envelope.phloPrice,
      phloLimit: envelope.phloLimit,
      validAfterBlockNumber: envelope.validAfterBlockNumber,
      shardId: envelope.shardId
    },
    deployer: envelope.deployer,
    signature: envelope.signature,
    sigAlgorithm: "secp256k1"
  };
}
