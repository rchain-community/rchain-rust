"""Offline/negative tests for Rholo #420 pinned Root-state evidence."""
import importlib.util
import json
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "tools" / "incident-root-evidence.py"
spec = importlib.util.spec_from_file_location("incident_root", SCRIPT)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)
A = "a" * 64
D = "d" * 64
MID = "c" * 64

def response(keys):
    return {"expr": [{"ExprTuple": {"data": [
        {"ExprBool": {"data": True}},
        {"ExprSet": {"data": [{"ExprString": {"data": k}} for k in keys]}}
    ]}}]}

def read(node, h, keys):
    return {"node": node, "blockHash": h,
            "request": {"blockHash": h, "usePreStateHash": False},
            "response": response(keys)}

def block(h, just, rejected=None):
    return {"blockHash": h, "response": {"blockInfo": {
        "blockHash": h, "justifications": just,
        "postStateHash": "deadbeef", "rejectedDeploys": rejected or []}}}

def example():
    return {"pair": {"ancestor": A, "descendant": D}, "nodes": ["B", "C"],
            "key": "component:governor",
            "reads": [read(node, h, keys) for node in ["B", "C"]
                      for h, keys in [(A, ["component:governor"]), (D, [])]],
            "blocks": [block(A, []), block(D, [A])]}

def check(doc):
    return probe.classify(json.dumps(doc).encode(), doc)

class RootEvidenceTests(unittest.TestCase):
    def test_declared_discontinuity_is_not_finalized_loss(self):
        out = check(example())
        self.assertTrue(out["reported_discontinuity_reproduced_in_supplied_json"])
        self.assertEqual(out["ancestry_from_supplied_headers"], "VERIFIED")
        self.assertFalse(out["finalized_effect_loss_proven"])
        self.assertFalse(out["kernel_oom_causation_proven"])

    def test_no_key_loss(self):
        doc = example()
        for r in doc["reads"]:
            r["response"] = response(["component:governor"])
        self.assertFalse(check(doc)["reported_discontinuity_reproduced_in_supplied_json"])

    def test_node_disagreement_not_inferred_as_chain_loss(self):
        doc = example()
        doc["reads"][1]["response"] = response(["component:governor"])
        self.assertEqual(check(doc)["cross_validator_checks"]["descendant"]["status"],
                         "CROSS_NODE_READ_MISMATCH")
        self.assertFalse(check(doc)["reported_discontinuity_reproduced_in_supplied_json"])

    def test_missing_read_not_empty_keys(self):
        doc = example()
        doc["reads"][0]["response"] = {}
        self.assertEqual(check(doc)["cross_validator_checks"]["ancestor"]["status"],
                         "MISSING_VALID_READ")
        self.assertEqual(len(check(doc)["read_errors"]), 1)

    def test_false_reply_not_empty(self):
        doc = example()
        doc["reads"][1]["response"]["expr"][0]["ExprTuple"]["data"][0]["ExprBool"]["data"] = False
        self.assertFalse(check(doc)["reported_discontinuity_reproduced_in_supplied_json"])

    def test_wrong_poststate_selector_rejected(self):
        doc = example()
        doc["reads"][0]["request"]["usePreStateHash"] = True
        self.assertEqual(len(check(doc)["read_errors"]), 1)

    def test_ancestry_incomplete_is_not_false(self):
        doc = example()
        doc["blocks"] = [block(D, [MID])]
        self.assertEqual(check(doc)["ancestry_from_supplied_headers"], "INCOMPLETE_HEADERS")

    def test_transitive_ancestry_can_be_verified(self):
        doc = example()
        doc["blocks"] = [block(A, []), block(D, [MID]), block(MID, [A])]
        self.assertEqual(check(doc)["ancestry_from_supplied_headers"], "VERIFIED")

    def test_header_hash_mismatch_detected(self):
        doc = example()
        doc["blocks"][1]["response"]["blockInfo"]["blockHash"] = MID
        out = check(doc)
        self.assertEqual(len(out["block_errors"]), 1)
        self.assertEqual(out["ancestry_from_supplied_headers"], "INCOMPLETE_HEADERS")

    def test_duplicate_node_hash_read_not_silently_overwritten(self):
        doc = example()
        doc["reads"].append(doc["reads"][0].copy())
        result = check(doc)
        self.assertEqual(len(result["read_errors"]), 1)
        self.assertEqual(result["cross_validator_checks"]["ancestor"]["status"],
                         "MISSING_VALID_READ")

    def test_invalid_pair_fails_closed(self):
        doc = example()
        doc["pair"]["ancestor"] = "not-full-hash"
        with self.assertRaises(probe.WitnessError):
            check(doc)

    def test_empty_set_is_valid_distinct_from_missing(self):
        self.assertEqual(probe.extract_keys(response([])), [])
        with self.assertRaises(probe.WitnessError):
            probe.extract_keys({"expr": []})

    def test_no_causal_claim_from_rejected_deploys(self):
        doc = example()
        doc["blocks"][1]["response"]["blockInfo"]["rejectedDeploys"] = ["deploy-id"]
        out = check(doc)
        self.assertEqual(out["descendant_rejected_deploys"], ["deploy-id"])
        self.assertFalse(out["finalized_effect_loss_proven"])

if __name__ == "__main__":
    unittest.main()
