use super::*;
#[derive(Clone)]
pub(super) struct FeedClient {
    pub http: reqwest::Client,
    pub base: String,
    selectors: Arc<Mutex<BTreeMap<String, String>>>,
}
#[derive(Clone, Deserialize)]
pub(super) struct Raw {
    pub provider: String,
    pub requested_at_ms: u64,
    pub received_at_ms: u64,
    pub data: Value,
}
impl Raw {
    fn validate(&self, max_age: u64) -> anyhow::Result<()> {
        let now = now_ms();
        anyhow::ensure!(
            self.requested_at_ms <= self.received_at_ms
                && self.received_at_ms <= now
                && now - self.requested_at_ms <= max_age,
            "{} feed stale or future-dated",
            self.provider
        );
        Ok(())
    }
}
impl FeedClient {
    pub fn new(base: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("Insights feed client"),
            base,
            selectors: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
    pub async fn get(
        &self,
        path: &str,
        params: &[(&str, String)],
        max_age: u64,
    ) -> anyhow::Result<Raw> {
        let raw = self
            .http
            .get(format!("{}{path}", self.base))
            .query(params)
            .send()
            .await?
            .error_for_status()?
            .json::<Raw>()
            .await?;
        raw.validate(max_age)?;
        Ok(raw)
    }
    pub async fn post(&self, path: &str, payload: Value) -> anyhow::Result<Raw> {
        let raw = self
            .http
            .post(format!("{}{path}", self.base))
            .json(&payload)
            .send()
            .await?
            .error_for_status()?
            .json::<Raw>()
            .await?;
        raw.validate(30_000)?;
        Ok(raw)
    }
    pub async fn binance(
        &self,
        endpoint: &str,
        symbol: &str,
        extra: &[(&str, String)],
    ) -> anyhow::Result<Raw> {
        let mut params = vec![("endpoint", endpoint.into()), ("symbol", symbol.into())];
        params.extend_from_slice(extra);
        self.get(
            "/market-data/alternative/binance",
            &params,
            if endpoint == "depth" { 30_000 } else { 90_000 },
        )
        .await
    }
    pub async fn deribit(&self, method: &str, extra: &[(&str, String)]) -> anyhow::Result<Raw> {
        let mut params = vec![("method", method.into())];
        params.extend_from_slice(extra);
        let mut raw = self
            .get(
                "/market-data/alternative/deribit",
                &params,
                if method == "get_instruments" || method == "get_instrument" {
                    360_000
                } else {
                    60_000
                },
            )
            .await?;
        anyhow::ensure!(
            raw.data["error"].is_null(),
            "Deribit error: {}",
            raw.data["error"]
        );
        raw.data = raw
            .data
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Deribit result missing"))?;
        Ok(raw)
    }
    pub async fn rpc(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let raw = self
            .post(
                "/market-data/alternative/ethereum",
                json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
            )
            .await?;
        anyhow::ensure!(
            raw.data["error"].is_null(),
            "Ethereum {method}: {}",
            raw.data["error"]
        );
        raw.data
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("RPC result missing for {method}"))
    }
    pub async fn batch(
        &self,
        calls: Vec<(&str, Value)>,
    ) -> anyhow::Result<Vec<Result<Value, String>>> {
        if calls.is_empty() {
            return Ok(Vec::new());
        }
        anyhow::ensure!(calls.len() <= 100, "RPC batch too large");
        let payload=calls.iter().enumerate().map(|(i,(method,params))|json!({"jsonrpc":"2.0","id":i+1,"method":method,"params":params})).collect::<Vec<_>>();
        let raw = self
            .post("/market-data/alternative/ethereum", json!(payload))
            .await?;
        let rows = raw
            .data
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("RPC batch array missing"))?;
        anyhow::ensure!(rows.len() == calls.len(), "Incomplete RPC batch");
        let mut ordered = BTreeMap::new();
        for row in rows {
            let id = row["id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("RPC batch id missing"))?
                as usize;
            anyhow::ensure!(
                id > 0 && id <= calls.len() && !ordered.contains_key(&id),
                "Duplicate/out-of-range RPC id"
            );
            let item = if row["error"].is_null() {
                row.get("result").cloned().ok_or("Result missing".into())
            } else {
                Err(row["error"].to_string())
            };
            ordered.insert(id, item);
        }
        Ok(ordered.into_values().collect())
    }
    pub async fn hash(&self, hex: &str) -> anyhow::Result<String> {
        let v = self.rpc("web3_sha3", json!([hex])).await?;
        let h = v
            .as_str()
            .filter(|s| s.len() == 66)
            .ok_or_else(|| anyhow::anyhow!("Invalid Keccak result"))?;
        Ok(h.into())
    }
    pub async fn selector(&self, signature: &str) -> anyhow::Result<String> {
        if let Some(v) = self.selectors.lock().await.get(signature).cloned() {
            return Ok(v);
        }
        let hex = format!(
            "0x{}",
            signature
                .bytes()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let hash = self.hash(&hex).await?;
        let result = hash[2..10].to_owned();
        self.selectors
            .lock()
            .await
            .insert(signature.into(), result.clone());
        Ok(result)
    }
    pub async fn call(
        &self,
        address: &str,
        signature: &str,
        args: &[String],
        block: &str,
    ) -> anyhow::Result<Vec<String>> {
        let data = format!("0x{}{}", self.selector(signature).await?, args.join(""));
        let result = self
            .rpc(
                "eth_call",
                json!([{"to":address,"data":data,"gas":"0x1e8480"},block]),
            )
            .await?;
        words(
            result
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Contract result not hex"))?,
        )
    }
}
pub(super) fn words(hex: &str) -> anyhow::Result<Vec<String>> {
    let s = hex
        .strip_prefix("0x")
        .ok_or_else(|| anyhow::anyhow!("ABI result lacks 0x"))?;
    anyhow::ensure!(
        !s.is_empty() && s.len() % 64 == 0 && s.bytes().all(|b| b.is_ascii_hexdigit()),
        "Malformed ABI words"
    );
    Ok((0..s.len())
        .step_by(64)
        .map(|i| s[i..i + 64].to_owned())
        .collect())
}
pub(super) fn addr_word(a: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        a.len() == 42 && a.starts_with("0x") && a[2..].bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid Ethereum address"
    );
    Ok(format!(
        "{:0>64}",
        a.trim_start_matches("0x").to_lowercase()
    ))
}
pub(super) fn uint(n: u128) -> String {
    format!("{n:064x}")
}
pub(super) fn hex_u64(s: &str) -> anyhow::Result<u64> {
    Ok(u64::from_str_radix(s.trim_start_matches("0x"), 16)?)
}
pub(super) fn field<'a>(w: &'a [String], i: usize) -> anyhow::Result<&'a str> {
    w.get(i)
        .map(String::as_str)
        .ok_or_else(|| anyhow::anyhow!("ABI word {i} missing"))
}
pub(super) fn number(w: &str) -> anyhow::Result<f64> {
    Ok(Word::parse(w)?.float())
}
pub(super) fn word_address(w: &str) -> anyhow::Result<String> {
    let v = Word::parse(w)?.hex();
    Ok(format!("0x{}", &v[24..]))
}
pub(super) fn signed(n: i64) -> String {
    if n >= 0 {
        uint(n as u128)
    } else {
        Word::default().sub(Word::from_u64(n.unsigned_abs())).hex()
    }
}
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct Word([u64; 4]);
impl Word {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let s = s.trim_start_matches("0x");
        anyhow::ensure!(
            s.len() <= 64 && !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid U256 word"
        );
        let s = format!("{s:0>64}");
        let mut out = [0; 4];
        for (i, item) in out.iter_mut().enumerate() {
            *item = u64::from_str_radix(&s[48 - i * 16..64 - i * 16], 16)?;
        }
        Ok(Self(out))
    }
    pub fn from_u64(n: u64) -> Self {
        Self([n, 0, 0, 0])
    }
    pub fn sub(self, rhs: Self) -> Self {
        let mut out = [0; 4];
        let mut borrow = false;
        for (i, item) in out.iter_mut().enumerate() {
            let (v, b1) = self.0[i].overflowing_sub(rhs.0[i]);
            let (v, b2) = v.overflowing_sub(u64::from(borrow));
            *item = v;
            borrow = b1 || b2;
        }
        Self(out)
    }
    pub fn float(self) -> f64 {
        self.0
            .iter()
            .rev()
            .fold(0.0, |a, n| a * 18446744073709551616.0 + *n as f64)
    }
    pub fn hex(self) -> String {
        self.0.iter().rev().map(|n| format!("{n:016x}")).collect()
    }
    pub fn ge(self, rhs: Self) -> bool {
        self.0.iter().rev().cmp(rhs.0.iter().rev()) != std::cmp::Ordering::Less
    }
    pub fn bit(self, i: usize) -> bool {
        i < 256 && self.0[i / 64] & (1u64 << (i % 64)) != 0
    }
    pub fn i24(self) -> i32 {
        let v = (self.0[0] & 0xffffff) as i32;
        if v & 0x800000 != 0 { v - 0x1000000 } else { v }
    }
}
pub(super) fn levels(v: &Value, side: &str) -> anyhow::Result<Vec<(f64, f64)>> {
    let rows = v[side]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Book {side} missing"))?;
    let mut result = Vec::new();
    for row in rows {
        let p = row[0]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| row[0].as_f64())
            .ok_or_else(|| anyhow::anyhow!("Invalid book price"))?;
        let q = row[1]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| row[1].as_f64())
            .ok_or_else(|| anyhow::anyhow!("Invalid book size"))?;
        anyhow::ensure!(
            p.is_finite() && p > 0.0 && q.is_finite() && q >= 0.0,
            "Nonfinite/nonpositive book"
        );
        result.push((p, q));
    }
    Ok(result)
}
pub(super) fn walk(levels: &[(f64, f64)], quantity: f64) -> Option<f64> {
    if !quantity.is_finite() || quantity <= 0.0 {
        return None;
    }
    let mut left = quantity;
    let mut value = 0.0;
    for &(p, q) in levels {
        let taken = left.min(q);
        value += taken * p;
        left -= taken;
        if left <= 1e-10 {
            return Some(value / quantity);
        }
    }
    None
}
pub(super) fn quote(book: &Value, quantity: f64, time: u64) -> anyhow::Result<Value> {
    let bids = levels(book, "bids")?;
    let asks = levels(book, "asks")?;
    let bid = walk(&bids, quantity)
        .ok_or_else(|| anyhow::anyhow!("Insufficient executable bid depth"))?;
    let ask = walk(&asks, quantity)
        .ok_or_else(|| anyhow::anyhow!("Insufficient executable ask depth"))?;
    anyhow::ensure!(bid <= ask, "Crossed book");
    Ok(
        json!({"bid":bid,"ask":ask,"bid_quantity":quantity,"ask_quantity":quantity,"timestamp_ms":time}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn u256_wrap_signed_ticks_and_inside_growth_are_exact() {
        let max = Word::default().sub(Word::from_u64(1));
        assert_eq!(max.hex(), "f".repeat(64));
        assert_eq!(Word::parse(&signed(-15)).unwrap().i24(), -15);
        assert_eq!(Word::from_u64(3).sub(max).float(), 4.0);
        assert!(Word::from_u64(8).bit(3));
        let a = Word::parse("7000000000000898e").unwrap();
        let b = Word::parse("7000000000000898f").unwrap();
        assert!(b.ge(a));
        assert!(!a.ge(b));
    }
    #[test]
    fn depth_walk_does_not_invent_liquidity() {
        assert_eq!(walk(&[(100.0, 1.0), (102.0, 1.0)], 2.0), Some(101.0));
        assert_eq!(walk(&[(100.0, 1.0)], 2.0), None);
    }

    #[test]
    fn stale_cached_acquisition_and_future_payloads_fail_closed() {
        let time = now_ms();
        let mut raw = Raw {
            provider: "test".into(),
            requested_at_ms: time - 60_000,
            received_at_ms: time,
            data: Value::Null,
        };
        assert!(raw.validate(30_000).is_err());
        raw.requested_at_ms = time + 60_000;
        raw.received_at_ms = time + 60_000;
        assert!(raw.validate(30_000).is_err());
        raw.requested_at_ms = time;
        raw.received_at_ms = time;
        assert!(raw.validate(30_000).is_ok());
    }
}
