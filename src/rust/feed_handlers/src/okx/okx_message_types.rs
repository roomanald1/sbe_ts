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

struct BookLevel<'a> {
    price: &'a str,
    size: &'a str,
}

impl<'de: 'a, 'a> Deserialize<'de> for BookLevel<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct BookLevelVisitor;
        impl<'de> serde::de::Visitor<'de> for BookLevelVisitor {
            type Value = BookLevel<'de>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an OKX book level containing price and size")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let price = seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let size = seq.next_element()?.ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(BookLevel { price, size })
            }
        }
        deserializer.deserialize_seq(BookLevelVisitor)
    }
}

#[derive(Deserialize)]
struct BookUpdate<'a> {
    #[serde(borrow)]
    asks: Vec<BookLevel<'a>>,
    #[serde(borrow)]
    bids: Vec<BookLevel<'a>>,
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

