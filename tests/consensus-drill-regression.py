#!/usr/bin/env python3
"""The execution witness must reject malformed and wrong state evidence."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("drill", Path(__file__).resolve().parents[1] / "tools/consensus-acceptance-drill.py")
drill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drill)


class EvidenceTests(unittest.TestCase):
    def test_campaign_refuses_to_touch_a_non_ci_network(self):
        with patch.dict(drill.fuzz.os.environ, {"GITHUB_ACTIONS": "false"}), patch.object(drill.fuzz.subprocess, "run") as run:
            with self.assertRaises(SystemExit):
                drill.fuzz.run_acceptance_campaign()
            run.assert_not_called()

    def test_block_identity_is_validated(self):
        self.assertEqual(drill.block_identity({"blockNumber": 10, "blockHash": "ab" * 32}), (10, "ab" * 32))
        for value in [None, [], {}, {"blockNumber": -1, "blockHash": "ab" * 32},
                      {"blockNumber": True, "blockHash": "ab" * 32},
                      {"blockNumber": 10, "blockHash": "zz" * 32},
                      {"blockNumber": 10, "blockHash": "abc"}]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                drill.block_identity(value)

    def test_only_the_exact_marker_value_passes(self):
        self.assertTrue(drill.marker_present({"expr": [{"ExprInt": {"data": 42}}]}, 42))
        for value in [{"expr": []}, {"expr": [{"ExprInt": {"data": 41}}]},
                      {"expr": [{"ExprString": {"data": "42"}}]},
                      {"expr": [{"ExprInt": {"data": 42}}, {"ExprInt": {"data": 42}}]}]:
            self.assertFalse(drill.marker_present(value, 42))
        for value in [None, [], {}, {"expr": "42"}]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                drill.marker_present(value, 42)


if __name__ == "__main__":
    unittest.main()
