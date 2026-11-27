"""Risk-adjusted metrics for complete, fixed-cadence net valuation returns.

Input returns are simple interval returns after all attributable costs and
divided by the documented capital base for that interval. This module refuses
to turn trade outcomes or incomplete/irregular valuations into a time series.
"""

from __future__ import annotations

import math

SECONDS_PER_YEAR = 365.2425 * 24 * 60 * 60
GROUP_DIMENSIONS = (
    "population", "portfolio", "exchange", "strategy", "symbol",
    "profile_version", "direction", "horizon", "entry_mode",
)
REQUIRED = (
    "interval_start_ms", "interval_end_ms", "net_return",
    "attributed_capital", "complete", "population", "capital_basis",
    "currency", "return_basis", "authoritative",
)


def _withheld(reason: str, *, count: int = 0, duration_days: float = 0.0):
    return {
        "status": "withheld", "reason": reason, "sample_count": count,
        "observation_duration_days": duration_days,
    }


def calculate_risk_metrics(
    rows,
    *,
    annual_risk_free_rate=0.0,
    rolling_days=(7, 30, 90),
    confidence_level=0.95,
):
    """Calculate metrics only for one complete, regular, attributable series.

    Annualisation uses 365.2425 calendar days divided by the observed fixed
    interval. Volatility and Sharpe use a Newey-West long-run variance with
    Bartlett weights and at most min(30, floor(sqrt(n))) lags. The confidence
    interval is a normal approximation for annualised excess arithmetic return
    using that HAC variance; it is withheld below 30 observations. Ratios are
    descriptive and do not constitute promotion criteria.
    """
    rows = list(rows)
    if not rows:
        return _withheld("no valuation intervals")
    for i, row in enumerate(rows):
        missing = [key for key in REQUIRED if key not in row or row[key] is None]
        if missing:
            return _withheld(f"row {i} missing required fields: {', '.join(missing)}", count=len(rows))
    if any(not row["complete"] for row in rows):
        return _withheld("one or more valuation intervals are incomplete", count=len(rows))
    if any(row["authoritative"] is not True for row in rows):
        return _withheld("return series is not marked authoritative", count=len(rows))
    if {row["return_basis"] for row in rows} != {"net_of_costs"}:
        return _withheld("return series is not consistently net of costs", count=len(rows))
    populations = {row["population"] for row in rows}
    if len(populations) != 1:
        return _withheld("AutoTrader and Scout populations cannot be combined", count=len(rows))
    if populations - {"autotrader", "scout"}:
        return _withheld("unsupported or unspecified population", count=len(rows))

    ordered = sorted(rows, key=lambda row: row["interval_start_ms"])
    try:
        returns = [float(row["net_return"]) for row in ordered]
        capital = [float(row["attributed_capital"]) for row in ordered]
        starts = [int(row["interval_start_ms"]) for row in ordered]
        ends = [int(row["interval_end_ms"]) for row in ordered]
        risk_free = float(annual_risk_free_rate)
    except (TypeError, ValueError, OverflowError):
        return _withheld("invalid numeric or timestamp value", count=len(rows))
    if not math.isfinite(risk_free) or risk_free <= -1:
        return _withheld("invalid annual risk-free rate", count=len(rows))
    if any(not math.isfinite(value) for value in returns + capital):
        return _withheld("non-finite return or capital value", count=len(rows))
    if any(value <= 0 for value in capital):
        return _withheld("capital attribution must be positive for every interval", count=len(rows))
    if any(end <= start for start, end in zip(starts, ends)):
        return _withheld("invalid interval bounds", count=len(rows))
    durations = [end - start for start, end in zip(starts, ends)]
    cadence = durations[0]
    if any(duration != cadence for duration in durations):
        return _withheld("irregular valuation interval duration", count=len(rows))
    if any(left_end != right_start for left_end, right_start in zip(ends, starts[1:])):
        return _withheld("missing, overlapping, or discontinuous valuation intervals", count=len(rows))
    if len({(row.get("currency"), row.get("capital_basis")) for row in ordered}) != 1:
        return _withheld("currency or capital attribution basis changed", count=len(rows))
    if any(value <= -1 for value in returns):
        return _withheld("net return at or below -100% is invalid", count=len(rows))

    n = len(returns)
    duration_days = (ends[-1] - starts[0]) / 86_400_000
    if n < 2:
        return _withheld(
            "at least two complete intervals are required",
            count=n,
            duration_days=duration_days,
        )
    periods_per_year = SECONDS_PER_YEAR * 1000 / cadence
    rf_period = (1.0 + risk_free) ** (1.0 / periods_per_year) - 1.0
    excess = [value - rf_period for value in returns]
    mean_excess = sum(excess) / n
    hac_var = _newey_west_variance(excess)
    annual_vol = math.sqrt(hac_var * periods_per_year) if hac_var > 0 else None
    sharpe = (mean_excess * math.sqrt(periods_per_year / hac_var)
              if hac_var > 0 else None)

    downside = [min(value, 0.0) for value in excess]
    downside_var = _newey_west_variance(downside, center=0.0)
    downside_dev = math.sqrt(downside_var * periods_per_year) if downside_var > 0 else 0.0
    sortino = mean_excess * periods_per_year / downside_dev if downside_dev > 0 else None

    wealth = 1.0
    peak = 1.0
    max_drawdown = 0.0
    for value in returns:
        wealth *= 1.0 + value
        peak = max(peak, wealth)
        max_drawdown = max(max_drawdown, (peak - wealth) / peak)
    years = (ends[-1] - starts[0]) / (SECONDS_PER_YEAR * 1000)
    cagr = wealth ** (1.0 / years) - 1.0 if years > 0 and wealth > 0 else None
    calmar = cagr / max_drawdown if cagr is not None and max_drawdown > 0 else None

    confidence = None
    if n >= 30 and hac_var >= 0:
        from statistics import NormalDist
        z = NormalDist().inv_cdf(0.5 + confidence_level / 2)
        annual_mean = mean_excess * periods_per_year
        standard_error = math.sqrt(hac_var / n) * periods_per_year
        confidence = {
            "level": confidence_level,
            "method": "Newey-West HAC normal interval for annualised excess arithmetic return",
            "annualised_excess_return_low": annual_mean - z * standard_error,
            "annualised_excess_return_high": annual_mean + z * standard_error,
        }

    result = {
        "status": "available" if n >= 2 else "insufficient_evidence",
        "sample_count": n,
        "observation_duration_days": duration_days,
        "evidence_coverage": 1.0,
        "net_return_basis": "interval net return / attributed capital; costs already included",
        "capital_attribution": ordered[0].get("capital_basis", "unspecified"),
        "annual_risk_free_rate": risk_free,
        "annualisation": {
            "calendar_days_per_year": 365.2425,
            "interval_seconds": cadence / 1000,
            "periods_per_year": periods_per_year,
            "serial_correlation": "Newey-West HAC, Bartlett weights, min(30, floor(sqrt(n))) lags",
        },
        "annualised_sharpe": sharpe,
        "sortino": sortino,
        "calmar": calmar,
        "maximum_drawdown": max_drawdown,
        "annualised_volatility": annual_vol,
        "annualised_downside_deviation": downside_dev,
        "confidence": confidence,
        "confidence_unavailable_reason": None if confidence else "fewer than 30 complete observations",
        "withheld_metrics": [
            name for name, value in (("sharpe", sharpe), ("sortino", sortino), ("calmar", calmar))
            if value is None
        ],
        "grouping": {key: ordered[0].get(key) for key in GROUP_DIMENSIONS},
    }
    result["rolling"] = {
        days: _rolling_metrics(ordered, days, annual_risk_free_rate)
        for days in rolling_days
    }
    if n < 2:
        result["status"] = "insufficient_evidence"
        result["reason"] = "at least two complete intervals are required"
    return result


def aggregate_series(rows, group_by):
    """Group one input population without combining AutoTrader and Scout."""
    rows = list(rows)
    if "population" not in group_by:
        raise ValueError("group_by must include population")
    unsupported = set(group_by) - set(GROUP_DIMENSIONS)
    if unsupported:
        raise ValueError(f"unsupported grouping dimensions: {sorted(unsupported)}")
    groups = {}
    for row in rows:
        missing = [key for key in group_by if row.get(key) is None]
        if missing:
            raise ValueError(f"row missing grouping dimensions: {', '.join(missing)}")
        key = tuple(row[key] for key in group_by)
        groups.setdefault(key, []).append(row)
    return [
        {
            "group": dict(zip(group_by, key)),
            **calculate_risk_metrics(group_rows),
        }
        for key, group_rows in sorted(groups.items(), key=lambda item: tuple(map(str, item[0])))
    ]


def _newey_west_variance(values, center=None):
    n = len(values)
    if n < 2:
        return 0.0
    mean = sum(values) / n if center is None else center
    residuals = [value - mean for value in values]
    gamma0 = sum(value * value for value in residuals) / n
    lags = min(30, int(math.sqrt(n)))
    variance = gamma0
    for lag in range(1, lags + 1):
        covariance = sum(residuals[i] * residuals[i - lag] for i in range(lag, n)) / n
        variance += 2.0 * (1.0 - lag / (lags + 1)) * covariance
    return max(variance, 0.0)


def _rolling_metrics(rows, days, annual_risk_free_rate):
    end = rows[-1]["interval_end_ms"]
    start = end - days * 86_400_000
    selected = [row for row in rows if row["interval_start_ms"] >= start]
    if not selected or selected[0]["interval_start_ms"] != start or selected[-1]["interval_end_ms"] != end:
        return _withheld("window lacks complete boundary-aligned valuation coverage")
    metrics = calculate_risk_metrics(
        selected, annual_risk_free_rate=annual_risk_free_rate, rolling_days=()
    )
    return metrics
