#!/usr/bin/env python3
"""Fail-closed acceptance evaluator regression tests. Fixtures are synthetic."""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "mainnet-acceptance.py"
SPEC = importlib.util.spec_from_file_location("mainnet_acceptance", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
CANDIDATE = "a" * 40


class AcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.manifest = {"schema_version": 1, "candidate_sha": CANDIDATE,
                         "gates": {g: [] for g in MODULE.CRITERIA}}
        self.count = 0

    def save(self):
        p = self.root / "manifest.json"
        p.write_text(json.dumps(self.manifest), encoding="utf-8")
        return p

    def add(self, gate, criterion, *, overrides=None, checks=None, file=None):
        self.count += 1
        path = file or f"proofs/proof-{self.count}.json"
        dest = self.root / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        witness = {
            "gate": gate, "criterion": criterion, "candidate_sha": CANDIDATE,
            "run_id": f"run-{self.count}", "command": "test-tool --isolated",
            "environment": "isolated-test-fixture", "started_at": "2026-10-24T00:00:00Z",
            "finished_at": "2026-10-24T00:01:00Z",
            "checks": checks if checks is not None
            else {c: True for c in MODULE.CRITERIA[gate][criterion]},
        }
        witness.update(overrides or {})
        raw = json.dumps(witness, sort_keys=True).encode()
        dest.write_bytes(raw)
        entry = {"criterion": criterion, "path": path, "sha256": hashlib.sha256(raw).hexdigest()}
        self.manifest["gates"][gate].append(entry)
        return dest, entry

    def status(self, gate, criterion):
        return MODULE.evaluate(self.save())["gates"][gate]["criteria"][criterion]["status"]

    def test_empty_manifest_blocks_all(self):
        r = MODULE.evaluate(self.save())
        self.assertEqual(set(r["gates"]), set(MODULE.CRITERIA))
        self.assertTrue(all(v["status"] == "BLOCKED" for v in r["gates"].values()))
        self.assertEqual(r["release_decision"], "BLOCKED")
        self.assertEqual(r["certification"], "NOT_CERTIFIED")

    def test_one_matching_digest_and_all_checks_can_pass(self):
        self.add("M1", "supported_capacity")
        self.assertEqual(self.status("M1", "supported_capacity"), "PASS")
        self.assertEqual(MODULE.evaluate(self.save())["release_decision"], "BLOCKED")

    def test_all_synthetic_fixtures_only_pending_signoff(self):
        for gate, criteria in MODULE.CRITERIA.items():
            for criterion in criteria:
                self.add(gate, criterion)
        report = MODULE.evaluate(self.save())
        self.assertTrue(all(x["status"] == "PASS" for x in report["gates"].values()))
        self.assertEqual(report["release_decision"], "PENDING_MAINTAINER_SIGNOFF")
        self.assertEqual(report["certification"], "NOT_CERTIFIED")

    def test_negative_check_is_fail(self):
        checks = {x: True for x in MODULE.CRITERIA["M1"]["supported_capacity"]}
        checks["no_unexpected_oom"] = False
        self.add("M1", "supported_capacity", checks=checks)
        self.assertEqual(self.status("M1", "supported_capacity"), "FAIL")
        self.assertEqual(MODULE.evaluate(self.save())["gates"]["M1"]["status"], "FAIL")

    def test_digest_tamper_blocks(self):
        dest, _ = self.add("M1", "supported_capacity")
        dest.write_text(dest.read_text() + " ")
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_candidate_mismatch_blocks(self):
        self.add("M1", "supported_capacity", overrides={"candidate_sha": "b" * 40})
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_missing_check_blocks(self):
        self.add("M1", "supported_capacity", checks={"no_unexpected_oom": True})
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_extra_check_blocks(self):
        c = {v: True for v in MODULE.CRITERIA["M1"]["supported_capacity"]}
        c["unreviewed"] = True
        self.add("M1", "supported_capacity", checks=c)
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_string_true_is_not_boolean(self):
        c = {v: True for v in MODULE.CRITERIA["M1"]["supported_capacity"]}
        c["no_unexpected_oom"] = "true"
        self.add("M1", "supported_capacity", checks=c)
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_duplicate_proof_blocks(self):
        _, entry = self.add("M1", "supported_capacity")
        self.manifest["gates"]["M1"].append(dict(entry))
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_path_traversal_blocks(self):
        _, entry = self.add("M1", "supported_capacity")
        entry["path"] = "../proof.json"
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_abs_path_blocks(self):
        _, entry = self.add("M1", "supported_capacity")
        entry["path"] = "/tmp/proof.json"
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_symlink_to_outside_blocks(self):
        dest, entry = self.add("M1", "supported_capacity")
        alias = self.root / "link.json"
        alias.symlink_to(dest)
        entry["path"] = "link.json"
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_wrong_criterion_blocks(self):
        self.add("M1", "supported_capacity", overrides={"criterion": "failure_domains"})
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_reversed_time_blocks(self):
        self.add("M1", "supported_capacity", overrides={"finished_at": "2026-10-23T23:00:00Z"})
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_missing_command_blocks(self):
        self.add("M1", "supported_capacity", overrides={"command": ""})
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_invalid_gate_rejected(self):
        self.manifest["gates"]["M7"] = []
        with self.assertRaises(MODULE.EvidenceError):
            MODULE.evaluate(self.save())

    def test_invalid_sha_rejected(self):
        self.manifest["candidate_sha"] = "0"
        with self.assertRaises(MODULE.EvidenceError):
            MODULE.evaluate(self.save())

    def test_duplicate_checklist_entries_cannot_satisfy_two_criteria(self):
        self.add("M1", "supported_capacity")
        _, entry = self.add("M1", "failure_domains")
        entry["path"] = "proofs/proof-1.json"
        self.assertEqual(self.status("M1", "failure_domains"), "BLOCKED")

    def test_duplicate_json_keys_in_witness_block(self):
        dest, entry = self.add("M1", "supported_capacity")
        raw = dest.read_text(encoding="utf-8")
        raw = raw.replace(
            '"run_id": "run-1"',
            '"run_id": "run-1", "run_id": "run-overridden"',
            1,
        )
        dest.write_text(raw, encoding="utf-8")
        entry["sha256"] = hashlib.sha256(dest.read_bytes()).hexdigest()
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_duplicate_json_keys_in_manifest_refused(self):
        path = self.save()
        raw = path.read_text(encoding="utf-8")
        raw = raw.replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1', 1)
        path.write_text(raw, encoding="utf-8")
        with self.assertRaises(MODULE.EvidenceError):
            MODULE.evaluate(path)

    def test_nonstandard_nan_in_evidence_blocks(self):
        dest, entry = self.add("M1", "supported_capacity")
        raw = dest.read_text(encoding="utf-8")
        raw = raw.replace('"environment": "isolated-test-fixture"',
                          '"environment": NaN', 1)
        dest.write_text(raw, encoding="utf-8")
        entry["sha256"] = hashlib.sha256(dest.read_bytes()).hexdigest()
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_boolean_manifest_schema_version_rejected(self):
        self.manifest["schema_version"] = True
        with self.assertRaises(MODULE.EvidenceError):
            MODULE.evaluate(self.save())

    def test_unknown_manifest_field_rejected(self):
        self.manifest["approved"] = True
        with self.assertRaises(MODULE.EvidenceError):
            MODULE.evaluate(self.save())

    def test_extra_evidence_payload_field_blocked(self):
        self.add("M1", "supported_capacity", overrides={"secrets": "do-not-embed"})
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_malformed_utc_blocks(self):
        for value in [
            "2026-10-24T00:00Z",
            "2026-10-24T00:00:00+00:00",
            "2026-10-24T25:00:00Z",
            "2026-10-24T00:00:00.123456789Z",
        ]:
            self.assertRaises(MODULE.EvidenceError, MODULE.valid_utc, value)

    def test_source_digest_is_exact_parsed_byte_sequence(self):
        dest, entry = self.add("M1", "supported_capacity")
        self.assertEqual(entry["sha256"], MODULE.load_json(dest)[1])
        self.assertEqual(self.status("M1", "supported_capacity"), "PASS")

    def test_empty_and_oversized_manifest_fail_closed(self):
        path = self.save()
        path.write_bytes(b"")
        with self.assertRaises(MODULE.EvidenceError):
            MODULE.evaluate(path)
        path.write_bytes(b" " * (MODULE.LIMIT + 1))
        with self.assertRaises(MODULE.EvidenceError):
            MODULE.evaluate(path)

    def test_manifest_symlink_rejected(self):
        real = self.save()
        link = self.root / "linked.json"
        link.symlink_to(real)
        with self.assertRaises(MODULE.EvidenceError):
            MODULE.evaluate(link)

    def test_raw_control_character_in_json_is_rejected(self):
        dest, entry = self.add("M1", "supported_capacity")
        raw = dest.read_text(encoding="utf-8")
        raw = raw.replace("test-tool --isolated", "test-tool --isolated\x00", 1)
        dest.write_text(raw, encoding="utf-8")
        entry["sha256"] = hashlib.sha256(dest.read_bytes()).hexdigest()
        self.assertEqual(self.status("M1", "supported_capacity"), "BLOCKED")

    def test_output_never_promotes_unsigned_claims_to_certification(self):
        for gate, criteria in MODULE.CRITERIA.items():
            for criterion in criteria:
                self.add(gate, criterion)
        result = MODULE.evaluate(self.save())
        self.assertEqual(result["certification"], "NOT_CERTIFIED")
        self.assertEqual(result["release_decision"], "PENDING_MAINTAINER_SIGNOFF")
        self.assertIn("NOT_CERTIFIED", MODULE.markdown(result))
        self.assertIn("not an authorization", MODULE.markdown(result))

    def test_markdown_never_claims_approval(self):
        self.add("M1", "supported_capacity")
        report = MODULE.evaluate(self.save())
        doc = MODULE.markdown(report)
        self.assertIn("NOT_CERTIFIED", doc)
        self.assertIn("not an authorization", doc)


if __name__ == "__main__":
    unittest.main()
