#!/usr/bin/env python3
"""Negative controls for causal evidence, independent of a running node."""
import hashlib
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("trace", Path(__file__).resolve().parents[1] / "tools/consensus-causal-trace.py")
trace = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trace)


def block(n=1, parents=None):
    return {"blockHash": f"{n:064x}", "blockNumber": n, "sender": "04" + "ab" * 64,
            "seqNum": n, "preStateHash": "ab" * 32, "postStateHash": "cd" * 32,
            "justifications": parents or [],
            "bonds": [{"validator": "04" + "ab" * 64, "stake": 100}]}


class TraceTests(unittest.TestCase):
    def test_capture_transport_preserves_long_raw_preimages_and_rejects_corruption(self):
        data = (b'"encoded":"' + b"abcde01234" * 100000 + b'"')
        digest, encoded = trace.encode_capture(data)
        self.assertEqual(trace.decode_capture(digest, encoded), data)
        self.assertLess(len(encoded), len(data))
        with self.assertRaises(ValueError):
            trace.decode_capture("00" * 32, encoded)

    def test_repeated_observation_does_not_create_a_second_message(self):
        self.assertEqual(len(trace.index_blocks([block(), block()])), 1)

    def test_conflicting_observation_for_one_hash_is_rejected(self):
        changed = {**block(), "postStateHash": "ef" * 32}
        with self.assertRaises(ValueError):
            trace.index_blocks([block(), changed])

    def test_missing_validator_and_duplicate_justifications_are_rejected(self):
        for invalid in [{**block(), "sender": ""},
                        {**block(), "justifications": ["ab" * 32, "ab" * 32]}]:
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                trace.index_blocks([invalid])

    def test_duplicate_bonded_identity_never_manufactures_stake(self):
        bond = block()["bonds"][0]
        with self.assertRaises(ValueError):
            trace.unique_bonds([bond, bond])
        for stake in [-1, True, "100"]:
            with self.subTest(stake=stake), self.assertRaises(ValueError):
                trace.unique_bonds([{**bond, "stake": stake}])

    def test_missing_dependency_marks_the_inventory_incomplete(self):
        view = trace.summarize([block(2, [f"{1:064x}"])])
        self.assertFalse(view["complete_inventory"])
        self.assertEqual(view["missing_dependencies"], [f"{1:064x}"])

    def test_different_node_views_and_empty_capture_are_not_convergence(self):
        a, b = trace.summarize([block()]), trace.summarize([block(), block(2)])
        self.assertFalse(trace.compare({0: a, 1: b})["same_accepted_ids"])
        self.assertEqual(trace.compare({0: a, 1: b})["missing_by_node"]["0"], [f"{2:064x}"])
        self.assertFalse(trace.compare({})["same_accepted_ids"])

    def test_block_capture_pages_past_the_default_api_limit(self):
        calls = []
        def get(node, path):
            calls.append((node, path))
            start, end = map(int, path.rsplit("/", 2)[-2:])
            self.assertLessEqual(end - start, 50)
            return [block(n) for n in range(start, end + 1)]
        blocks = trace.read_bounded_blocks(get, 2, 121)
        self.assertEqual(len(blocks), 122)
        self.assertEqual(calls, [(2, "/api/blocks/0/49"),
                                 (2, "/api/blocks/50/99"),
                                 (2, "/api/blocks/100/121")])

    def test_raw_bytes_are_verified_and_a_changed_preimage_fails(self):
        encoded = b"test preimage"
        item = {**block(), "blockHash": hashlib.blake2b(encoded, digest_size=32).hexdigest()}
        raw = {"blockHash": item["blockHash"], "sender": item["sender"], "encoded": encoded.hex()}
        trace.verify_raw(item, raw)
        with self.assertRaises(ValueError):
            trace.verify_raw(item, {**raw, "encoded": (encoded + b"wrong").hex()})

    def test_attestation_inputs_are_preserved_without_inventing_sender_ids(self):
        line = "attestation at tip 12: withheld — nothing_to_finalize=false new_state_transition=true quorum_reachable=false cadence_due=false paced=true round=[04abcd12 04ef1234] blocked_attempts=0 escape=false"
        decision = trace.decisions(line)[0]
        self.assertEqual(decision["height"], 12)
        self.assertEqual(decision["decision"], "withheld")
        self.assertEqual(decision["inputs"]["quorum_reachable"], "false")
        self.assertEqual(decision["round_sender_prefixes"], ["04abcd12", "04ef1234"])


if __name__ == "__main__":
    unittest.main()
