import unittest

from risk_adjusted_performance import aggregate_series, calculate_risk_metrics


DAY = 86_400_000


def series(returns, *, cadence=DAY, start=0, capital=1000, **overrides):
    rows = []
    for i, value in enumerate(returns):
        row = {
            "interval_start_ms": start + i * cadence,
            "interval_end_ms": start + (i + 1) * cadence,
            "net_return": value,
            "return_basis": "net_of_costs",
            "authoritative": True,
            "attributed_capital": capital if not isinstance(capital, list) else capital[i],
            "capital_basis": "time_weighted_opening_equity_usd",
            "currency": "USD",
            "complete": True,
            "population": "autotrader",
            "portfolio": "paper-main",
            "exchange": "paper",
            "strategy": "strategy-a",
            "symbol": "BTC-USDT",
            "profile_version": "profile-a:v1",
            "direction": "long",
            "horizon": "intraday",
            "entry_mode": "strict",
        }
        row.update({key: val[i] if isinstance(val, list) else val for key, val in overrides.items()})
        rows.append(row)
    return rows


class RiskAdjustedPerformanceTests(unittest.TestCase):
    def test_fixed_interval_metrics_have_expected_drawdown_and_annualisation(self):
        result = calculate_risk_metrics(series([0.01, -0.02, 0.01, 0.01]))
        self.assertEqual(result["status"], "available")
        self.assertAlmostEqual(result["maximum_drawdown"], 0.02)
        self.assertAlmostEqual(result["annualisation"]["periods_per_year"], 365.2425)
        self.assertIsNotNone(result["annualised_volatility"])
        self.assertIsNotNone(result["annualised_sharpe"])
        self.assertIsNotNone(result["sortino"])
        self.assertIsNotNone(result["calmar"])
        self.assertEqual(result["sample_count"], 4)
        self.assertEqual(result["evidence_coverage"], 1.0)

    def test_metrics_use_documented_net_returns_and_risk_free_rate(self):
        result = calculate_risk_metrics(
            series([0.01, -0.01]), annual_risk_free_rate=0.0
        )
        # HAC long-run variance for two alternating observations is 0.00005.
        expected_volatility = (0.00005 * 365.2425) ** 0.5
        self.assertAlmostEqual(result["annualised_volatility"], expected_volatility)
        self.assertAlmostEqual(result["annualised_sharpe"], 0.0)
        self.assertAlmostEqual(result["maximum_drawdown"], (1.01 - 0.9999) / 1.01)
        risk_free = calculate_risk_metrics(
            series([0.001, 0.001, 0.001]), annual_risk_free_rate=0.0365
        )
        self.assertEqual(risk_free["annual_risk_free_rate"], 0.0365)
        self.assertEqual(risk_free["net_return_basis"].split(";")[0], "interval net return / attributed capital")

    def test_missing_and_incomplete_intervals_are_withheld_without_zero_fill(self):
        missing = series([0.01, 0.01, 0.01])
        missing[1]["interval_start_ms"] += 1
        missing[1]["interval_end_ms"] += 1
        result = calculate_risk_metrics(missing)
        self.assertEqual(result["status"], "withheld")
        self.assertIn("discontinuous", result["reason"])
        incomplete = series([0.01, 0.0, 0.01])
        incomplete[1]["complete"] = False
        result = calculate_risk_metrics(incomplete)
        self.assertEqual(result["status"], "withheld")
        self.assertIn("incomplete", result["reason"])

    def test_non_authoritative_or_gross_return_series_is_withheld(self):
        rows = series([0.01, -0.01])
        rows[0]["authoritative"] = False
        self.assertIn("not marked authoritative", calculate_risk_metrics(rows)["reason"])
        rows = series([0.01, -0.01])
        rows[1]["return_basis"] = "gross"
        self.assertIn("net of costs", calculate_risk_metrics(rows)["reason"])

    def test_irregular_sampling_is_rejected(self):
        rows = series([0.01, -0.01, 0.02])
        rows[1]["interval_end_ms"] += 1000
        rows[2]["interval_start_ms"] += 1000
        rows[2]["interval_end_ms"] += 1000
        result = calculate_risk_metrics(rows)
        self.assertEqual(result["status"], "withheld")
        self.assertIn("irregular", result["reason"])

    def test_non_daily_annualisation_uses_observed_cadence(self):
        result = calculate_risk_metrics(series([0.001] * 4, cadence=DAY // 4))
        self.assertAlmostEqual(result["annualisation"]["periods_per_year"], 4 * 365.2425)

    def test_zero_volatility_withholds_undefined_ratios(self):
        result = calculate_risk_metrics(series([0.0] * 4))
        self.assertIsNone(result["annualised_sharpe"])
        self.assertIsNone(result["sortino"])
        self.assertIsNone(result["annualised_volatility"])
        self.assertIn("sharpe", result["withheld_metrics"])

    def test_capital_attribution_basis_and_values_are_required(self):
        rows = series([0.01, -0.005, 0.002], capital=[1000, 1200, 900])
        result = calculate_risk_metrics(rows)
        self.assertEqual(result["status"], "available")
        self.assertEqual(result["capital_attribution"], "time_weighted_opening_equity_usd")
        rows[1]["attributed_capital"] = 0
        self.assertIn("capital attribution", calculate_risk_metrics(rows)["reason"])
        rows = series([0.01, -0.005])
        rows[0].pop("capital_basis")
        self.assertIn("capital_basis", calculate_risk_metrics(rows)["reason"])

    def test_scout_and_autotrader_never_combine(self):
        rows = series([0.01, -0.01])
        rows += series([0.02, -0.02], start=2 * DAY, population="scout")
        result = calculate_risk_metrics(rows)
        self.assertEqual(result["status"], "withheld")
        groups = aggregate_series(rows, ["population", "strategy"])
        self.assertEqual({g["group"]["population"] for g in groups}, {"autotrader", "scout"})

    def test_confidence_is_reported_only_with_supported_sample_count(self):
        short = calculate_risk_metrics(series([0.001, -0.001, 0.002]))
        self.assertIsNone(short["confidence"])
        long = calculate_risk_metrics(series([0.001 if i % 2 else -0.0005 for i in range(40)]))
        self.assertEqual(long["confidence"]["level"], 0.95)
        self.assertIn("HAC", long["confidence"]["method"])

    def test_rolling_windows_withhold_when_window_coverage_is_missing(self):
        result = calculate_risk_metrics(series([0.001] * 10))
        self.assertEqual(result["rolling"][7]["status"], "available")
        self.assertEqual(result["rolling"][30]["status"], "withheld")
        self.assertEqual(result["rolling"][90]["status"], "withheld")


if __name__ == "__main__":
    unittest.main()
