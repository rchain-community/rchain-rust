import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

MODULE = Path(__file__).resolve().parents[1] / "tools" / "incident-oom-evidence.py"
spec = importlib.util.spec_from_file_location("incident_oom_evidence", MODULE)
oom = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oom)

EXAMPLE = b"""2026-10-23T18:37:41+0000 host kernel: oom-kill:constraint=CONSTRAINT_NONE,task_memcg=/system.slice/rnode.service,task=rnode,pid=999,uid=0
2026-10-23T18:37:41+0000 host kernel: Out of memory: Killed process 999 (rnode) total-vm:20
2026-10-23T18:46:21+0000 host kernel: oom-kill:task_memcg=/system.slice/rnode-c.service,task=rnode
2026-10-23T18:46:21+0000 host kernel: Out of memory: Killed process 999 (rnode) total-vm:20
"""


class KernelOomTests(unittest.TestCase):
    def test_operator_supplied_example_matches_two_services(self):
        data = oom.summarize(EXAMPLE, "HostBC")
        self.assertEqual(data["status"], "KERNEL_OOM_KILLS_PRESENT")
        self.assertEqual(len(data["kernel_oom_kill_events"]), 2)
        self.assertEqual([x["service"] for x in data["kernel_oom_kill_events"]],
                         ["rnode.service", "rnode-c.service"])
        self.assertFalse(data["casper_finality_proven"])
        self.assertFalse(data["host_source_authenticated"])

    def test_empty_window_is_not_proof_no_oom_occurred(self):
        data = oom.summarize(b"2026-10-23T18:00:00+0000 host kernel: normal\n", "HostBC")
        self.assertEqual(data["status"], "NO_MATCHING_EVENTS_IN_SUPPLIED_WINDOW")
        self.assertFalse(data["log_window_complete"])

    def test_unmatched_oom_signal_does_not_prove_kill(self):
        raw = EXAMPLE.splitlines()[0] + b"\n"
        self.assertFalse(oom.summarize(raw, "HostBC")["kernel_oom_kill_events"])

    def test_unmatched_kill_without_unit_does_not_attribute_which_validator(self):
        raw = EXAMPLE.splitlines()[1] + b"\n"
        self.assertFalse(oom.summarize(raw, "HostBC")["kernel_oom_kill_events"])

    def test_non_node_oom_not_misattributed(self):
        raw = EXAMPLE.replace(b"rnode.service", b"nginx.service")
        data = oom.summarize(raw, "HostBC")
        self.assertEqual(len(data["kernel_oom_kill_events"]), 1)

    def test_a_d_host_secondary_rnode_d_unit(self):
        example = (
            b"2026-10-23T19:09:10+0000 host kernel: "
            b"oom-kill:task_memcg=/system.slice/rnode.service,task=rnode,pid=101\n"
            b"2026-10-23T19:09:10+0000 host kernel: "
            b"Out of memory: Killed process 101 (rnode) total-vm:20\n"
            b"2026-10-23T22:54:26+0000 host kernel: "
            b"oom-kill:task_memcg=/system.slice/rnode-d.service,task=rnode,pid=202\n"
            b"2026-10-23T22:54:26+0000 host kernel: "
            b"Out of memory: Killed process 202 (rnode) total-vm:20\n"
        )
        result = oom.summarize(example, "HostAD")
        self.assertEqual(
            [x["service"] for x in result["kernel_oom_kill_events"]],
            ["rnode.service", "rnode-d.service"],
        )
        self.assertTrue(all(x["match_basis"] == "PID_AND_TIMESTAMP"
                            for x in result["kernel_oom_kill_events"]))
        self.assertNotIn("\"pid\"", json.dumps(result))
        self.assertFalse(result["host_source_authenticated"])

    def test_pid_mismatch_is_not_correlated_by_nearby_timestamp(self):
        raw = (
            b"2026-10-23T18:00:00Z host kernel: "
            b"oom-kill:task_memcg=/system.slice/rnode-d.service,task=rnode,pid=200\n"
            b"2026-10-23T18:00:00Z host kernel: "
            b"Out of memory: Killed process 201 (rnode) total-vm:20\n"
        )
        self.assertEqual(oom.summarize(raw, "HostAD")["kernel_oom_kill_events"], [])

    def test_single_kill_cannot_be_claimed_by_two_validator_units(self):
        raw = (
            b"2026-10-23T18:00:00Z host kernel: "
            b"oom-kill:task_memcg=/system.slice/rnode.service,task=rnode\n"
            b"2026-10-23T18:00:00Z host kernel: "
            b"oom-kill:task_memcg=/system.slice/rnode-d.service,task=rnode\n"
            b"2026-10-23T18:00:00Z host kernel: "
            b"Out of memory: Killed process 201 (rnode) total-vm:20\n"
        )
        packet = oom.summarize(raw, "HostAD")
        self.assertEqual(packet["kernel_oom_kill_events"], [])
        self.assertEqual(packet["status"], "AMBIGUOUS_OOM_PAIRING_IN_SUPPLIED_WINDOW")
        self.assertEqual(packet["ambiguous_pairing_signals"], 2)

    def test_two_nearby_victims_without_pid_in_signal_are_ambiguous(self):
        raw = (
            b"2026-10-23T18:00:00Z host kernel: "
            b"oom-kill:task_memcg=/system.slice/rnode-d.service,task=rnode\n"
            b"2026-10-23T18:00:00Z host kernel: "
            b"Out of memory: Killed process 201 (rnode) total-vm:20\n"
            b"2026-10-23T18:00:01Z host kernel: "
            b"Out of memory: Killed process 202 (rnode) total-vm:20\n"
        )
        packet = oom.summarize(raw, "HostAD")
        self.assertEqual(packet["kernel_oom_kill_events"], [])
        self.assertEqual(packet["status"], "AMBIGUOUS_OOM_PAIRING_IN_SUPPLIED_WINDOW")
        self.assertEqual(packet["ambiguous_pairing_signals"], 1)

    def test_pid_disambiguates_two_nearby_victims(self):
        raw = (
            b"2026-10-23T18:00:00Z host kernel: "
            b"oom-kill:task_memcg=/system.slice/rnode-d.service,task=rnode,pid=202\n"
            b"2026-10-23T18:00:00Z host kernel: "
            b"Out of memory: Killed process 201 (rnode) total-vm:20\n"
            b"2026-10-23T18:00:01Z host kernel: "
            b"Out of memory: Killed process 202 (rnode) total-vm:20\n"
        )
        events = oom.summarize(raw, "HostAD")["kernel_oom_kill_events"]
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["service"], "rnode-d.service")
        self.assertEqual(events[0]["match_basis"], "PID_AND_TIMESTAMP")

    def test_unambiguous_timestamp_pair_when_kernel_omits_task_pid(self):
        raw = (
            b"2026-10-23T18:00:00Z host kernel: "
            b"oom-kill:task_memcg=/system.slice/rnode-d.service,task=rnode\n"
            b"2026-10-23T18:00:01Z host kernel: "
            b"Out of memory: Killed process 201 (rnode) total-vm:20\n"
        )
        events = oom.summarize(raw, "HostAD")["kernel_oom_kill_events"]
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["match_basis"], "UNAMBIGUOUS_TIMESTAMP")

    def test_source_digest_exact_byte_binding(self):
        digest = hashlib.sha256(EXAMPLE).hexdigest()
        data = oom.summarize(EXAMPLE, "HostBC", digest)
        self.assertTrue(data["external_digest_match"])
        self.assertEqual(data["raw_sha256"], digest)
        with self.assertRaises(oom.EvidenceError):
            oom.summarize(EXAMPLE, "HostBC", "f" * 64)

    def test_reject_unsafe_host_labels(self):
        with self.assertRaises(oom.EvidenceError):
            oom.summarize(EXAMPLE, "../server/private")

    def test_utf8_requirement(self):
        with self.assertRaises(oom.EvidenceError):
            oom.summarize(b"\xff", "HostBC")

    def test_direct_summary_refuses_oversized_payload(self):
        with self.assertRaises(oom.EvidenceError):
            oom.summarize(b"x" * (oom.MAX_BYTES + 1), "HostAD")

    def test_bounded_source_read(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "journal.txt"
            path.write_bytes(b"x" * (oom.MAX_BYTES + 1))
            with self.assertRaises(oom.EvidenceError):
                oom.source_bytes(str(path))

    def test_cli_exclusive_create(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "journal.txt"
            out = Path(folder) / "summary.json"
            source.write_bytes(EXAMPLE)
            self.assertEqual(oom.main(["--journal", str(source),
                                      "--host-label", "HostBC",
                                      "--output", str(out)]), 0)
            data = json.loads(out.read_text())
            self.assertEqual(data["status"], "KERNEL_OOM_KILLS_PRESENT")
            self.assertEqual(oom.main(["--journal", str(source),
                                      "--host-label", "HostBC",
                                      "--output", str(out)]), 2)


if __name__ == "__main__":
    unittest.main()
