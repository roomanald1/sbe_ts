use super::*;
// Lagged regressions: no return estimates its own alpha/beta.
pub(super) fn scores(assets: &[Value]) -> anyhow::Result<BTreeMap<String, f64>> {
    anyhow::ensure!(
        (4..=32).contains(&assets.len()),
        "Need 4–32 synchronized histories"
    );
    let mut histories = Vec::new();
    let mut anchor = None;
    let mut symbols = BTreeSet::new();
    for asset in assets {
        let name = asset["symbol"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Symbol missing"))?;
        anyhow::ensure!(symbols.insert(name), "Duplicate residual symbol");
        let bars = asset["closes"]
            .as_array()
            .filter(|v| v.len() == 45)
            .ok_or_else(|| anyhow::anyhow!("Need exactly 45 closes"))?;
        let times = bars
            .iter()
            .map(|b| {
                b["timestamp_ms"]
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("Close time missing"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        anyhow::ensure!(
            times
                .windows(2)
                .all(|t| t[1].checked_sub(t[0]) == Some(DAY)),
            "Daily history has gaps"
        );
        if let Some(previous) = &anchor {
            anyhow::ensure!(previous == &times, "Daily histories differ");
        } else {
            anchor = Some(times);
        }
        let closes = bars
            .iter()
            .map(|b| {
                b["close"]
                    .as_f64()
                    .filter(|p| p.is_finite() && *p > 0.0)
                    .ok_or_else(|| anyhow::anyhow!("Invalid close"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        histories.push((
            name,
            closes
                .windows(2)
                .map(|p| p[1] / p[0] - 1.0)
                .collect::<Vec<_>>(),
        ));
    }
    let market = (0..44)
        .map(|i| histories.iter().map(|(_, r)| r[i]).sum::<f64>() / histories.len() as f64)
        .collect::<Vec<_>>();
    let mut result = BTreeMap::new();
    for (name, returns) in histories {
        let mut score = 0.0;
        for t in 30..44 {
            let mx = market[t - 30..t].iter().sum::<f64>() / 30.0;
            let my = returns[t - 30..t].iter().sum::<f64>() / 30.0;
            let variance = market[t - 30..t]
                .iter()
                .map(|v| (v - mx).powi(2))
                .sum::<f64>();
            anyhow::ensure!(variance > 1e-16, "Market factor variance is zero");
            let beta = (t - 30..t)
                .map(|j| (market[j] - mx) * (returns[j] - my))
                .sum::<f64>()
                / variance;
            score += returns[t] - (my - beta * mx) - beta * market[t];
        }
        anyhow::ensure!(score.is_finite(), "Invalid residual factor");
        result.insert(name.to_owned(), score);
    }
    Ok(result)
}
