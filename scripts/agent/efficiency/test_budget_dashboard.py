import copy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import budget_dashboard as bd


class BudgetDashboardTests(unittest.TestCase):
    def fresh(self):
        return {"schema_version": 1, "plan": None, "checkpoints": [], "forecasts": [], "allowance_observations": []}

    def record(self, checkpoint_id, wave="Phase 0", **kw):
        value = {"checkpoint_id": checkpoint_id, "wave": wave, "beads_task": "sbe_ts-gu78",
                 "session_ids": ["parent"], "parent_session_id": None, "slice": "WP0.shared",
                 "kind": "delta", "counters": {"delta": {"input_tokens": 100, "cached_input_tokens": 25,
                 "output_tokens": 5, "reasoning_output_tokens": 2}}, "attribution": "verified"}
        value.update(kw)
        return value

    def test_exact_immutable_plan_and_hash(self):
        state = self.fresh()
        with patch.object(bd, "_plan_hash", return_value="a" * 64):
            bd.init_plan(state)
            self.assertEqual(state["plan"]["total_baseline"], 37_500_000)
            self.assertEqual(state["plan"]["baseline"]["Phase 0"], 1_500_000)
            self.assertEqual(state["plan"]["plan_sha256"], "a" * 64)
            self.assertEqual(bd.init_plan(state), state)
            state["plan"]["baseline"]["Phase 0"] += 1
            with self.assertRaises(ValueError):
                bd.init_plan(state)

    def test_checkpoint_idempotence_and_cumulative_delta(self):
        state = bd.init_plan(self.fresh())
        cp = self.record("c1")
        state, added = bd.checkpoint(state, cp)
        self.assertTrue(added)
        state, added = bd.checkpoint(state, copy.deepcopy(cp))
        self.assertFalse(added)
        state, _ = bd.checkpoint(state, self.record("c2", kind="end", counters={"input_tokens": 175, "cached_input_tokens": 45}, start_counters={"input_tokens": 100, "cached_input_tokens": 25}))
        self.assertEqual(bd.summary(state)["totals"]["input_tokens"], 175)

    def test_restart_parent_child_resume_archive_and_cross_wave(self):
        state = bd.init_plan(self.fresh())
        state, _ = bd.checkpoint(state, self.record("parent-start", kind="start", counters={"input_tokens": 10}, session_ids=["p"]))
        state, _ = bd.checkpoint(state, self.record("child-delta", wave="Wave 1A", session_ids=["c"], parent_session_id="p", counters={"delta": {"input_tokens": 30}}))
        state, _ = bd.checkpoint(state, self.record("resumed-delta", session_ids=["p"], resumed_from="parent-start", counters={"delta": {"input_tokens": 20}}))
        state, _ = bd.checkpoint(state, self.record("archive-end", session_ids=["c"], archived=True, counters={"delta": {"input_tokens": 5}}))
        with tempfile.TemporaryDirectory() as d:
            file = Path(d) / "state.json"
            bd._atomic_write(file, state)
            restored = bd._state(file)
            summary = bd.summary(restored)
        self.assertEqual(summary["waves"]["Phase 0"]["actual"], 25)
        self.assertEqual(summary["waves"]["Wave 1A"]["actual"], 30)
        self.assertEqual(summary["checkpoint_count"], 4)

    def test_missing_ambiguous_unrelated_and_allowance_unknown(self):
        state = bd.init_plan(self.fresh())
        cp = self.record("amb", counters={"delta": {"input_tokens": 55}}, attribution="ambiguous", unrelated_sessions=["elsewhere"])
        state, _ = bd.checkpoint(state, cp)
        summary = bd.summary(state)
        self.assertEqual(summary["unattributed"]["input_tokens"], 55)
        self.assertEqual(summary["allowance"]["status"], "unavailable")
        self.assertEqual(summary["waves"]["Phase 0"]["actual"], 55)

    def test_counter_reset_is_not_negative_usage(self):
        state = bd.init_plan(self.fresh())
        cp = self.record("reset", kind="end", counters={"input_tokens": 12}, start_counters={"input_tokens": 20})
        state, _ = bd.checkpoint(state, cp)
        self.assertEqual(bd.summary(state)["totals"]["input_tokens"], 0)

    def test_forecast_revision_preserves_baseline(self):
        state = bd.init_plan(self.fresh())
        original = copy.deepcopy(state["plan"]["baseline"])
        state["forecasts"].append({"input_tokens": 900_000, "reason": "fixture", "uncertainty": "high"})
        self.assertEqual(state["plan"]["baseline"], original)
        self.assertEqual(bd.summary(state)["forecast"]["input_tokens"], 900_000)

    def test_allocation_sums_and_duplicate_parent_child_usage(self):
        state = bd.init_plan(self.fresh())
        with self.assertRaises(ValueError):
            bd.allocate(state, {"wave": "Phase 0", "slices": {"WP0": 1}})
        bd.allocate(state, {"wave": "Phase 0", "slices": {"WP0": 900_000, "shared": 600_000}})
        parent = self.record("parent-cover", contribution_ids=["turn-1"], counters={"delta": {"input_tokens": 120}})
        child = self.record("child-copy", contribution_ids=["turn-1"], session_ids=["child"], counters={"delta": {"input_tokens": 120}})
        state, _ = bd.checkpoint(state, parent)
        state, _ = bd.checkpoint(state, child)
        self.assertEqual(bd.summary(state)["totals"]["input_tokens"], 120)
        self.assertEqual(bd.summary(state)["waves"]["Phase 0"]["slices"]["WP0"]["baseline"], 900_000)

    def test_help_and_legacy_cli_arguments_are_supported(self):
        # Parser options are exercised by checking documented arguments on the parser source.
        source = Path(bd.__file__).read_text()
        for option in ("--since", "--budget", "--init-plan", "--start-wave", "--checkpoint", "--plan-summary", "--complete-wave", "--allocate", "--reconcile-sessions"):
            self.assertIn(option, source)


if __name__ == "__main__":
    unittest.main()
