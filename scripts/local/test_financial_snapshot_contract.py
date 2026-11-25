import unittest
import copy
import json
from pathlib import Path
from tempfile import TemporaryDirectory

from financial import (
    compare_paper_exchange_match_evidence,
    reconcile_independent_execution_fills,
    reconcile_independent_portfolio_intervals,
    load_execution_reconciliation_inputs,
)
from financial_snapshot import valid_financial_snapshot_body


class FinancialSnapshotContractTests(unittest.TestCase):
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
            "opening_cash_value_usd_by_currency": {"USD": 10_000.0, "USDT": 1_000.0},
            "closing_cash_value_usd_by_currency": {"USD": 10_100.0, "USDT": 1_097.99},
            "opening_cash_total_usd": 11_000.0,
            "closing_cash_total_usd": 11_197.99,
            "cash_movements_by_currency": {"USD": 100.0, "USDT": 97.99},
            "external_contributions_by_currency": {"USD": 100.0},
            "external_contribution_events": copy.deepcopy(pm_interval["external_contribution_events"]),
            "known_fee_fills": 2,
            "unknown_fee_fills": 0,
            "fills": [
                {"execution_id": "exchange-fill-2", "fill_cursor": 2,
                 "order_id": "order-buy", "symbol": "BTC-USDT", "side": "buy",
                 "quantity": 1.0, "price": 10_000.0, "fee": -1.0,
                 "fee_known": True, "fee_currency": "USDT"},
                {"execution_id": "exchange-fill-3", "fill_cursor": 3,
                 "order_id": "order-sell", "symbol": "BTC-USDT", "side": "sell",
                 "quantity": 1.0, "price": 10_100.0, "fee": -1.01,
                 "fee_known": True, "fee_currency": "USDT"},
            ],
            "fx_marks": [
                {"boundary": "opening", "currency": "USD", "rate_usd": 1.0, "updated_at_utc_ms": 1_000},
                {"boundary": "opening", "currency": "USDT", "rate_usd": 1.0, "updated_at_utc_ms": 1_000},
                {"boundary": "closing", "currency": "USD", "rate_usd": 1.0, "updated_at_utc_ms": 3_000},
                {"boundary": "closing", "currency": "USDT", "rate_usd": 1.0, "updated_at_utc_ms": 3_000},
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

        result = reconcile_independent_execution_fills(
            pm_trades, independent, self.pm_source_coverage(pm_trades)
        )

        self.assertFalse(result["independent_execution_ledger_reconciled"])
        self.assertEqual(result["mismatch_count"], 2)
        by_order = {row["order_id"]: row["reasons"] for row in result["mismatches"]}
        self.assertIn("fee_amount_mismatch", by_order["order-amount"])
        self.assertIn("fee_currency_mismatch", by_order["order-currency"])

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
            "contribution_id"
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
                    "/positions/portfolio-evidence-intervals",
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
                        if route == "/positions/portfolio-evidence-intervals"
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
