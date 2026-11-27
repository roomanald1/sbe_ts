import unittest
import copy
import json
from types import SimpleNamespace
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from financial import (
    compare_paper_exchange_match_evidence,
    MAX_RECONCILIATION_SOURCE_BYTES,
    reconcile_independent_execution_fills,
    reconcile_independent_portfolio_intervals,
    load_execution_reconciliation_inputs,
    project_paper_exchange_financial_evidence_page,
    reconciliation,
    reconstruct_portfolio_intervals_from_paper_exchange,
    validate_pm_durable_ledger_epoch,
)
from financial_snapshot import collect_paged_source, valid_financial_snapshot_body


class FinancialSnapshotContractTests(unittest.TestCase):
    def pm_epoch_coverage(self, **overrides):
        return {
            "ledger_evidence_epoch": "pm-ledger-evidence:v1:allocator-epoch-1",
            "ledger_evidence_epoch_status": "clean_slate",
            "trade_rows": 2,
            "fills_in_evidence_epoch": 2,
            "rows_without_fill_identity": 0,
            "rows_with_fill_identity_outside_evidence_epoch": 0,
            **overrides,
        }

    def test_pm_durable_epoch_accepts_clean_slate_with_matching_fill_ids(self):
        rows = [
            {"fill_id": "pm-fill:v1:allocator-epoch-1:1"},
            {"fill_id": "pm-fill:v1:allocator-epoch-1:2"},
        ]
        result = validate_pm_durable_ledger_epoch(self.pm_epoch_coverage(), rows)
        self.assertEqual(result["status"], "VALID", result)
        self.assertTrue(result["durable_epoch_usable"])
        self.assertEqual(result["fill_ids_checked"], 2)

    def test_pm_durable_epoch_rejects_missing_epoch_or_status(self):
        rows = [{"fill_id": "pm-fill:v1:allocator-epoch-1:1"},
                {"fill_id": "pm-fill:v1:allocator-epoch-1:2"}]
        missing_epoch = validate_pm_durable_ledger_epoch(
            self.pm_epoch_coverage(ledger_evidence_epoch=None), rows
        )
        self.assertFalse(missing_epoch["durable_epoch_usable"])
        self.assertIn(
            "Position Management durable ledger epoch is missing or malformed",
            missing_epoch["blockers"],
        )

        missing_status = validate_pm_durable_ledger_epoch(
            self.pm_epoch_coverage(ledger_evidence_epoch_status=None), rows
        )
        self.assertFalse(missing_status["durable_epoch_usable"])
        self.assertIn(
            "Position Management durable ledger epoch status is missing or unrecognized",
            missing_status["blockers"],
        )

    def test_pm_durable_epoch_rejects_wrong_version_and_fill_id_epoch_mismatch(self):
        rows = [
            {"fill_id": "pm-fill:v1:allocator-epoch-1:1"},
            {"fill_id": "pm-fill:v1:allocator-epoch-1:2"},
        ]
        wrong_version = validate_pm_durable_ledger_epoch(
            self.pm_epoch_coverage(
                ledger_evidence_epoch="pm-ledger-evidence:v2:allocator-epoch-1"
            ),
            rows,
        )
        self.assertFalse(wrong_version["durable_epoch_usable"])
        self.assertIn(
            "Position Management durable ledger epoch is missing or malformed",
            wrong_version["blockers"],
        )

        mismatch = validate_pm_durable_ledger_epoch(
            self.pm_epoch_coverage(),
            [
                {"fill_id": "pm-fill:v1:allocator-epoch-1:1"},
                {"fill_id": "pm-fill:v1:other-allocator-epoch:2"},
            ],
        )
        self.assertFalse(mismatch["durable_epoch_usable"])
        self.assertIn(
            "Position Management fill IDs do not match the advertised allocator epoch",
            mismatch["blockers"],
        )

    def test_pm_durable_epoch_keeps_mixed_history_explicitly_blocked(self):
        mixed = validate_pm_durable_ledger_epoch(
            self.pm_epoch_coverage(ledger_evidence_epoch_status="mixed_legacy_and_current"),
            [
                {"fill_id": "pm-fill:v1:allocator-epoch-1:1"},
                {"fill_id": "pm-fill:v1:allocator-epoch-1:2"},
            ],
        )
        self.assertFalse(mixed["durable_epoch_usable"])
        self.assertIn(
            "Position Management fill history is mixed across durable epochs",
            mixed["blockers"],
        )

    def test_reconciliation_uses_verified_pm_epoch_instead_of_unconditional_blocker(self):
        rows = [
            {"fill_id": "pm-fill:v1:allocator-epoch-1:1"},
            {"fill_id": "pm-fill:v1:allocator-epoch-1:2"},
        ]
        manifest = {
            "snapshot_id": "pm-paper-v1:fixture",
            "execution_mode": "paper",
            "ledger_snapshot_consistent": True,
            "ledger_coverage": {
                **self.pm_epoch_coverage(),
                "unknown_fee_fills": 0,
                "fee_currency_unknown_rows": 0,
                "fee_cashflow_currency_mismatch_rows": 0,
                "rows_without_parent_order_id": 0,
                "timestamp_order_inversion_rows": 0,
            },
            "execution_reconciliation_inputs": {
                "position_trades": rows,
                "execution_ledger": None,
                "position_source_coverage": None,
                "match_evidence_ledger": None,
                "pm_portfolio_intervals": None,
                "independent_portfolio_interval_contract": None,
                "portfolio_interval_source_coverage": None,
            },
        }
        timestamp = "2026-01-01T00:00:00+00:00"
        source_data = {
            "account": {"realized_pnl": 0.0, "currency": "USD"},
            "positions": {"trade_count": 2},
            "realized-pnl": {"realized_pnl_all_time_usd": 0.0},
            "fee-summary": {"fills": 2},
        }

        with patch("financial.SNAPSHOT_MANIFEST", manifest), patch(
            "financial.source",
            side_effect=lambda endpoint: (
                source_data[endpoint],
                {"collected_at": timestamp, "modified_at": timestamp},
            ),
        ):
            result = reconciliation()

        self.assertTrue(
            result["position_management_epoch_validation"]["durable_epoch_usable"]
        )
        self.assertNotIn(
            "Durable ledger epoch across restarts is not exposed", result["blockers"]
        )
        self.assertNotIn(
            "Position Management durable ledger epoch is missing or malformed",
            result["blockers"],
        )

    def test_financial_evidence_bundle_above_32mb_remains_reconcilable(self):
        """The multi-page collector's valid 64 MB bundle must survive loading."""
        self.assertGreater(MAX_RECONCILIATION_SOURCE_BYTES, 32_018_923)
        with TemporaryDirectory() as temporary:
            cache = Path(temporary).resolve()
            epoch = "execution-epoch-1"
            fee_schedule = {
                "schema_version": "paper_exchange_financial_evidence:v1",
                "event_id": "fee-schedule:v1:BTC-USDT",
                "event_type": "fee_schedule",
                "source_system": "paper_exchange_simulator",
                "source_epoch": epoch,
                "source_cursor": 1,
                "source_record_id": "fee-schedule-record-1",
                "source_timestamp_utc_ms": 100,
                "fee_schedule_id": "v35-paper-sim-spot",
                "fee_schedule_version": "v1",
                "symbol": "BTC-USDT",
                "effective_from_utc_ms": 1,
                "effective_to_utc_ms": None,
                "maker_rate": 0.0008,
                "taker_rate": 0.001,
            }
            settled_execution = {
                "schema_version": "paper_exchange_financial_evidence:v1",
                "event_id": "paper-settlement:execution-1",
                "event_type": "settled_execution",
                "source_system": "paper_exchange_simulator",
                "source_epoch": epoch,
                "source_cursor": 2,
                "source_record_id": "execution-1",
                "source_timestamp_utc_ms": 200,
                "execution_id": "execution-1",
                "order_id": "order-1",
                "symbol": "BTC-USDT",
                "side": "sell",
                "quantity": 1.0,
                "price": 100.0,
                "fee": -0.08,
                "fee_currency": "USDT",
                "fee_schedule_id": "v35-paper-sim-spot",
                "fee_schedule_version": "v1",
                "execution_origin": "resting_limit",
            }
            events = []
            for cursor, event in enumerate((fee_schedule, settled_execution), start=1):
                events.append({
                    "source_cursor": cursor,
                    "source_epoch": epoch,
                    "source_record_id": event["source_record_id"],
                    "source_system": event["source_system"],
                    "source_timestamp_utc_ms": event["source_timestamp_utc_ms"],
                    "event": event,
                })
            source_bodies = [
                ("positions", "/positions/trades", "positions-trades-fixture.json", {
                    "trades": [{
                        "fill_id": "pm-fill-1",
                        "parent_order_id": "order-1",
                        "symbol": "BTC-USDT",
                        "side": "sell",
                        "quantity": 1.0,
                        "execution_price": 100.0,
                        "fee": -0.08,
                        "fee_currency": "USDT",
                        "fee_known": True,
                    }],
                    "total": 1,
                    "has_older": False,
                }),
                ("positions", "/positions/portfolio-evidence/intervals",
                 "positions-portfolio-evidence-intervals-fixture.json", {
                    "schema_version": 1,
                    "coverage": "no_durable_checkpoint",
                    "truncated": False,
                    "next_cursor": None,
                    "intervals": [],
                }),
                ("paper_exchange", "/internal/financial/evidence",
                 "paper_exchange-financial-evidence-complete-fixture.json", {
                    "schema_version": "paper_exchange_financial_evidence:v1",
                    "evidence_kind": "paper_exchange_source_attributed_account_evidence",
                    "source_epoch": "financial-history-epoch",
                    "cursor_kind": "financial_event",
                    "after_cursor": 0,
                    "next_cursor": None,
                    "max_cursor": 2,
                    "page_size": 2,
                    "page_complete": True,
                    "history_complete": True,
                    "opening_capital_status": "explicit_simulator_opening_capital_captured",
                    "fee_schedule_status": "versioned_simulator_fee_schedule_captured",
                    "source_attribution_status": "source_attributed_not_cryptographically_verified",
                    "captured_execution_flow_types": ["resting_limit"],
                    "uncovered_execution_flow_types": [],
                    "events": events,
                }),
            ]
            sources = []
            large_bundle_path = None
            for service, route, filename, body in source_bodies:
                body_bytes = json.dumps(body, separators=(",", ":")).encode()
                payload = {
                    "service": service,
                    "route": route,
                    "application_status": 200,
                    "bytes": len(body_bytes),
                    "body": body,
                }
                path = cache / filename
                path.write_text(json.dumps(payload))
                if service == "paper_exchange":
                    large_bundle_path = path.resolve()
                sources.append({
                    "service": service,
                    "endpoint": (
                        "portfolio-evidence-intervals"
                        if route == "/positions/portfolio-evidence/intervals"
                        else "financial-evidence"
                        if service == "paper_exchange"
                        else "trades"
                    ),
                    "status": "COLLECTED",
                    "evidence": str(path),
                    "bytes": len(body_bytes),
                })

            original_stat = Path.stat

            def report_large_bundle_size(path, *args, **kwargs):
                stat = original_stat(path, *args, **kwargs)
                if large_bundle_path is not None and path.name == large_bundle_path.name:
                    return SimpleNamespace(st_size=32_018_923)
                return stat

            with patch.object(Path, "stat", report_large_bundle_size):
                loaded = load_execution_reconciliation_inputs(sources, cache)

            self.assertIsNotNone(loaded)
            self.assertEqual(len(loaded["execution_ledger"]["fills"]), 1)
            result = reconcile_independent_execution_fills(
                loaded["position_trades"],
                loaded["execution_ledger"],
                loaded["position_source_coverage"],
            )
            self.assertEqual(result["status"], "RECONCILED")
            self.assertEqual(result["matched_order_count"], 1)
            self.assertEqual(result["fee_rows_reconciled"], 1)

    def test_paged_trade_collector_persists_complete_source_bundle(self):
        response = {"service": "positions", "route": "/positions/trades?limit=500&offset=0",
                    "application_status": 200, "bytes": 2,
                    "body": {"trades": [], "total": 0, "has_older": False,
                             "next_offset": None}}
        with patch("financial_snapshot.api", return_value=response), \
                patch("financial_snapshot.save", side_effect=["/tmp/page.json", "/tmp/bundle.json"]), \
                patch("financial_snapshot.now", return_value="2026-11-26T00:00:00+00:00"):
            collected = collect_paged_source("positions", "trades", "offset")
        self.assertEqual(collected["status"], "COLLECTED")
        self.assertTrue(collected["collector_bundle"])
        self.assertEqual(collected["body"]["total"], 0)
        self.assertEqual(len(collected["page_evidence"]), 1)

    def test_paper_exchange_event_collector_pins_initial_max_cursor(self):
        def event(cursor):
            return {"source_cursor": cursor, "event": {"source_cursor": cursor}}

        pages = [
            {"source_epoch": "epoch-1", "max_cursor": 2, "after_cursor": 0,
             "next_cursor": 1, "page_complete": False, "events": [event(1)]},
            # A concurrent append raised the live maximum to 3, but the
            # collector asks for the first page's stable through_cursor=2.
            {"source_epoch": "epoch-1", "max_cursor": 2, "after_cursor": 1,
             "next_cursor": None, "page_complete": True, "events": [event(2)]},
        ]
        responses = [{"service": "paper_exchange",
                      "route": "/internal/financial/evidence",
                      "application_status": 200, "bytes": 1, "body": page}
                     for page in pages]
        with patch("financial_snapshot.api", side_effect=responses) as api_mock, \
                patch("financial_snapshot.save", side_effect=["/tmp/page-1.json",
                      "/tmp/page-2.json", "/tmp/bundle.json"]):
            collected = collect_paged_source(
                "paper_exchange", "financial-evidence", "after_cursor", page_size=1
            )

        calls = [call.kwargs["params"] for call in api_mock.call_args_list]
        self.assertEqual(calls[0], {"limit": 1, "after_cursor": 0})
        self.assertEqual(calls[1], {
            "limit": 1, "after_cursor": 1, "through_cursor": 2,
            "through_epoch": "epoch-1",
        })
        self.assertEqual(collected["body"]["max_cursor"], 2)
        self.assertEqual([row["source_cursor"] for row in collected["body"]["events"]],
                         [1, 2])
        self.assertEqual(len(collected["page_evidence"]), 2)

    def test_interval_collector_aggregates_complete_rows_before_empty_terminal_page(self):
        pages = [
            {"schema_version": 1, "coverage": "pm_internal_complete",
             "truncated": True, "next_cursor": 2,
             "intervals": [{"evidence_id": "i-1", "complete": True}]},
            {"schema_version": 1, "coverage": "pm_internal_complete",
             "truncated": False, "next_cursor": 3,
             "intervals": [{"evidence_id": "i-2", "complete": True}]},
            {"schema_version": 1, "coverage": "no_durable_checkpoint",
             "truncated": False, "next_cursor": None, "intervals": []},
        ]
        responses = [{"service": "positions", "route": "/positions/portfolio-evidence/intervals",
                      "application_status": 200, "bytes": 1, "body": page}
                     for page in pages]
        with patch("financial_snapshot.api", side_effect=responses) as api_mock, \
                patch("financial_snapshot.save", side_effect=["/tmp/page-1.json",
                      "/tmp/page-2.json", "/tmp/page-3.json", "/tmp/bundle.json"]):
            collected = collect_paged_source(
                "positions", "portfolio-evidence-intervals", "after_cursor", page_size=1
            )

        self.assertEqual([call.kwargs["params"]["after_cursor"]
                          for call in api_mock.call_args_list], [0, 2, 3])
        self.assertEqual(collected["body"]["coverage"], "pm_internal_complete")
        self.assertFalse(collected["body"]["truncated"])
        self.assertEqual([row["evidence_id"] for row in collected["body"]["intervals"]],
                         ["i-1", "i-2"])
        self.assertEqual(len(collected["page_evidence"]), 3)
        # The terminal empty page is retained for audit, but its local coverage
        # label does not erase coverage established by the populated pages.
        self.assertEqual(collected["page_evidence"][-1]["evidence"], "/tmp/page-3.json")

    def test_interval_collector_does_not_upgrade_incomplete_rows(self):
        pages = [
            {"schema_version": 1, "coverage": "pm_internal_complete",
             "truncated": False, "next_cursor": 1,
             "intervals": [{"evidence_id": "i-1", "complete": True}]},
            {"schema_version": 1, "coverage": "no_durable_checkpoint",
             "truncated": False, "next_cursor": None,
             "intervals": [{"evidence_id": "i-2", "complete": False}]},
        ]
        responses = [{"service": "positions", "route": "/positions/portfolio-evidence/intervals",
                      "application_status": 200, "bytes": 1, "body": page}
                     for page in pages]
        with patch("financial_snapshot.api", side_effect=responses), \
                patch("financial_snapshot.save", side_effect=["/tmp/page-1.json",
                      "/tmp/page-2.json", "/tmp/bundle.json"]):
            collected = collect_paged_source(
                "positions", "portfolio-evidence-intervals", "after_cursor", page_size=1
            )

        self.assertEqual(collected["body"]["coverage"], "pm_internal_incomplete")
        self.assertFalse(collected["body"]["truncated"])
        self.assertFalse(collected["body"]["intervals"][1]["complete"])
        self.assertEqual(len(collected["page_evidence"]), 2)

    def test_interval_collector_preserves_source_truncation_on_final_populated_page(self):
        pages = [
            {"schema_version": 1, "coverage": "pm_internal_complete",
             "truncated": True, "next_cursor": 1,
             "intervals": [{"evidence_id": "i-1", "complete": True}]},
            {"schema_version": 1, "coverage": "no_durable_checkpoint",
             "truncated": False, "next_cursor": None, "intervals": []},
        ]
        responses = [{"service": "positions", "route": "/positions/portfolio-evidence/intervals",
                      "application_status": 200, "bytes": 1, "body": page}
                     for page in pages]
        with patch("financial_snapshot.api", side_effect=responses), \
                patch("financial_snapshot.save", side_effect=["/tmp/page-1.json",
                      "/tmp/page-2.json", "/tmp/bundle.json"]):
            collected = collect_paged_source(
                "positions", "portfolio-evidence-intervals", "after_cursor", page_size=1
            )

        self.assertEqual(collected["body"]["coverage"], "pm_internal_complete")
        self.assertTrue(collected["body"]["truncated"])
        self.assertEqual(len(collected["body"]["intervals"]), 1)

    def test_interval_collector_preserves_no_checkpoint_for_genuinely_empty_history(self):
        response = {"service": "positions", "route": "/positions/portfolio-evidence/intervals",
                    "application_status": 200, "bytes": 1,
                    "body": {"schema_version": 1, "coverage": "no_durable_checkpoint",
                             "truncated": False, "next_cursor": None, "intervals": []}}
        with patch("financial_snapshot.api", return_value=response), \
                patch("financial_snapshot.save", side_effect=["/tmp/empty-page.json",
                      "/tmp/empty-bundle.json"]):
            collected = collect_paged_source(
                "positions", "portfolio-evidence-intervals", "after_cursor"
            )

        self.assertEqual(collected["body"]["coverage"], "no_durable_checkpoint")
        self.assertEqual(collected["body"]["intervals"], [])
        self.assertFalse(collected["body"]["truncated"])
        self.assertEqual(len(collected["page_evidence"]), 1)

    def test_paper_exchange_event_projection_requires_complete_monotonic_source_pages(self):
        events = []
        payloads = [
            {"event_type": "fee_schedule", "fee_schedule_id": "sim", "fee_schedule_version": "v1",
             "symbol": "BTC-USDT", "maker_rate": 0.001, "taker_rate": 0.002,
             "effective_from_utc_ms": 100, "effective_to_utc_ms": None},
            {"event_type": "settled_execution", "execution_id": "exec-1", "order_id": "order-1",
             "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0, "price": 100.0,
             "fee": -0.2, "fee_currency": "USDT", "fee_schedule_id": "sim",
             "fee_schedule_version": "v1", "execution_origin": "market_match"},
        ]
        for cursor, payload in enumerate(payloads, start=1):
            events.append({"source_cursor": cursor, "source_system": "paper_exchange_simulator",
                           "source_epoch": "epoch-1", "source_record_id": f"record-{cursor}",
                           "source_timestamp_utc_ms": 100 + cursor,
                           "event": {"schema_version": "paper_exchange_financial_evidence:v1",
                                     "event_id": f"event-{cursor}", "source_system": "paper_exchange_simulator",
                                     "source_epoch": "epoch-1", "source_cursor": cursor,
                                     "source_record_id": f"record-{cursor}",
                                     "source_timestamp_utc_ms": 100 + cursor, **payload}})
        page = {"schema_version": "paper_exchange_financial_evidence:v1",
                "evidence_kind": "paper_exchange_source_attributed_account_evidence",
                "source_epoch": "ledger-epoch-1", "cursor_kind": "financial_event",
                "after_cursor": 0, "next_cursor": None, "max_cursor": 2, "page_size": 2,
                "page_complete": True, "history_complete": True,
                "source_attribution_status": "source_attributed_not_cryptographically_verified",
                "opening_capital_status": "explicit_simulator_opening_capital_captured",
                "fee_schedule_status": "versioned_simulator_fee_schedule_captured",
                "uncovered_execution_flow_types": [], "events": events}
        projected = project_paper_exchange_financial_evidence_page(page)
        self.assertTrue(projected["complete"])
        self.assertEqual(projected["fee_schedules"][0]["taker_rate"], 0.002)
        self.assertEqual(projected["fills"][0]["execution_origin"], "market_match")
        broken = copy.deepcopy(page)
        broken["events"][1]["source_cursor"] = 3
        self.assertIsNone(project_paper_exchange_financial_evidence_page(broken))

    def test_paper_exchange_cash_reconstruction_uses_opening_events_fills_contributions_and_fx(self):
        pm_intervals, _contract, coverage, trades, trade_coverage = self.synthetic_portfolio_interval_inputs()
        trades[1]["paper_exchange_execution_id"] = "exchange-fill-2"
        trades[2]["paper_exchange_execution_id"] = "exchange-fill-3"
        pm_intervals[0]["external_contribution_events"][0].update({
            "independent_source_epoch": "ledger-source-epoch",
            "independent_source_cursor": 5,
            "independent_source_record_id": "deposit-1",
        })
        source_events = []
        def add(event_id, source_system, source_epoch, source_record_id, source_time,
                source_cursor, payload):
            event = {"schema_version": "paper_exchange_financial_evidence:v1",
                     "event_id": event_id, "source_system": source_system,
                     "source_epoch": source_epoch, "source_cursor": source_cursor,
                     "source_record_id": source_record_id,
                     "source_timestamp_utc_ms": source_time, **payload}
            local_cursor = len(source_events) + 1
            source_events.append({"source_cursor": local_cursor, "source_system": source_system,
                                 "source_epoch": source_epoch, "source_record_id": source_record_id,
                                 "source_timestamp_utc_ms": source_time, "event": event})

        add("opening-1", "paper_exchange_simulator_account_configuration", "ledger-source-epoch",
            "opening-record", 500, 1, {"event_type": "account_balance", "portfolio_id": "paper-default",
            "boundary": "opening", "balances_by_currency": {"USD": 10_000.0, "USDT": 1_000.0}})
        add("fees-1", "paper_exchange_simulator_configuration", "ledger-source-epoch",
            "fee-schedule-record", 500, 2, {"event_type": "fee_schedule", "fee_schedule_id": "synthetic-fees-btc",
            "fee_schedule_version": "fixture-v1", "symbol": "BTC-USDT", "effective_from_utc_ms": 500,
            "effective_to_utc_ms": None, "maker_rate": 0.0001, "taker_rate": 0.0001})
        add("exchange-fill-2", "paper_exchange_simulator", "ledger-source-epoch", "exchange-fill-2", 1_500, 2,
            {"event_type": "settled_execution", "execution_id": "exchange-fill-2", "order_id": "order-buy",
             "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0, "price": 10_000.0,
             "fee": -1.0, "fee_currency": "USDT", "fee_schedule_id": "synthetic-fees-btc",
             "fee_schedule_version": "fixture-v1", "execution_origin": "market_match"})
        add("exchange-fill-3", "paper_exchange_simulator", "ledger-source-epoch", "exchange-fill-3", 2_500, 3,
            {"event_type": "settled_execution", "execution_id": "exchange-fill-3", "order_id": "order-sell",
             "symbol": "BTC-USDT", "side": "sell", "quantity": 1.0, "price": 10_100.0,
             "fee": -1.01, "fee_currency": "USDT", "fee_schedule_id": "synthetic-fees-btc",
             "fee_schedule_version": "fixture-v1", "execution_origin": "market_match"})
        add("deposit-event", "paper_operator", "paper-operator-contributions:v1", "deposit-1", 1_500, 1,
            {"event_type": "cash_movement", "movement_id": "provider-deposit-1", "portfolio_id": "paper-default",
             "movement_kind": "contribution", "currency": "USD", "amount": 100.0})
        for boundary, timestamp in (("opening", 1_000), ("closing", 3_000)):
            for currency in ("USD", "USDT"):
                add(f"fx-{boundary}-{currency}", "fixed_financial_fixture", "fx-v1",
                    f"fx-{boundary}-{currency}", timestamp, 1 if boundary == "opening" else 2,
                    {"event_type": "fx_mark", "portfolio_id": "paper-default", "currency": currency,
                     "rate_usd": 1.0})
        page = {"schema_version": "paper_exchange_financial_evidence:v1",
                "evidence_kind": "paper_exchange_source_attributed_account_evidence",
                "source_epoch": "ledger-source-epoch", "cursor_kind": "financial_event",
                "after_cursor": 0, "next_cursor": None, "max_cursor": len(source_events),
                "page_size": len(source_events), "page_complete": True, "history_complete": True,
                "source_attribution_status": "source_attributed_not_cryptographically_verified",
                "opening_capital_status": "explicit_simulator_opening_capital_captured",
                "fee_schedule_status": "versioned_simulator_fee_schedule_captured",
                "uncovered_execution_flow_types": [], "events": source_events}
        independent = reconstruct_portfolio_intervals_from_paper_exchange(page, pm_intervals, trades)
        self.assertIsNotNone(independent)
        wrong_portfolio = copy.deepcopy(page)
        wrong_portfolio["events"][0]["event"]["portfolio_id"] = "other-paper-account"
        self.assertIsNone(reconstruct_portfolio_intervals_from_paper_exchange(
            wrong_portfolio, pm_intervals, trades
        ))
        result = reconcile_independent_portfolio_intervals(
            pm_intervals, independent, coverage, trades, trade_coverage
        )
        self.assertEqual(result["status"], "RECONCILED", result)

    def body(self):
        return {
            "snapshot_id": "pm-paper-v1:123:456:1",
            "execution_mode": "paper",
            "snapshot_consistency": "single_locked_ledger_clone",
            "ledger_snapshot_consistent": True,
            "account": {},
            "positions": {},
            "realized_pnl": {},
            "fee_summary": {},
            "fee_rates": {},
            "ledger_coverage": {
                "trade_rows": 3,
                "known_fee_fills": 2,
                "unknown_fee_fills": 1,
                "fee_currency_unknown_rows": 0,
                "fee_cashflow_currency_mismatch_rows": 0,
                "parent_order_linked_rows": 2,
                "rows_without_parent_order_id": 1,
                "timestamp_order_inversion_rows": 0,
                "independent_execution_ledger_reconciled": False,
            },
        }

    def pm_source_coverage(self, rows, **overrides):
        return {
            "has_older": False,
            "total_records": len(rows),
            "returned_records": len(rows),
            **overrides,
        }

    def attach_fee_schedule_evidence(self, ledger):
        """Attach explicitly synthetic fee schedule provenance to a test fixture."""
        schedules = {}
        inferred_rates = {}
        for row in ledger["fills"]:
            notional = float(row.get("quantity", 0)) * float(row.get("price", 0))
            if notional > 0:
                inferred_rates.setdefault(row["symbol"], abs(float(row.get("fee", 0))) / notional)
        for index, row in enumerate(ledger["fills"], start=1):
            symbol = row["symbol"]
            schedule_id = f"fixture-fees-{symbol}"
            schedule_version = "fixture-v1"
            row.update({
                "fee_schedule_id": schedule_id,
                "fee_schedule_version": schedule_version,
                "timestamp_utc_ms": 1_500 + index,
                "source_system": "synthetic_test_fixture_only",
                "source_epoch": "fixture-execution-epoch",
                "source_cursor": index,
                "source_record_id": f"fixture-execution-{index}",
                "execution_origin": "market_match",
            })
            schedules[(schedule_id, schedule_version, symbol)] = {
                "fee_schedule_id": schedule_id,
                "fee_schedule_version": schedule_version,
                "symbol": symbol,
                "source_system": "synthetic_test_fixture_only",
                "source_record_id": f"fixture-schedule-{symbol}",
                "effective_from_utc_ms": 1_000,
                "effective_to_utc_ms": 10_000,
                "maker_rate": inferred_rates[symbol],
                "taker_rate": inferred_rates[symbol],
            }
        ledger["fee_schedules"] = list(schedules.values())
        return ledger

    def match_evidence_inputs(self):
        pm_trades = [{
            "fill_id": "pm-fill:1",
            "parent_order_id": "order-1",
            "paper_exchange_schema_version": "paper_exchange_match:v1",
            "paper_exchange_source_epoch": "a" * 32,
            "paper_exchange_source_cursor": 7,
            "paper_exchange_request_fingerprint": "fingerprint-a",
            "paper_exchange_execution_id": "pe-fill:1",
            "paper_exchange_source_quantity": 0.5,
            "paper_exchange_source_complete": True,
            "symbol": "BTC-USDT",
            "side": "buy",
            "quantity": 0.5,
            "execution_price": 100.0,
        }]
        match_ledger = {
            "schema_version": "paper_exchange_match_ledger:v1",
            "evidence_kind": "paper_exchange_match_evidence",
            "capture_scope": "matches_with_caller_supplied_order_id_only",
            "reconciliation_eligible": False,
            "source_epoch": "a" * 32,
            "cursor_kind": "match_fill",
            "history_complete": False,
            "after_cursor": 0,
            "next_cursor": 7,
            "max_cursor": 7,
            "page_size": 1,
            "page_complete": True,
            "identity_complete": True,
            "rows": [{
                "source_cursor": 7,
                "source_execution_id": "pe-fill:1",
                "order_id": "order-1",
                "request_fingerprint": "fingerprint-a",
                "symbol": "BTC-USDT",
                "side": "buy",
                "quantity": 0.5,
                "price": 100.0,
                "recorded_at_utc_ms": 123,
                "quote_evidence": {"best_ask": 100.0},
            }],
        }
        return pm_trades, match_ledger

    def synthetic_portfolio_interval_inputs(self):
        """Synthetic test-only PM and Paper Exchange contract fixture."""
        position_trades = [
            {
                "fill_id": "pm-fill:v1:epoch-a:1",
                "parent_order_id": "prior-order",
                "symbol": "BTC-USDT",
                "side": "buy",
                "quantity": 1.0,
                "execution_price": 100.0,
                "timestamp": 900,
                "fee": -0.1,
                "fee_known": True,
                "fee_currency": "USDT",
            },
            {
                "fill_id": "pm-fill:v1:epoch-a:2",
                "parent_order_id": "order-buy",
                "symbol": "BTC-USDT",
                "side": "buy",
                "quantity": 1.0,
                "execution_price": 10_000.0,
                "timestamp": 1_500,
                "fee": -1.0,
                "fee_known": True,
                "fee_currency": "USDT",
            },
            {
                "fill_id": "pm-fill:v1:epoch-a:3",
                "parent_order_id": "order-sell",
                "symbol": "BTC-USDT",
                "side": "sell",
                "quantity": 1.0,
                "execution_price": 10_100.0,
                "timestamp": 2_500,
                "fee": -1.01,
                "fee_known": True,
                "fee_currency": "USDT",
            },
        ]
        pm_interval = {
            "schema_version": 1,
            "evidence_id": "pm-portfolio-evidence:test-interval-1",
            "portfolio_id": "paper-default",
            "execution_mode": "paper",
            "interval_start_utc_ms": 1_000,
            "interval_end_utc_ms": 3_000,
            "ledger_epoch": "epoch-a",
            "fill_cursor_start": 1,
            "fill_cursor_end": 3,
            "trade_row_cursor_end": 3,
            "topup_cursor_start": 0,
            "topup_cursor_end": 1,
            "baseline_evidence_id": "pm-baseline:synthetic-test-only",
            "is_baseline": False,
            "complete": True,
            "truncated": False,
            "coverage_gap": False,
            "heartbeat_gap": False,
            "ledger_epoch_changed": False,
            "all_symbols_covered": True,
            "positions": [],
            "portfolio_return_complete": True,
            "portfolio_return_completeness_reasons": [],
            "portfolio_return_scope": "pm_internal_flat_cash_only",
            "unknown_cashflow_fills": 0,
            "known_fee_fills": 3,
            "unknown_fee_fills": 0,
            "legacy_fill_rows_without_allocator_identity": 0,
            "opening_cash_balances_by_currency": {"USD": 10_000.0, "USDT": 1_000.0},
            "cash_balances_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "opening_cash_value_usd_by_currency": {"USD": 10_000.0, "USDT": 1_000.0},
            "closing_cash_value_usd_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "opening_cash_total_usd": 11_000.0,
            "closing_cash_total_usd": 11_197.99,
            "opening_fx_rates_usd_by_currency": {"USD": 1.0, "USDT": 1.0},
            "opening_fx_rates_updated_at_by_currency": {"USD": 1_000, "USDT": 1_000},
            "closing_fx_rates_usd_by_currency": {"USD": 1.0, "USDT": 1.0},
            "closing_fx_rates_updated_at_by_currency": {"USD": 3_000, "USDT": 3_000},
            "cash_movements_by_currency": {"USD": 100.0, "USDT": 97.99},
            "external_contributions_by_currency": {"USD": 100.0},
            "external_contribution_events": [{
                "contribution_id": "pm-topup:v1:0",
                "source": "position_manager_account_topup_ledger",
                "source_cursor": 0,
                "timestamp_utc_ms": 1_500,
                "currency": "USD",
                "amount": 100.0,
                "value_usd": 100.0,
                "independent_source_record_id": "provider-deposit-record-1",
                "independent_source_epoch": "paper-exchange-fixture-epoch",
                "independent_source_cursor": 91,
            }],
            "flat_cash_period_net_pnl_usd": 97.99,
            "flat_cash_period_return_pct": 97.99 / 11_000.0 * 100.0,
        }
        independent_interval = {
            "interval_id": "exchange-interval:test-1",
            "complete": True,
            **{key: pm_interval[key] for key in (
                "portfolio_id", "execution_mode", "ledger_epoch",
                "interval_start_utc_ms", "interval_end_utc_ms",
                "fill_cursor_start", "fill_cursor_end", "trade_row_cursor_end",
                "topup_cursor_start", "topup_cursor_end",
            )},
            "opening_cash_balances_by_currency": {"USD": 10_000.0, "USDT": 1_000.0},
            "closing_cash_balances_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "opening_balance_evidence": {
                "schema_version": 1, "source_system": "synthetic_test_fixture_only",
                "source_record_id": "account-snapshot-opening", "source_epoch": "account-fixture-epoch",
                "source_cursor": 1, "source_timestamp_utc_ms": 1_000,
                "balances_by_currency": {"USD": 10_000.0, "USDT": 1_000.0},
            },
            "closing_balance_evidence": {
                "schema_version": 1, "source_system": "synthetic_test_fixture_only",
                "source_record_id": "account-snapshot-closing", "source_epoch": "account-fixture-epoch",
                "source_cursor": 8, "source_timestamp_utc_ms": 3_000,
                "balances_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            },
            "opening_cash_value_usd_by_currency": {"USD": 10_000.0, "USDT": 1_000.0},
            "closing_cash_value_usd_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "opening_cash_total_usd": 11_000.0,
            "closing_cash_total_usd": 11_197.99,
            "cash_movements_by_currency": {"USD": 100.0, "USDT": 97.99},
            "external_contributions_by_currency": {"USD": 100.0},
            "external_contribution_events": [{
                "contribution_id": "provider-deposit-1",
                "pm_contribution_id": "pm-topup:v1:0",
                "source": "synthetic_test_fixture_only",
                "source_record_id": "provider-deposit-record-1",
                "provider_source_epoch": "paper-exchange-fixture-epoch",
                "provider_source_cursor": 91,
                "source_cursor": 0,
                "timestamp_utc_ms": 1_500,
                "currency": "USD",
                "amount": 100.0,
                "value_usd": 100.0,
            }],
            "known_fee_fills": 2,
            "unknown_fee_fills": 0,
            "fee_schedules": [{
                "fee_schedule_id": "synthetic-fees-btc",
                "fee_schedule_version": "fixture-v1",
                "symbol": "BTC-USDT",
                "source_system": "synthetic_test_fixture_only",
                "source_record_id": "synthetic-fee-schedule-only",
                "effective_from_utc_ms": 1_000,
                "effective_to_utc_ms": 3_000,
                "maker_rate": 0.0001,
                "taker_rate": 0.0001,
            }],
            "fills": [
                {"execution_id": "exchange-fill-2", "fill_cursor": 2,
                 "order_id": "order-buy", "symbol": "BTC-USDT", "side": "buy",
                 "quantity": 1.0, "price": 10_000.0, "fee": -1.0,
                 "fee_known": True, "fee_currency": "USDT",
                 "fee_schedule_id": "synthetic-fees-btc",
                 "fee_schedule_version": "fixture-v1", "timestamp_utc_ms": 1_500,
                 "source_system": "synthetic_test_fixture_only", "source_epoch": "exchange-fixture-epoch",
                 "source_cursor": 2, "source_record_id": "exchange-fill-record-2", "execution_origin": "market_match"},
                {"execution_id": "exchange-fill-3", "fill_cursor": 3,
                 "order_id": "order-sell", "symbol": "BTC-USDT", "side": "sell",
                 "quantity": 1.0, "price": 10_100.0, "fee": -1.01,
                 "fee_known": True, "fee_currency": "USDT",
                 "fee_schedule_id": "synthetic-fees-btc",
                 "fee_schedule_version": "fixture-v1", "timestamp_utc_ms": 2_500,
                 "source_system": "synthetic_test_fixture_only", "source_epoch": "exchange-fixture-epoch",
                 "source_cursor": 3, "source_record_id": "exchange-fill-record-3", "execution_origin": "market_match"},
            ],
            "fx_marks": [
                {"boundary": "opening", "currency": "USD", "rate_usd": 1.0, "updated_at_utc_ms": 1_000,
                 "schema_version": 1, "source_system": "synthetic_test_fixture_only", "source_record_id": "fx-open-usd",
                 "source_epoch": "fx-fixture-epoch", "source_cursor": 1},
                {"boundary": "opening", "currency": "USDT", "rate_usd": 1.0, "updated_at_utc_ms": 1_000,
                 "schema_version": 1, "source_system": "synthetic_test_fixture_only", "source_record_id": "fx-open-usdt",
                 "source_epoch": "fx-fixture-epoch", "source_cursor": 2},
                {"boundary": "closing", "currency": "USD", "rate_usd": 1.0, "updated_at_utc_ms": 3_000,
                 "schema_version": 1, "source_system": "synthetic_test_fixture_only", "source_record_id": "fx-close-usd",
                 "source_epoch": "fx-fixture-epoch", "source_cursor": 3},
                {"boundary": "closing", "currency": "USDT", "rate_usd": 1.0, "updated_at_utc_ms": 3_000,
                 "schema_version": 1, "source_system": "synthetic_test_fixture_only", "source_record_id": "fx-close-usdt",
                 "source_epoch": "fx-fixture-epoch", "source_cursor": 4},
            ],
        }
        independent_contract = {
            "schema_version": 1,
            "contract": "paper_exchange_portfolio_interval_v1",
            "source_system": "paper_exchange",
            "execution_mode": "paper",
            "independent_source": True,
            "complete": True,
            "snapshot_id": "paper-exchange-fixture-only:v1:1",
            "baseline_evidence_id": "pm-baseline:synthetic-test-only",
            "fixture_only": True,
            "intervals": [independent_interval],
        }
        return (
            [pm_interval], independent_contract,
            {"schema_version": 1, "coverage": "pm_internal_complete",
             "truncated": False, "returned_intervals": 1,
             "baseline_verified": True,
             "baseline_evidence_id": "pm-baseline:synthetic-test-only"},
            position_trades,
            self.pm_source_coverage(position_trades),
        )

    def test_accepts_complete_ledger_snapshot_contract_without_claiming_atomic_exchange(self):
        self.assertTrue(valid_financial_snapshot_body(self.body()))

    def test_rejects_missing_or_inconsistent_coverage(self):
        missing = self.body()
        del missing["ledger_coverage"]["rows_without_parent_order_id"]
        self.assertFalse(valid_financial_snapshot_body(missing))

        inconsistent = self.body()
        inconsistent["ledger_coverage"]["known_fee_fills"] = 1
        self.assertFalse(valid_financial_snapshot_body(inconsistent))

        boolean_count = self.body()
        boolean_count["ledger_coverage"]["trade_rows"] = True
        self.assertFalse(valid_financial_snapshot_body(boolean_count))

    def test_rejects_unverified_external_ledger_claim(self):
        unverified = self.body()
        unverified["ledger_coverage"]["independent_execution_ledger_reconciled"] = True
        self.assertFalse(valid_financial_snapshot_body(unverified))

    def test_match_evidence_comparison_matches_observed_fill_without_financial_claim(self):
        pm_trades, match_ledger = self.match_evidence_inputs()
        result = compare_paper_exchange_match_evidence(
            pm_trades, match_ledger, self.pm_source_coverage(pm_trades)
        )
        self.assertEqual(result["status"], "MATCHED_OBSERVED_EVIDENCE_PARTIAL_HISTORY")
        self.assertTrue(result["observed_rows_consistent"])
        self.assertEqual(result["matched_fill_count"], 1)
        self.assertEqual(result["source_epoch"], "a" * 32)
        self.assertTrue(result["page_complete"])
        self.assertFalse(result["source_history_complete"])
        self.assertFalse(result["financial_reconciliation_eligible"])
        self.assertFalse(result["completed_financial_reconciliation"])
        self.assertFalse(any("return" in key or "profit" in key for key in result))

    def test_match_evidence_mismatches_duplicates_and_pm_clipping_fail_closed(self):
        pm_trades, match_ledger = self.match_evidence_inputs()

        clipped = copy.deepcopy(pm_trades)
        clipped[0]["quantity"] = 0.25
        clipped[0]["paper_exchange_source_complete"] = False
        clipped_result = compare_paper_exchange_match_evidence(
            clipped, match_ledger, self.pm_source_coverage(clipped)
        )
        self.assertEqual(clipped_result["clipped_fill_count"], 1)
        self.assertGreater(clipped_result["missing_source_link_count"], 0)
        self.assertIn(
            "position_manager_clipped_quantity",
            clipped_result["mismatches"][0]["reasons"],
        )
        self.assertFalse(clipped_result["financial_reconciliation_eligible"])

        mixed_epoch = copy.deepcopy(pm_trades)
        mixed_epoch[0]["paper_exchange_source_epoch"] = "b" * 32
        mixed_result = compare_paper_exchange_match_evidence(
            mixed_epoch, match_ledger, self.pm_source_coverage(mixed_epoch)
        )
        self.assertIn("source_epoch_mismatch", mixed_result["mismatches"][0]["reasons"])

        wrong_fingerprint = copy.deepcopy(pm_trades)
        wrong_fingerprint[0]["paper_exchange_request_fingerprint"] = "foreign"
        fingerprint_result = compare_paper_exchange_match_evidence(
            wrong_fingerprint, match_ledger, self.pm_source_coverage(wrong_fingerprint)
        )
        self.assertIn(
            "request_fingerprint_mismatch",
            fingerprint_result["mismatches"][0]["reasons"],
        )

        foreign_order = copy.deepcopy(match_ledger)
        foreign_order["rows"][0]["order_id"] = "foreign-order"
        order_result = compare_paper_exchange_match_evidence(
            pm_trades, foreign_order, self.pm_source_coverage(pm_trades)
        )
        self.assertIn("source_order_id_mismatch", order_result["mismatches"][0]["reasons"])

        wrong_source_quantity = copy.deepcopy(match_ledger)
        wrong_source_quantity["rows"][0]["quantity"] = 0.6
        quantity_result = compare_paper_exchange_match_evidence(
            pm_trades, wrong_source_quantity, self.pm_source_coverage(pm_trades)
        )
        self.assertIn("source_quantity_mismatch", quantity_result["mismatches"][0]["reasons"])

        wrong_price = copy.deepcopy(pm_trades)
        wrong_price[0]["execution_price"] = 101.0
        notional_result = compare_paper_exchange_match_evidence(
            wrong_price, match_ledger, self.pm_source_coverage(wrong_price)
        )
        self.assertIn("execution_price_mismatch", notional_result["mismatches"][0]["reasons"])
        self.assertIn("source_notional_mismatch", notional_result["mismatches"][0]["reasons"])

        invalid_cursor_range = copy.deepcopy(match_ledger)
        invalid_cursor_range["max_cursor"] = 6
        cursor_range_result = compare_paper_exchange_match_evidence(
            pm_trades, invalid_cursor_range, self.pm_source_coverage(pm_trades)
        )
        self.assertTrue(any("cursor" in blocker for blocker in cursor_range_result["blockers"]))

        duplicate_ledger = copy.deepcopy(match_ledger)
        duplicate_ledger["rows"].append(copy.deepcopy(duplicate_ledger["rows"][0]))
        duplicate_ledger["page_size"] = 2
        duplicate_result = compare_paper_exchange_match_evidence(
            pm_trades, duplicate_ledger, self.pm_source_coverage(pm_trades)
        )
        self.assertTrue(any("duplicate" in blocker for blocker in duplicate_result["blockers"]))
        self.assertFalse(duplicate_result["financial_reconciliation_eligible"])

    def test_match_evidence_missing_links_cursor_mismatches_and_incomplete_pages_block(self):
        pm_trades, match_ledger = self.match_evidence_inputs()
        missing = copy.deepcopy(pm_trades)
        missing[0]["paper_exchange_execution_id"] = None
        missing_result = compare_paper_exchange_match_evidence(
            missing, match_ledger, self.pm_source_coverage(missing)
        )
        self.assertGreater(missing_result["missing_source_link_count"], 0)

        wrong_cursor = copy.deepcopy(pm_trades)
        wrong_cursor[0]["paper_exchange_source_cursor"] = 8
        cursor_result = compare_paper_exchange_match_evidence(
            wrong_cursor, match_ledger, self.pm_source_coverage(wrong_cursor)
        )
        self.assertIn("source_cursor_mismatch", cursor_result["mismatches"][0]["reasons"])

        incomplete_page = copy.deepcopy(match_ledger)
        incomplete_page["max_cursor"] = 8
        incomplete_page["page_complete"] = False
        incomplete_result = compare_paper_exchange_match_evidence(
            pm_trades, incomplete_page, self.pm_source_coverage(pm_trades)
        )
        self.assertFalse(incomplete_result["page_complete"])
        self.assertFalse(incomplete_result["source_history_complete"])
        self.assertFalse(incomplete_result["financial_reconciliation_eligible"])

        missing_coverage = compare_paper_exchange_match_evidence(pm_trades, match_ledger)
        self.assertFalse(missing_coverage["position_source_complete"])
        self.assertTrue(any("coverage" in blocker for blocker in missing_coverage["blockers"]))

    def test_independent_execution_ledger_reconciles_by_stable_order_identity(self):
        pm_trades = [
            {"fill_id": "pm-fill:1", "parent_order_id": "order-1", "symbol": "BTC-USDT",
             "side": "buy", "quantity": 0.4, "execution_price": 100.0,
             "fee": -0.04, "fee_known": True, "fee_currency": "USDT"},
            {"fill_id": "pm-fill:2", "parent_order_id": "order-1", "symbol": "BTC-USDT",
             "side": "buy", "quantity": 0.6, "execution_price": 101.0,
             "fee": -0.0606, "fee_known": True, "fee_currency": "USDT"},
        ]
        independent = {
            "schema_version": 1,
            "source_system": "paper_exchange",
            "execution_mode": "paper",
            "independent_source": True,
            "complete": True,
            "snapshot_id": "paper-exchange-v1:fixture-1",
            "fills": [
                {"execution_id": "ex-1", "order_id": "order-1", "symbol": "BTC-USDT",
                 "side": "buy", "quantity": 0.4, "price": 100.0,
                 "fee": -0.04, "fee_known": True, "fee_currency": "USDT"},
                {"execution_id": "ex-2", "order_id": "order-1", "symbol": "BTC-USDT",
                 "side": "buy", "quantity": 0.6, "price": 101.0,
                 "fee": -0.0606, "fee_known": True, "fee_currency": "USDT"},
            ],
        }
        self.attach_fee_schedule_evidence(independent)
        result = reconcile_independent_execution_fills(
            pm_trades, independent, self.pm_source_coverage(pm_trades)
        )
        self.assertTrue(result["independent_execution_ledger_reconciled"])
        self.assertEqual(result["matched_order_count"], 1)
        self.assertEqual(result["mismatch_count"], 0)
        self.assertEqual(result["fee_rows_reconciled"], 2)
        self.assertTrue(result["position_source_complete"])
        self.assertTrue(result["independent_execution_source_complete"])
        self.assertNotIn("profit", result)

    def test_independent_execution_reconciliation_reports_unmatched_and_mismatch(self):
        pm_trades = [
            {"fill_id": "pm-fill:1", "parent_order_id": "order-pm", "symbol": "ETH-USDT",
             "side": "sell", "quantity": 2.0, "execution_price": 50.0,
             "fee": -0.1, "fee_known": True, "fee_currency": "USDT"},
        ]
        independent = {
            "schema_version": 1,
            "source_system": "paper_exchange",
            "execution_mode": "paper",
            "independent_source": True,
            "complete": True,
            "snapshot_id": "paper-exchange-v1:fixture-2",
            "fills": [
                {"execution_id": "ex-1", "order_id": "order-pm", "symbol": "ETH-USDT",
                 "side": "sell", "quantity": 1.0, "price": 50.0,
                 "fee": -0.05, "fee_known": True, "fee_currency": "USDT"},
                {"execution_id": "ex-2", "order_id": "order-ex", "symbol": "ETH-USDT",
                 "side": "sell", "quantity": 1.0, "price": 50.0,
                 "fee": -0.05, "fee_known": True, "fee_currency": "USDT"},
            ],
        }
        self.attach_fee_schedule_evidence(independent)
        result = reconcile_independent_execution_fills(
            pm_trades, independent, self.pm_source_coverage(pm_trades)
        )
        self.assertEqual(result["status"], "MISMATCH")
        self.assertEqual(result["unmatched_position_orders"], [])
        self.assertEqual(result["unmatched_execution_orders"], ["order-ex"])
        self.assertEqual(result["mismatch_count"], 1)
        self.assertFalse(result["independent_execution_ledger_reconciled"])

    def test_fee_amount_and_currency_mismatches_are_reported_per_order(self):
        pm_trades = [
            {"fill_id": "pm-fill-amount", "parent_order_id": "order-amount",
             "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0,
             "execution_price": 100.0, "fee": -0.1, "fee_known": True,
             "fee_currency": "USDT"},
            {"fill_id": "pm-fill-currency", "parent_order_id": "order-currency",
             "symbol": "ETH-USDT", "side": "sell", "quantity": 1.0,
             "execution_price": 50.0, "fee": -0.05, "fee_known": True,
             "fee_currency": "USDT"},
        ]
        independent = {
            "schema_version": 1, "source_system": "paper_exchange",
            "execution_mode": "paper", "independent_source": True,
            "complete": True, "snapshot_id": "paper-exchange-fees",
            "fills": [
                {"execution_id": "ex-amount", "order_id": "order-amount",
                 "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0,
                 "price": 100.0, "fee": -0.09, "fee_known": True,
                 "fee_currency": "USDT"},
                {"execution_id": "ex-currency", "order_id": "order-currency",
                 "symbol": "ETH-USDT", "side": "sell", "quantity": 1.0,
                 "price": 50.0, "fee": -0.05, "fee_known": True,
                 "fee_currency": "USD"},
            ],
        }
        self.attach_fee_schedule_evidence(independent)

        result = reconcile_independent_execution_fills(
            pm_trades, independent, self.pm_source_coverage(pm_trades)
        )

        self.assertFalse(result["independent_execution_ledger_reconciled"])
        self.assertFalse(result["independent_execution_ledger_reconciled"])
        self.assertTrue(any("quote currency" in blocker for blocker in result["blockers"]))

    def test_fee_schedule_arithmetic_mismatch_fails_closed(self):
        trade = {"fill_id": "pm-fill-1", "parent_order_id": "order-1",
                 "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0,
                 "execution_price": 100.0, "fee": -0.09, "fee_known": True,
                 "fee_currency": "USDT"}
        ledger = {"schema_version": 1, "source_system": "paper_exchange",
                  "execution_mode": "paper", "independent_source": True,
                  "complete": True, "snapshot_id": "fee-arithmetic",
                  "fills": [{"execution_id": "ex-1", "order_id": "order-1",
                             "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0,
                             "price": 100.0, "fee": -0.09, "fee_known": True,
                             "fee_currency": "USDT"}]}
        self.attach_fee_schedule_evidence(ledger)
        ledger["fee_schedules"][0]["taker_rate"] = 0.001
        ledger["fills"][0]["fee"] = -0.09
        result = reconcile_independent_execution_fills(
            [trade], ledger, self.pm_source_coverage([trade])
        )
        self.assertFalse(result["independent_execution_ledger_reconciled"])
        self.assertIn("signed fee differs", result["blockers"][0])

    def test_missing_fee_and_incomplete_source_metadata_fail_closed(self):
        pm_trade = {
            "fill_id": "pm-fill-1", "parent_order_id": "order-1",
            "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0,
            "execution_price": 100.0, "fee": -0.1, "fee_known": True,
            "fee_currency": "USDT",
        }
        independent = {
            "schema_version": 1, "source_system": "paper_exchange",
            "execution_mode": "paper", "independent_source": True,
            "complete": True, "snapshot_id": "paper-exchange-incomplete-fees",
            "fills": [{
                "execution_id": "ex-1", "order_id": "order-1",
                "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0,
                "price": 100.0,
            }],
        }
        self.attach_fee_schedule_evidence(independent)
        coverage = self.pm_source_coverage([pm_trade])
        result = reconcile_independent_execution_fills([pm_trade], independent, coverage)
        self.assertFalse(result["independent_execution_ledger_reconciled"])
        self.assertIn("Paper Exchange row has unknown fee metadata", result["blockers"])

        incomplete_coverage = self.pm_source_coverage(
            [pm_trade], has_older=True
        )
        blocked = reconcile_independent_execution_fills(
            [pm_trade], independent, incomplete_coverage
        )
        self.assertFalse(blocked["position_source_complete"])
        self.assertFalse(blocked["independent_execution_source_complete"])
        self.assertIn(
            "Position fill source lacks complete, count-matched history coverage",
            blocked["blockers"],
        )

    def test_missing_independent_source_and_duplicate_execution_ids_fail_closed(self):
        self.assertFalse(
            reconcile_independent_execution_fills(None, None, None)[
                "independent_execution_ledger_reconciled"
            ]
        )
        position_trade = {
            "fill_id": "pm-fill:1", "parent_order_id": "order-1", "symbol": "BTC-USDT",
            "side": "buy", "quantity": 1.0, "execution_price": 100.0,
            "fee": -0.1, "fee_known": True, "fee_currency": "USDT",
        }
        external_fill = {
            "execution_id": "ex-1", "order_id": "order-1", "symbol": "BTC-USDT",
            "side": "buy", "quantity": 1.0, "price": 100.0,
            "fee": -0.1, "fee_known": True, "fee_currency": "USDT",
        }
        independent = {
            "schema_version": 1, "source_system": "paper_exchange", "execution_mode": "paper",
            "independent_source": True, "complete": True,
            "snapshot_id": "paper-exchange-v1:fixture-3",
            "fills": [external_fill, dict(external_fill)],
        }
        self.attach_fee_schedule_evidence(independent)
        result = reconcile_independent_execution_fills(
            [position_trade], independent, self.pm_source_coverage([position_trade])
        )
        self.assertFalse(result["independent_execution_ledger_reconciled"])
        self.assertIn("duplicate execution identity", result["blockers"][0])

    def test_synthetic_portfolio_interval_contract_reconciles_all_cash_evidence(self):
        """Synthetic test-only fixture; no historical or runtime evidence."""
        inputs = self.synthetic_portfolio_interval_inputs()
        result = reconcile_independent_portfolio_intervals(*inputs)

        self.assertEqual(result["status"], "RECONCILED")
        self.assertTrue(result["independent_portfolio_interval_reconciled"])
        self.assertTrue(result["source_is_synthetic_test_fixture"])
        self.assertEqual(result["matched_interval_count"], 1)
        interval = result["intervals"][0]
        self.assertEqual(interval["position_fill_rows_reconciled"], 2)
        self.assertEqual(interval["known_fee_fill_rows_reconciled"], 2)
        self.assertEqual(interval["contribution_events_reconciled"], 1)
        self.assertAlmostEqual(
            interval["reconciled_flat_cash_period_net_pnl_usd"], 97.99
        )
        self.assertAlmostEqual(
            interval["reconciled_flat_cash_period_return_percent"],
            97.99 / 11_000.0 * 100.0,
        )

    def test_portfolio_interval_missing_or_mismatched_evidence_fails_closed(self):
        """Synthetic fixture mutations prove no return escapes a failed gate."""
        inputs = self.synthetic_portfolio_interval_inputs()
        pm_intervals, independent, coverage, trades, trade_coverage = inputs

        incomplete = copy.deepcopy(independent)
        incomplete["complete"] = False
        blocked = reconcile_independent_portfolio_intervals(
            pm_intervals, incomplete, coverage, trades, trade_coverage
        )
        self.assertFalse(blocked["independent_portfolio_interval_reconciled"])
        self.assertIsNone(blocked["reconciled_flat_cash_period_net_pnl_usd"])
        self.assertIsNone(blocked["reconciled_flat_cash_period_return_percent"])

        mismatched_fx = copy.deepcopy(independent)
        mismatched_fx["intervals"][0]["fx_marks"][-1]["updated_at_utc_ms"] -= 1
        mismatch = reconcile_independent_portfolio_intervals(
            pm_intervals, mismatched_fx, coverage, trades, trade_coverage
        )
        self.assertEqual(mismatch["status"], "MISMATCH")
        self.assertFalse(mismatch["independent_portfolio_interval_reconciled"])
        self.assertIsNone(mismatch["reconciled_flat_cash_period_net_pnl_usd"])
        self.assertIsNone(mismatch["reconciled_flat_cash_period_return_percent"])

        mismatched_contribution = copy.deepcopy(independent)
        mismatched_contribution["intervals"][0]["external_contribution_events"][0][
            "pm_contribution_id"
        ] = "unidentified-topup"
        contribution_mismatch = reconcile_independent_portfolio_intervals(
            pm_intervals,
            mismatched_contribution,
            coverage,
            trades,
            trade_coverage,
        )
        self.assertEqual(contribution_mismatch["status"], "MISMATCH")
        self.assertIsNone(
            contribution_mismatch["reconciled_flat_cash_period_return_percent"]
        )

    def test_portfolio_interval_requires_shared_cursor_bounds_and_complete_pages(self):
        """Synthetic source proves cursor/page gaps block interval returns."""
        inputs = self.synthetic_portfolio_interval_inputs()
        pm_intervals, independent, coverage, trades, trade_coverage = inputs

        mismatched_cursor = copy.deepcopy(independent)
        mismatched_cursor["intervals"][0]["fill_cursor_end"] += 1
        cursor_result = reconcile_independent_portfolio_intervals(
            pm_intervals, mismatched_cursor, coverage, trades, trade_coverage
        )
        self.assertFalse(cursor_result["independent_portfolio_interval_reconciled"])
        self.assertIsNone(cursor_result["reconciled_flat_cash_period_net_pnl_usd"])

        truncated_coverage = dict(coverage, truncated=True)
        page_result = reconcile_independent_portfolio_intervals(
            pm_intervals, independent, truncated_coverage, trades, trade_coverage
        )
        self.assertFalse(page_result["independent_portfolio_interval_reconciled"])
        self.assertIsNone(page_result["reconciled_flat_cash_period_return_percent"])

    def test_portfolio_interval_rejects_pm_copied_or_unlinked_account_evidence(self):
        """Independent source IDs are required; PM rows cannot be relabeled as independent."""
        inputs = self.synthetic_portfolio_interval_inputs()
        pm_intervals, independent, coverage, trades, trade_coverage = inputs

        copied_balance = copy.deepcopy(independent)
        copied_balance["intervals"][0]["opening_balance_evidence"]["source_system"] = (
            "position_management"
        )
        blocked_balance = reconcile_independent_portfolio_intervals(
            pm_intervals, copied_balance, coverage, trades, trade_coverage
        )
        self.assertFalse(blocked_balance["independent_portfolio_interval_reconciled"])
        self.assertIsNone(blocked_balance["reconciled_flat_cash_period_return_percent"])
        self.assertIn("copied from the Position Management", blocked_balance["blockers"][0])

        missing_contribution_source = copy.deepcopy(independent)
        del missing_contribution_source["intervals"][0]["external_contribution_events"][0][
            "source_record_id"
        ]
        blocked_contribution = reconcile_independent_portfolio_intervals(
            pm_intervals, missing_contribution_source, coverage, trades, trade_coverage
        )
        self.assertFalse(blocked_contribution["independent_portfolio_interval_reconciled"])
        self.assertIsNone(blocked_contribution["reconciled_flat_cash_period_return_percent"])

    def test_portfolio_interval_rejects_unavailable_or_mismatched_versioned_fee_schedule(self):
        inputs = self.synthetic_portfolio_interval_inputs()
        pm_intervals, independent, coverage, trades, trade_coverage = inputs

        missing = copy.deepcopy(independent)
        missing["intervals"][0]["fee_schedules"] = []
        blocked = reconcile_independent_portfolio_intervals(
            pm_intervals, missing, coverage, trades, trade_coverage
        )
        self.assertFalse(blocked["independent_portfolio_interval_reconciled"])
        self.assertIsNone(blocked["reconciled_flat_cash_period_return_percent"])
        self.assertTrue(any("fee schedule" in reason.lower() for reason in blocked["blockers"]))

        mismatch = copy.deepcopy(independent)
        mismatch["intervals"][0]["fills"][0]["fee_schedule_version"] = "foreign-version"
        rejected = reconcile_independent_portfolio_intervals(
            pm_intervals, mismatch, coverage, trades, trade_coverage
        )
        self.assertFalse(rejected["independent_portfolio_interval_reconciled"])
        self.assertIsNone(rejected["reconciled_flat_cash_period_return_percent"])

    def test_one_interval_mismatch_withholds_returns_for_the_entire_series(self):
        """Synthetic test-only second interval proves no partial return series leaks."""
        inputs = self.synthetic_portfolio_interval_inputs()
        pm_intervals, independent, coverage, trades, trade_coverage = inputs
        second_pm = copy.deepcopy(pm_intervals[0])
        second_pm.update({
            "evidence_id": "pm-portfolio-evidence:test-interval-2",
            "interval_start_utc_ms": 3_000,
            "interval_end_utc_ms": 4_000,
            "fill_cursor_start": 3,
            "fill_cursor_end": 3,
            "trade_row_cursor_end": 3,
            "topup_cursor_start": 1,
            "topup_cursor_end": 1,
            "opening_cash_balances_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "cash_balances_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "opening_cash_value_usd_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "closing_cash_value_usd_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "opening_cash_total_usd": 11_197.99,
            "closing_cash_total_usd": 11_197.99,
            "opening_fx_rates_updated_at_by_currency": {"USD": 3_000, "USDT": 3_000},
            "closing_fx_rates_updated_at_by_currency": {"USD": 4_000, "USDT": 4_000},
            "cash_movements_by_currency": {},
            "external_contributions_by_currency": {},
            "external_contribution_events": [],
            "flat_cash_period_net_pnl_usd": 0.0,
            "flat_cash_period_return_pct": 0.0,
        })
        second_external = copy.deepcopy(independent["intervals"][0])
        second_external.update({
            "interval_id": "exchange-interval:test-2",
            "interval_start_utc_ms": 3_000,
            "interval_end_utc_ms": 4_000,
            "fill_cursor_start": 3,
            "fill_cursor_end": 3,
            "trade_row_cursor_end": 3,
            "topup_cursor_start": 1,
            "topup_cursor_end": 1,
            "opening_cash_balances_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "closing_cash_balances_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "opening_cash_value_usd_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "closing_cash_value_usd_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "opening_cash_total_usd": 11_197.99,
            "closing_cash_total_usd": 11_197.99,
            "cash_movements_by_currency": {},
            "external_contributions_by_currency": {},
            "external_contribution_events": [],
            "known_fee_fills": 0,
            "fills": [],
            "fx_marks": [
                {"boundary": "opening", "currency": "USD", "rate_usd": 1.0, "updated_at_utc_ms": 3_000},
                {"boundary": "opening", "currency": "USDT", "rate_usd": 1.0, "updated_at_utc_ms": 3_000},
                {"boundary": "closing", "currency": "USD", "rate_usd": 1.0, "updated_at_utc_ms": 4_000},
                {"boundary": "closing", "currency": "USDT", "rate_usd": 1.0, "updated_at_utc_ms": 4_000},
            ],
        })
        # Deliberately mismatch the second source's closing balance.
        second_external["closing_cash_balances_by_currency"]["USD"] += 1.0
        pm_intervals.append(second_pm)
        independent["intervals"].append(second_external)
        coverage["returned_intervals"] = 2

        result = reconcile_independent_portfolio_intervals(
            pm_intervals, independent, coverage, trades, trade_coverage
        )

        self.assertEqual(result["status"], "MISMATCH")
        self.assertFalse(result["independent_portfolio_interval_reconciled"])
        self.assertIsNone(result["reconciled_flat_cash_period_net_pnl_usd"])
        self.assertIsNone(result["reconciled_flat_cash_period_return_percent"])
        self.assertEqual(result["intervals"][0]["status"], "WITHHELD")
        self.assertIsNone(
            result["intervals"][0]["reconciled_flat_cash_period_return_percent"]
        )

    def test_manifest_loader_never_accepts_a_synthetic_interval_as_live_evidence(self):
        """A deterministic fixture is test data, never collected history."""
        with TemporaryDirectory() as temporary:
            cache = Path(temporary).resolve()
            bodies = [
                (
                    "positions",
                    "/positions/trades",
                    "positions-trades-fixture.json",
                    {"has_older": False, "total": 0, "trades": []},
                ),
                (
                    "positions",
                    "/positions/portfolio-evidence/intervals",
                    "positions-portfolio-evidence-intervals-fixture.json",
                    {"schema_version": 1, "coverage": "pm_internal_complete",
                     "truncated": False, "next_cursor": 0, "intervals": []},
                ),
                (
                    "paper_exchange",
                    "/internal/executions/ledger",
                    "paper_exchange-execution-ledger-fixture.json",
                    {"portfolio_interval_contract": {
                        "schema_version": 1,
                        "contract": "paper_exchange_portfolio_interval_v1",
                        "source_system": "paper_exchange",
                        "execution_mode": "paper",
                        "independent_source": True,
                        "complete": True,
                        "snapshot_id": "synthetic-test-only",
                        "fixture_only": True,
                        "intervals": [],
                    }},
                ),
                (
                    "paper_exchange",
                    "/internal/orders/match/ledger",
                    "paper_exchange-match-evidence-ledger-fixture.json",
                    self.match_evidence_inputs()[1],
                ),
            ]
            sources = []
            for service, route, filename, body in bodies:
                body_bytes = json.dumps(body).encode()
                payload = {
                    "service": service,
                    "route": route,
                    "application_status": 200,
                    "bytes": len(body_bytes),
                    "body": body,
                }
                path = cache / filename
                path.write_text(json.dumps(payload))
                sources.append({
                    "service": service,
                    "endpoint": (
                        "portfolio-evidence-intervals"
                        if route == "/positions/portfolio-evidence/intervals"
                        else "match-evidence-ledger"
                        if route == "/internal/orders/match/ledger"
                        else "execution-ledger"
                        if service == "paper_exchange"
                        else "trades"
                    ),
                    "status": "COLLECTED",
                    "evidence": str(path),
                    "bytes": len(body_bytes),
                })

            loaded = load_execution_reconciliation_inputs(sources, cache)
            self.assertIsNotNone(loaded)
            self.assertIsNone(loaded["independent_portfolio_interval_contract"])
            self.assertEqual(
                loaded["match_evidence_ledger"]["schema_version"],
                "paper_exchange_match_ledger:v1",
            )
            comparison = compare_paper_exchange_match_evidence(
                loaded["position_trades"],
                loaded["match_evidence_ledger"],
                loaded["position_source_coverage"],
            )
            self.assertFalse(comparison["financial_reconciliation_eligible"])
            self.assertFalse(comparison["source_history_complete"])


if __name__ == "__main__":
    unittest.main()
