"""Offline regression tests for the read-only #420 incident probe."""

import datetime as dt
import importlib.util
import io
import json
from pathlib import Path
import socket
import unittest
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "tools" / "incident-snapshot.py"
spec = importlib.util.spec_from_file_location("incident_snapshot", SCRIPT)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class FakeResponse:
    status = 200

    def __init__(self, value):
        self.content = json.dumps(value).encode("utf8")

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def read(self, amount):
        return self.content[:amount]


class ProbeTests(unittest.TestCase):
    def test_url_validation_and_no_credentials(self):
        self.assertEqual(probe.parse_mapping(["A=http://localhost:40403"], "node"),
                         {"A": "http://localhost:40403"})
        for value in ("A=http://user:pw@localhost", "A=ftp://localhost",
                      "A=http://localhost/path", "A=https://localhost?x=1"):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    probe.parse_mapping([value], "node")
        with self.assertRaises(ValueError):
            probe.parse_mapping(["A=http://one", "A=http://two"], "node")

    def test_requires_health_path(self):
        with self.assertRaises(ValueError):
            probe.parse_mapping(["A=http://localhost:8080/status"], "health")

    def test_get_only_and_selects_fields(self):
        def response(req, timeout):
            self.assertEqual(req.get_method(), "GET")
            self.assertEqual(timeout, 1.0)
            return FakeResponse({"latestBlockNumber": 12, "ignore": "other"})
        with mock.patch.object(probe.urllib.request, "urlopen", side_effect=response):
            obj = probe.get_json("http://localhost/api/status", 1.0, probe.STATUS_FIELDS)
        self.assertTrue(obj["ok"])
        self.assertEqual(obj["data"], {"latestBlockNumber": 12})
        self.assertEqual(len(obj["sha256"]), 64)

    def test_timeout_is_failure_not_no_blocks(self):
        with mock.patch.object(probe.urllib.request, "urlopen",
                               side_effect=socket.timeout("timed out")):
            obj = probe.get_json("http://localhost/api/status", .1, probe.STATUS_FIELDS)
        self.assertFalse(obj["ok"])
        self.assertEqual(obj["error_type"], "TimeoutError")
        self.assertNotIn("data", obj)

    def test_height_spread_is_not_fork(self):
        now = dt.datetime(2026, 10, 23, 19, tzinfo=dt.timezone.utc)
        def fake(url, timeout, fields):
            if "one" in url:
                n = 12
            else:
                n = 10
            return {"url": url, "ok": True, "data": {
                ("blockNumber" if "last-finalized" in url else "latestBlockNumber"): n-2}}
        with mock.patch.object(probe, "get_json", side_effect=fake):
            data = probe.collect({"A": "http://one", "B": "http://two"}, {}, 1, now)
        # Synthetic A tip 10, B tip 8. The warning is observational, not a safety verdict.
        self.assertTrue(any("not evidence of a divergent finalized state" in w
                            for w in data["diagnostics"]["warnings"]))
        self.assertEqual(data["diagnostics"]["reachable_status"], 2)

    def test_stale_health_not_treated_as_live(self):
        now = dt.datetime(2026, 10, 23, 19, tzinfo=dt.timezone.utc)
        with mock.patch.object(probe, "get_json", return_value={
            "ok": True, "data": {"ts": now.timestamp()-180, "ok": True,
                                 "mem_available_mb": 1}}):
            data = probe.collect({}, {"A": "https://node/health"}, 1, now)
        self.assertTrue(any("older than 120" in x for x in data["diagnostics"]["warnings"]))
        self.assertEqual(data["diagnostics"]["reachable_status"], 0)

    def test_finalized_beyond_tip_is_warning(self):
        def fake(url, timeout, fields):
            key = "blockNumber" if "last-finalized" in url else "latestBlockNumber"
            return {"ok": True, "data": {key: 20 if key == "blockNumber" else 18}}
        with mock.patch.object(probe, "get_json", side_effect=fake):
            data = probe.collect({"A": "http://node"}, {}, 1)
        self.assertTrue(any("exceeds local tip" in w for w
                            in data["diagnostics"]["warnings"]))


if __name__ == "__main__":
    unittest.main()
