#[derive(Deserialize)]

struct TickerMessage<'a> {
    #[serde(borrow)]
    data: Option<Vec<Ticker<'a>>>,
    event: Option<&'a str>,
    msg: Option<&'a str>,
}

#[derive(Deserialize)]
struct Ticker<'a> {
    #[serde(rename = "instId", borrow)]
    instrument: &'a str,
    #[serde(borrow)]
    last: &'a str,
    #[serde(borrow)]
    ts: Option<&'a str>,
}

#[derive(Deserialize)]
struct BookMessage<'a> {
    #[serde(borrow)]
    arg: Option<BookArg<'a>>,
    #[serde(borrow)]
    data: Option<Vec<BookUpdate<'a>>>,
    event: Option<&'a str>,
    msg: Option<&'a str>,
    action: Option<&'a str>,
}

#[derive(Deserialize)]
struct BookArg<'a> {
    #[serde(rename = "instId", borrow)]
    instrument: &'a str,
}

#[derive(Deserialize)]
struct BookUpdate<'a> {
    #[serde(borrow)]
    asks: Vec<Vec<&'a str>>,
    #[serde(borrow)]
    bids: Vec<Vec<&'a str>>,
    #[serde(borrow)]
    ts: &'a str,
}

const BOOK_DEPTH: usize = 7;

#[derive(Deserialize)]
struct TradeMessage<'a> {
    #[serde(borrow)]
    data: Option<Vec<Trade<'a>>>,
    event: Option<&'a str>,
    msg: Option<&'a str>,
}

#[derive(Deserialize)]
struct Trade<'a> {
    #[serde(rename = "instId", borrow)]
    instrument: &'a str,
    #[serde(rename = "tradeId", borrow)]
    trade_id: &'a str,
    #[serde(borrow)]
    px: &'a str,
    #[serde(borrow)]
    sz: &'a str,
    #[serde(borrow)]
    side: &'a str,
    #[serde(borrow)]
    ts: &'a str,
}

#[derive(Clone, Debug)]
pub struct TickerSample {
    pub instrument: String,
    pub price: f64,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UsdRateSnapshot {
    pub rates: BTreeMap<String, f64>,
    pub updated_at: u64,
    pub updated_at_by_currency: BTreeMap<String, u64>,
}

#[derive(Deserialize)]
struct IndexTickerRestResponse { data: Vec<IndexTickerRestItem> }

#[derive(Deserialize)]
struct IndexTickerRestItem {
    #[serde(rename = "instId")] instrument: String,
    #[serde(rename = "idxPx")] index_price: String,
    ts: String,
}

#[derive(Deserialize)]
struct IndexTickerWsMessage {
    event: Option<String>,
    msg: Option<String>,
    data: Option<Vec<IndexTickerWsItem>>,
}

#[derive(Deserialize)]
struct IndexTickerWsItem {
    #[serde(rename = "instId")] instrument: String,
    #[serde(rename = "idxPx")] index_price: String,
    ts: String,
}

#[derive(Clone, Debug)]
struct UsdIndexUpdate { currency: String, rate: f64, timestamp: u64 }

