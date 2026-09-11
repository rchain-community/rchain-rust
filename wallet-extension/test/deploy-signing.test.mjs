import { test } from "node:test";
import assert from "node:assert/strict";
import { secp256k1 } from "@noble/curves/secp256k1.js";
import {
  encodeDeployDataProto,
  hashDeployData,
  signDeployHash,
  signDeploy,
  toDeployRequestBody
} from "../src/wallet/deploySigning.ts";
import { deriveUncompressedPublicKey } from "../src/wallet/revAddress.ts";
import { generatePrivateKey, importPrivateKey } from "../src/wallet/keys.ts";

const DEVNET_DEPLOYER_PRIVATE_KEY_HEX =
  "a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76";

function bytesToHex(bytes) {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

test("encodes fields in tag order, omitting zero/empty defaults", () => {
  const encoded = encodeDeployDataProto({
    term: "test",
    timestamp: 0,
    phloPrice: 1,
    phloLimit: 1000000,
    validAfterBlockNumber: -1,
    shardId: "root"
  });
  // Cross-checked independently against the protobuf varint/tag rules
  // directly (see Glidegraph's own test/wallet-deploy-signing.test.mjs,
  // which this vendored copy is a signing-compatible port of):
  // timestamp=0 is fully omitted (no tag byte at all), and
  // validAfterBlockNumber=-1 is a full 10-byte varint since int64 encodes
  // negatives as their 64-bit two's-complement value cast unsigned.
  assert.equal(
    bytesToHex(encoded),
    "120474657374380140c0843d50ffffffffffffffffff015a04726f6f74"
  );
});

test("validAfterBlockNumber: -1 alone encodes as the expected 10-byte varint tail", () => {
  const encoded = encodeDeployDataProto({
    term: "",
    timestamp: 0,
    phloPrice: 0,
    phloLimit: 0,
    validAfterBlockNumber: -1,
    shardId: ""
  });
  // tag(10, wiretype 0) = (10<<3)|0 = 0x50, then varint(0xFFFFFFFFFFFFFFFF).
  assert.equal(bytesToHex(encoded), "50ffffffffffffffffff01");
});

test("a positive validAfterBlockNumber (e.g. a real block height) round-trips as a short varint, not the negative form", () => {
  const encoded = encodeDeployDataProto({
    term: "",
    timestamp: 0,
    phloPrice: 0,
    phloLimit: 0,
    validAfterBlockNumber: 5,
    shardId: ""
  });
  assert.equal(bytesToHex(encoded), "5005");
});

test("all-default fields produce an empty encoding (every field omitted)", () => {
  const encoded = encodeDeployDataProto({
    term: "",
    timestamp: 0,
    phloPrice: 0,
    phloLimit: 0,
    validAfterBlockNumber: 0,
    shardId: ""
  });
  assert.equal(encoded.length, 0);
});

test("signDeployHash produces a signature that secp256k1.verify accepts against the real hash and pubkey", () => {
  const privateKey = importPrivateKey(DEVNET_DEPLOYER_PRIVATE_KEY_HEX);
  const publicKey = deriveUncompressedPublicKey(privateKey);
  const encoded = encodeDeployDataProto({
    term: "@\"glidegraph:events\"!((\"post.v1\",))",
    timestamp: 1735689600000,
    phloPrice: 1,
    phloLimit: 1000000,
    validAfterBlockNumber: -1,
    shardId: "root"
  });
  const hash = hashDeployData(encoded);
  assert.equal(hash.length, 32);
  const signature = signDeployHash(hash, privateKey);
  const isValid = secp256k1.verify(signature, hash, publicKey, { prehash: false, format: "der" });
  assert.equal(isValid, true);

  // A signature over the wrong hash must not verify.
  const wrongHash = hashDeployData(encodeDeployDataProto({ term: "different", timestamp: 0, phloPrice: 0, phloLimit: 0, validAfterBlockNumber: 0, shardId: "" }));
  assert.equal(secp256k1.verify(signature, wrongHash, publicKey, { prehash: false, format: "der" }), false);
});

test("signDeploy assembles a RholangDeployEnvelope with a verifiable signature and correct deployer", () => {
  const privateKey = generatePrivateKey();
  const publicKey = deriveUncompressedPublicKey(privateKey);
  const envelope = signDeploy(
    { term: "Nil", timestamp: 1735689600000, phloPrice: 1, phloLimit: 1000000, validAfterBlockNumber: -1, shardId: "root" },
    privateKey
  );
  assert.equal(envelope.deployer, bytesToHex(publicKey));
  assert.equal(envelope.sigAlgorithm, "secp256k1");

  const encoded = encodeDeployDataProto(envelope);
  const hash = hashDeployData(encoded);
  const signatureBytes = Uint8Array.from(Buffer.from(envelope.signature, "hex"));
  assert.equal(
    secp256k1.verify(signatureBytes, hash, publicKey, { prehash: false, format: "der" }),
    true
  );
});

test("toDeployRequestBody nests the signed data under `data`, deployer/signature/sigAlgorithm at top level", () => {
  const privateKey = generatePrivateKey();
  const envelope = signDeploy(
    { term: "Nil", timestamp: 42, phloPrice: 1, phloLimit: 1000000, validAfterBlockNumber: -1, shardId: "root" },
    privateKey
  );
  const body = toDeployRequestBody(envelope);
  assert.deepEqual(body.data, {
    term: "Nil",
    timestamp: 42,
    phloPrice: 1,
    phloLimit: 1000000,
    validAfterBlockNumber: -1,
    shardId: "root"
  });
  assert.equal(body.deployer, envelope.deployer);
  assert.equal(body.signature, envelope.signature);
  assert.equal(body.sigAlgorithm, "secp256k1");
});
