import unittest

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


if __name__ == "__main__":
    unittest.main()
