mod daily_summary;
mod feed_client;
mod logging;
use axum::{Json, Router, http::StatusCode, routing::get};
use dashmap::DashMap;
use futures_util::StreamExt;
use insights::signals::{
    ExternalMarketFeatures, SignalEngine, SignalSnapshot, SignalStore, StoredSignalRecord,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{sync::{watch, Mutex, Notify}, time::{Duration, Instant}};

#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SignalFeedEvent {
    Price {
        timestamp: u64,
        price: f64,
        volatility_1m: f64,
    },
    Trade {
        timestamp: u64,
        price: f64,
        size: f64,
        is_buy: bool,
    },
    Book {
        timestamp: u64,
        best_bid: f64,
        best_ask: f64,
        bid_sizes: [f64; 7],
        ask_sizes: [f64; 7],
        #[serde(default)]
        bid_levels: [(f64, f64); 7],
        #[serde(default)]
        ask_levels: [(f64, f64); 7],
    },
}
#[derive(Deserialize)]
struct SignalEventRequest {
    symbol: String,
    #[serde(flatten)]
    event: SignalFeedEvent,
}
struct SignalRuntime {
    store: SignalStore,
    engines: Mutex<HashMap<String, SignalEngine>>,
    signal_push_cache: Arc<DashMap<String, serde_json::Value>>,
    signal_push_notify: Arc<Notify>,
    ui_snapshot_cache: Arc<DashMap<String, serde_json::Value>>,
    ui_snapshot_notify: Arc<Notify>,
    news: watch::Receiver<Arc<NewsFeedSnapshot>>,
    news_learning: Mutex<NewsLearning>,
    snapshot_diagnostic_at: DashMap<String, u64>,
}

#[derive(Default)]
struct NewsLearning {
    samples: usize,
    correct: usize,
    weight: f64,
    pending: HashMap<(String, u64), f64>,
}

#[derive(Clone, Serialize)]
struct NewsArticle {
    title: String,
    article_text: String,
    url: String,
    published_at: String,
    received_at_ms: u64,
    matched_asset: String,
    direction: String,
    score: f64,
    signal_reason: String,
    sentiment_model: String,
    sentiment_confidence: f64,
}

#[derive(Clone, Default, Serialize)]
struct NewsFeedSnapshot {
    updated_at_ms: u64,
    source: String,
    stale: bool,
    error: Option<String>,
    articles: Vec<NewsArticle>,
}


static POSITION_MANAGEMENT_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn signal_diagnostic_due(runtime: &SignalRuntime, symbol: &str, timestamp: u64) -> bool {
    let mut last = runtime.snapshot_diagnostic_at.entry(symbol.to_owned()).or_insert(0);
    if timestamp >= *last && timestamp.saturating_sub(*last) < 60_000 {
        return false;
    }
    *last = timestamp;
    true
}

fn decode_xml_text(value: &str) -> String {
    let value = value.replace("<![CDATA[", "").replace("]]>", "");
    let mut output = String::with_capacity(value.len());
    let mut inside_tag = false;
    for character in value.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => output.push(character),
            _ => {}
        }
    }
    output
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .trim()
        .to_owned()
}

fn xml_field(item: &str, field: &str) -> Option<String> {
    let open = format!("<{field}");
    let start = item.find(&open)?;
    let content_start = item[start..].find('>')? + start + 1;
    let close = format!("</{field}>");
    let content_end = item[content_start..].find(&close)? + content_start;
    let value = decode_xml_text(&item[content_start..content_end]);
    (!value.is_empty()).then_some(value)
}

fn news_asset(title: &str) -> String {
    const ASSETS: [(&str, &str); 18] = [
        ("BITCOIN", "BTC"), ("ETHEREUM", "ETH"), ("RIPPLE", "XRP"),
        ("SOLANA", "SOL"), ("BINANCE COIN", "BNB"), ("CARDANO", "ADA"),
        ("DOGECOIN", "DOGE"), ("AVALANCHE", "AVAX"), ("POLKADOT", "DOT"),
        ("CHAINLINK", "LINK"), ("LITECOIN", "LTC"), ("BITCOIN", "BTC"),
        ("BTC", "BTC"), ("ETH", "ETH"), ("XRP", "XRP"), ("SOL", "SOL"),
        ("BNB", "BNB"), ("ADA", "ADA"),
    ];
    let upper = format!(" {} ", title.to_ascii_uppercase().replace(|c: char| !c.is_ascii_alphanumeric(), " "));
    let matched = ASSETS.iter().find_map(|(name, ticker)| {
        let needle = format!(" {} ", name);
        upper.contains(&needle).then(|| (*ticker).to_owned())
    });
    matched.unwrap_or_else(|| {
        // Generic security, policy, and company news must not be treated as
        // crypto market sentiment simply because it came from a crypto RSS feed.
        const CRYPTO_CONTEXT: [&str; 16] = [
            " CRYPTO ", " CRYPTOCURRENCY ", " DIGITAL ASSET ", " DIGITAL ASSETS ",
            " BLOCKCHAIN ", " TOKEN ", " TOKENS ", " STABLECOIN ", " STABLECOINS ",
            " DEFI ", " WEB3 ", " EXCHANGE ", " ON CHAIN ", " ONCHAIN ", " ALTCOIN ", " ALTCOINS ",
        ];
        if CRYPTO_CONTEXT.iter().any(|term| upper.contains(term)) { "CRYPTO".to_owned() }
        else { "GENERAL".to_owned() }
    })
}

fn has_adverse_tax_policy(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    ["TAX", "TAXES", "TAXATION", "LEVY", "DUTY", "DUTIES"]
        .iter().any(|term| upper.contains(term))
        && !["TAX CUT", "TAX CUTS", "LOWER TAX", "LOWER TAXES"]
            .iter().any(|term| upper.contains(term))
}

fn has_market_risk_warning(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    (upper.contains("LEVERAGE") && (upper.contains("RISK") || upper.contains("HEATING")))
        || (upper.contains("VOLATILITY") && upper.contains("RISK"))
        || (upper.contains("CRASH") && (upper.contains("SIMILAR SETUP") || upper.contains("WARNING") || upper.contains("RISK")))
}

fn has_corporate_objection(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    ["OBJECTION", "OBJECTIONS", "OPPOSITION", "PUSHBACK", "RESISTANCE", "OPT OUT", "OPTED OUT", "OPTS OUT", "WITHDRAW FROM LISTING"]
        .iter().any(|term| upper.contains(term))
}

fn has_positive_upgrade_announcement(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    let upgrade = ["UPGRADE", "OVERHAUL", "NETWORK OVERHAUL", "PROTOCOL UPGRADE"]
        .iter().any(|term| upper.contains(term));
    let rollout = ["GOES LIVE", "GO LIVE", "LAUNCHES", "ROLLOUT", "ROLLS OUT", "ACTIVATES", "ACTIVATION"]
        .iter().any(|term| upper.contains(term));
    upgrade && rollout
}

fn has_material_adverse_event(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    ["HACK", "EXPLOIT", "BREACH", "ATTACK", "OUTAGE", "FAILS TO LAUNCH", "POSTPONED", "DELAYED"]
        .iter().any(|term| upper.contains(term))
}

fn classify_news(title: &str, asset: &str, url: String, published_at: String, now: u64) -> NewsArticle {
    const POSITIVE: [&str; 16] = ["APPROVES", "APPROVAL", "ADOPTS", "LAUNCHES", "SURGES", "RALLIES", "GAINS", "INFLOWS", "PARTNERSHIP", "INTEGRATES", "RECORD HIGH", "BREAKS OUT", "GOES LIVE", "GO LIVE", "UPGRADE", "OVERHAUL"];
    const NEGATIVE: [&str; 25] = ["HACK", "EXPLOIT", "BREACH", "LAWSUIT", "SUES", "BANNED", "BAN", "PLUNGES", "FALLS", "DROPS", "OUTFLOWS", "LIQUIDATION", "BANKRUPT", "DELIST", "TAX", "TAXES", "TAXATION", "LEVY", "DUTY", "DUTIES", "OBJECTION", "OPPOSITION", "PUSHBACK", "RESISTANCE", "DELAYED"];
    let upper = title.to_ascii_uppercase();
    let positive = POSITIVE.iter().filter(|term| upper.contains(**term)).count() as f64;
    let negative = NEGATIVE.iter().filter(|term| upper.contains(**term)).count() as f64;
    let adverse_tax_policy = has_adverse_tax_policy(title);
    let market_risk_warning = has_market_risk_warning(title);
    // Risk warnings take precedence over positive price movement wording.
    let score = if asset == "GENERAL" {
        0.0
    } else if adverse_tax_policy || market_risk_warning {
        -1.0
    } else if positive + negative == 0.0 {
        0.0
    } else {
        (positive - negative) / (positive + negative)
    };
    let direction = if score >= 0.5 { "bullish" } else if score <= -0.5 { "bearish" } else { "neutral" };
    let signal_reason = if asset == "GENERAL" {
        "General business or security news; excluded from crypto sentiment".to_owned()
    } else if market_risk_warning {
        "Leverage, volatility, or crash-risk warning overrides positive price wording; heuristic only".to_owned()
    } else if adverse_tax_policy {
        "Adverse tax or levy policy keyword; overrides positive approval wording; heuristic only".to_owned()
    } else if direction == "neutral" {
        "Headline has no clear match in the conservative event keyword list".to_owned()
    } else {
        format!("Headline matched {} event keyword(s); heuristic only", if direction == "bullish" { positive as usize } else { negative as usize })
    };
    NewsArticle { title: title.to_owned(), article_text: String::new(), url, published_at, received_at_ms: now, matched_asset: asset.to_owned(), direction: direction.to_owned(), score, signal_reason, sentiment_model: "keyword_fallback".to_owned(), sentiment_confidence: 0.0 }
}

fn parse_news_rss(xml: &str, now: u64) -> Vec<NewsArticle> {
    let mut articles = Vec::new();
    let mut remaining = xml;
    while let Some(start) = remaining.find("<item") {
        let Some(open_end) = remaining[start..].find('>') else { break; };
        let body_start = start + open_end + 1;
        let Some(end) = remaining[body_start..].find("</item>") else { break; };
        let item = &remaining[body_start..body_start + end];
        let (Some(title), Some(url)) = (xml_field(item, "title"), xml_field(item, "link")) else {
            remaining = &remaining[body_start + end + 7..];
            continue;
        };
        if articles.iter().any(|article: &NewsArticle| article.url == url) {
            remaining = &remaining[body_start + end + 7..];
            continue;
        }
        let published_at = xml_field(item, "pubDate").unwrap_or_default();
        let rss_text = xml_field(item, "description").unwrap_or_default();
        let asset = news_asset(&title);
        let mut article = classify_news(&title, &asset, url, published_at, now);
        article.article_text = rss_text.chars().take(4000).collect();
        articles.push(article);
        if articles.len() >= 50 { break; }
        remaining = &remaining[body_start + end + 7..];
    }
    articles
}


fn apply_news_signal(snapshot: &mut SignalSnapshot, news: &NewsFeedSnapshot, now: u64, weight: f64) -> Option<f64> {
    let asset = snapshot.symbol.split('-').next().unwrap_or(&snapshot.symbol);
    let relevant = news.articles.iter().filter(|article| {
        (article.matched_asset == asset || article.matched_asset == "CRYPTO")
            && article.score != 0.0
            && article.direction != "neutral"
            && now.saturating_sub(article.received_at_ms) <= 30 * 60 * 1000
    }).collect::<Vec<_>>();
    if relevant.is_empty() {
        return None;
    }
    let news_score = relevant.iter().map(|article| article.score).sum::<f64>() / relevant.len() as f64;
    // Headline sentiment is noisy, so its learned contribution is capped at 20%.
    snapshot.score = (snapshot.score * (1.0 - weight) + news_score * weight).clamp(-1.0, 1.0);
    snapshot.direction = if snapshot.score > 0.2 {
        "bullish".to_owned()
    } else if snapshot.score < -0.2 {
        "bearish".to_owned()
    } else {
        "neutral".to_owned()
    };
    snapshot.reasons.push(format!(
        "strategy_vote:news_sentiment;direction={};vote={news_score:.2};headlines={};weight={weight:.3};source=CoinDesk_RSS;context=headline+description;max_age_seconds=1800;classifier=local_crypto_finbert_or_keyword_fallback",
        if news_score > 0.0 { "bullish" } else { "bearish" },
        relevant.len()
    ));
    Some(news_score)
}

async fn classify_headlines_with_model(articles: &mut [NewsArticle]) {
    if articles.is_empty() { return; }
    #[derive(Deserialize)]
    struct Prediction { direction: String, score: f64, confidence: f64 }
    let crypto_indexes = articles.iter().enumerate()
        .filter_map(|(index, article)| (article.matched_asset != "GENERAL").then_some(index))
        .collect::<Vec<_>>();
    if crypto_indexes.is_empty() { return; }
    let input = crypto_indexes.iter().map(|&index| {
        let article = &articles[index];
        serde_json::json!({"title": article.title, "article_text": article.article_text})
    }).collect::<Vec<_>>();
    let endpoint = std::env::var("NEWS_SENTIMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3006/classify".to_owned());
    let client = POSITION_MANAGEMENT_CLIENT.get_or_init(reqwest::Client::new);
    let response = client.post(endpoint).timeout(Duration::from_secs(45)).json(&input).send().await;
    let Ok(response) = response.and_then(reqwest::Response::error_for_status) else { return; };
    let Ok(predictions) = response.json::<Vec<Prediction>>().await else { return; };
    if predictions.len() != crypto_indexes.len() { return; }
    for (index, prediction) in crypto_indexes.into_iter().zip(predictions) {
        let article = &mut articles[index];
        if !prediction.score.is_finite() || !prediction.confidence.is_finite() { continue; }
        article.direction = prediction.direction;
        article.score = prediction.score.clamp(-1.0, 1.0);
        article.sentiment_confidence = prediction.confidence.clamp(0.0, 1.0);
        article.sentiment_model = "finetuned-finbert-crypto".to_owned();
        let text = format!("{} {}", article.title, article.article_text);
        if has_positive_upgrade_announcement(&text) && !has_material_adverse_event(&text) {
            article.direction = "bullish".to_owned();
            article.score = article.score.max(0.5);
            article.signal_reason = format!("Positive network upgrade rollout overrides incidental negative wording · model confidence {:.0}%", article.sentiment_confidence * 100.0);
        } else if has_market_risk_warning(&text) {
            article.direction = "bearish".to_owned();
            article.score = -1.0;
            article.signal_reason = format!("Leverage, volatility, or crash-risk warning overrides positive price wording · model confidence {:.0}%", article.sentiment_confidence * 100.0);
        } else if has_adverse_tax_policy(&text) {
            article.direction = "bearish".to_owned();
            article.score = -1.0;
            article.signal_reason = format!("Adverse tax policy overrides model tone · model confidence {:.0}%", article.sentiment_confidence * 100.0);
        } else if has_corporate_objection(&text) {
            article.direction = "bearish".to_owned();
            article.score = article.score.min(-0.5);
            article.signal_reason = "Issuer objections and opt-outs indicate negative market tone".to_owned();
        } else {
            article.signal_reason = format!("Local crypto FinBERT tone classification · confidence {:.0}%", article.sentiment_confidence * 100.0);
        }
    }
}

async fn poll_news_feed(news: watch::Sender<Arc<NewsFeedSnapshot>>) {
    let mut snapshot = NewsFeedSnapshot { source: "CoinDesk RSS".to_owned(), stale: true, ..NewsFeedSnapshot::default() };
    let client = reqwest::Client::builder()
        .user_agent("sbe-ts-news-ticker/1.0 (RSS reader)")
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    loop {
        let result = async {
            let response = client
                .get("https://www.coindesk.com/arc/outboundfeeds/rss/?outputType=xml")
                .send()
                .await?
                .error_for_status()?;
            response.text().await
        }
        .await;
        let refreshed = match result {
            Ok(xml) => {
                let timestamp = now_ms();
                let mut articles = parse_news_rss(&xml, timestamp);
                classify_headlines_with_model(&mut articles).await;
                Some((timestamp, articles, None))
            }
            Err(error) => Some((now_ms(), Vec::new(), Some(format!("Could not refresh CoinDesk RSS: {error}")))),
        };
        if let Some((timestamp, articles, error)) = refreshed {
            if let Some(error) = error {
                snapshot.error = Some(error);
                snapshot.stale = true;
            } else if articles.is_empty() {
                snapshot.error = Some("RSS feed returned no parseable articles".to_owned());
                snapshot.stale = true;
            } else {
                snapshot.updated_at_ms = timestamp;
                snapshot.source = "CoinDesk RSS".to_owned();
                snapshot.stale = false;
                snapshot.error = None;
                snapshot.articles = articles;
            }
        }
        let forwarded = serde_json::to_vec(&snapshot).unwrap_or_default();
        news.send_replace(Arc::new(snapshot.clone()));
        let feed_handlers_url = std::env::var("FEED_HANDLERS_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3001".to_owned());
        if !forwarded.is_empty() {
            if let Err(error) = client
                .post(format!("{feed_handlers_url}/internal/news/snapshot"))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(forwarded)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
            {
                crate::service_log!("WARN", "Could not forward RSS snapshot to FeedHandlers: {error}");
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

fn json_number(value: &serde_json::Value, field: &str) -> Option<f64> {
    value
        .get(field)?
        .as_str()?
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
}

async fn okx_first(
    client: &reqwest::Client,
    path: &str,
    query: &[(&str, &str)],
) -> Option<serde_json::Value> {
    let query = query
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let response = client
        .get(format!("https://www.okx.com{path}?{query}"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    if response.get("code")?.as_str()? != "0" {
        return None;
    }
    response.get("data")?.as_array()?.first().cloned()
}

async fn okx_data(
    client: &reqwest::Client,
    path: &str,
    query: &[(&str, &str)],
) -> Option<Vec<serde_json::Value>> {
    let query = query
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let response = client
        .get(format!("https://www.okx.com{path}?{query}"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    if response.get("code")?.as_str()? != "0" {
        return None;
    }
    response.get("data")?.as_array().cloned()
}

fn option_volatility_summary(
    rows: &[serde_json::Value],
    now: u64,
) -> (Option<f64>, Option<f64>, Option<u64>) {
    let days_now = 86_400_000.0;
    let options = rows
        .iter()
        .filter_map(|row| {
            let expiry = row.get("expTime")?.as_str()?.parse::<u64>().ok()?;
            let days = expiry.saturating_sub(now) as f64 / days_now;
            if !(7.0..=60.0).contains(&days) {
                return None;
            }
            Some((
                row.get("optType")?.as_str()?.to_owned(),
                json_number(row, "delta")?,
                json_number(row, "markVol")?,
                expiry,
                days,
            ))
        })
        .collect::<Vec<_>>();
    let select = |kind: &str, target_delta: f64| {
        options
            .iter()
            .filter(|(option_type, _, _, _, _)| option_type == kind)
            .min_by(|a, b| {
                let score_a = (a.4 - 30.0).abs() + (a.1.abs() - target_delta).abs() * 30.0;
                let score_b = (b.4 - 30.0).abs() + (b.1.abs() - target_delta).abs() * 30.0;
                score_a.total_cmp(&score_b)
            })
    };
    let call_atm = select("C", 0.5);
    let put_atm = select("P", 0.5);
    let iv = call_atm
        .zip(put_atm)
        .map(|(call, put)| (call.2 + put.2) / 2.0)
        .or_else(|| call_atm.map(|row| row.2))
        .or_else(|| put_atm.map(|row| row.2));
    let call_25d = select("C", 0.25);
    let put_25d = select("P", 0.25);
    let skew = put_25d.zip(call_25d).map(|(put, call)| put.2 - call.2);
    let expiry = call_atm
        .map(|row| row.3)
        .or_else(|| put_atm.map(|row| row.3));
    (iv, skew, expiry)
}

async fn poll_external_market_features(store: SignalStore) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let mut previous_open_interest = HashMap::<String, f64>::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        for underlying in ["BTC", "ETH"] {
            let swap = format!("{underlying}-USDT-SWAP");
            let spot_index = format!("{underlying}-USDT");
            let option_underlying = format!("{underlying}-USD");
            let funding_q = [("instId", swap.as_str())];
            let oi_q = [("instType", "SWAP"), ("instId", swap.as_str())];
            let mark_q = [("instType", "SWAP"), ("instId", swap.as_str())];
            let index_q = [("instId", spot_index.as_str())];
            let options_q = [("uly", option_underlying.as_str())];
            let (funding, oi, mark, index, options) = tokio::join!(
                okx_first(&client, "/api/v5/public/funding-rate", &funding_q),
                okx_first(&client, "/api/v5/public/open-interest", &oi_q),
                okx_first(&client, "/api/v5/public/mark-price", &mark_q),
                okx_first(&client, "/api/v5/market/index-tickers", &index_q),
                okx_data(&client, "/api/v5/public/opt-summary", &options_q),
            );
            let timestamp = now_ms();
            let open_interest_contracts = oi.as_ref().and_then(|row| json_number(row, "oi"));
            let open_interest_change_percent = open_interest_contracts.and_then(|current| {
                let change = previous_open_interest
                    .insert(underlying.to_owned(), current)
                    .map(|previous| (current / previous - 1.0) * 100.0);
                change.filter(|value| value.is_finite())
            });
            let mark_price = mark.as_ref().and_then(|row| json_number(row, "markPx"));
            let index_price = index.as_ref().and_then(|row| json_number(row, "idxPx"));
            let option_summary = options
                .as_deref()
                .map(|rows| option_volatility_summary(rows, timestamp))
                .unwrap_or((None, None, None));
            let features = ExternalMarketFeatures {
                underlying: underlying.to_owned(),
                timestamp,
                swap_instrument: swap,
                funding_rate: funding
                    .as_ref()
                    .and_then(|row| json_number(row, "fundingRate")),
                next_funding_rate: funding
                    .as_ref()
                    .and_then(|row| json_number(row, "nextFundingRate")),
                open_interest_contracts,
                open_interest_currency: oi.as_ref().and_then(|row| json_number(row, "oiCcy")),
                open_interest_change_percent,
                mark_price,
                index_price,
                perp_basis_bps: mark_price
                    .zip(index_price)
                    .filter(|(_, index)| *index > 0.0)
                    .map(|(mark, index)| (mark / index - 1.0) * 10_000.0),
                options_atm_iv: option_summary.0,
                options_25d_skew: option_summary.1,
                options_expiry_timestamp: option_summary.2,
            };
            let has_data = features.funding_rate.is_some()
                || features.open_interest_contracts.is_some()
                || features.mark_price.is_some()
                || features.options_atm_iv.is_some()
                || features.options_25d_skew.is_some();
            if has_data {
                if let Err(error) = store.append_external_market_features(&features).await {
                    crate::service_log!(
                        "ERROR",
                        "Failed to store {underlying} derivatives/options context: {error}"
                    );
                }
            }
        }
    }
}

fn decode_signal_event(bytes: &[u8]) -> Option<SignalEventRequest> {
    let symbol_bytes = bytes.get(8..40)?;
    let symbol = std::str::from_utf8(symbol_bytes)
        .ok()?
        .trim_end_matches([' ', '\0'])
        .to_owned();
    let template = u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?);
    let event = match template {
        1 if bytes.len() >= 72 => SignalFeedEvent::Price {
            timestamp: u64::from_le_bytes(bytes.get(64..72)?.try_into().ok()?),
            price: f64::from_le_bytes(bytes.get(40..48)?.try_into().ok()?),
            volatility_1m: f64::from_le_bytes(bytes.get(48..56)?.try_into().ok()?),
        },
        2 if bytes.len() >= 272 => {
            let bid_levels = std::array::from_fn(|index| {
                (
                    f64::from_le_bytes(
                        bytes[48 + index * 16..56 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                    f64::from_le_bytes(
                        bytes[56 + index * 16..64 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                )
            });
            let ask_levels = std::array::from_fn(|index| {
                (
                    f64::from_le_bytes(
                        bytes[160 + index * 16..168 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                    f64::from_le_bytes(
                        bytes[168 + index * 16..176 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                )
            });
            SignalFeedEvent::Book {
                timestamp: u64::from_le_bytes(bytes.get(40..48)?.try_into().ok()?),
                best_bid: bid_levels[0].0,
                best_ask: ask_levels[0].0,
                bid_sizes: bid_levels.map(|(_, size)| size),
                ask_sizes: ask_levels.map(|(_, size)| size),
                bid_levels,
                ask_levels,
            }
        }
        3 if bytes.len() >= 97 => SignalFeedEvent::Trade {
            timestamp: u64::from_le_bytes(bytes.get(72..80)?.try_into().ok()?),
            price: f64::from_le_bytes(bytes.get(80..88)?.try_into().ok()?),
            size: f64::from_le_bytes(bytes.get(88..96)?.try_into().ok()?),
            is_buy: bytes[96] == 1,
        },
        _ => return None,
    };
    Some(SignalEventRequest { symbol, event })
}

async fn run_feed_signal_consumer(runtime: Arc<SignalRuntime>) {
    let mut retry_delay = Duration::from_secs(1);
    loop {
        let connection_started = Instant::now();
        match feed_client::signal_events().await {
            Ok(mut events) => {
                crate::service_log!(
                    "INFO",
                    "Connected to FeedHandlers RSocket signal-event stream"
                );
                let mut heartbeat = tokio::time::interval(Duration::from_secs(60));
                heartbeat.tick().await;
                let mut price_events = 0_u64;
                let mut trade_events = 0_u64;
                let mut book_events = 0_u64;
                let mut invalid_events = 0_u64;
                let mut last_event_at: Option<Instant> = None;
                let mut last_price_event_at: HashMap<String, Instant> = HashMap::new();
                loop {
                    tokio::select! {
                        _ = heartbeat.tick() => {
                            let idle_seconds = last_event_at
                                .map(|last_event| last_event.elapsed().as_secs())
                                .map(|seconds| seconds.to_string())
                                .unwrap_or_else(|| "never".to_owned());
                            let mut stale_price_symbols = last_price_event_at
                                .iter()
                                .map(|(symbol, last_event)| (symbol.clone(), last_event.elapsed().as_secs()))
                                .filter(|(_, age_seconds)| *age_seconds >= 60)
                                .collect::<Vec<_>>();
                            stale_price_symbols.sort_by(|left, right| right.1.cmp(&left.1));
                            let stale_price_count = stale_price_symbols.len();
                            stale_price_symbols.truncate(10);
                            crate::service_log!(
                                "INFO",
                                "FeedHandlers signal-event stream heartbeat: price={price_events} trade={trade_events} book={book_events} invalid={invalid_events} last_event_idle_seconds={idle_seconds} symbols_without_price_for_60s={stale_price_count} stalest_price_events={stale_price_symbols:?}"
                            );
                            price_events = 0;
                            trade_events = 0;
                            book_events = 0;
                            invalid_events = 0;
                        }
                        event = events.next() => {
                            match event {
                                Some(Ok(bytes)) => match decode_signal_event(&bytes) {
                                    Some(request) => {
                                        last_event_at = Some(Instant::now());
                                        match &request.event {
                                            SignalFeedEvent::Price { .. } => {
                                                price_events += 1;
                                                last_price_event_at.insert(request.symbol.clone(), Instant::now());
                                            }
                                            SignalFeedEvent::Trade { .. } => trade_events += 1,
                                            SignalFeedEvent::Book { .. } => book_events += 1,
                                        }
                                        if let Err((status, error)) = ingest_signal_event(
                                            axum::extract::State(Arc::clone(&runtime)),
                                            Json(request),
                                        ).await {
                                            crate::service_log!("WARN", "Signal event rejected with {status}: {error}");
                                        }
                                    }
                                    None => {
                                        invalid_events += 1;
                                        crate::service_log!("WARN", "Invalid FeedHandlers binary signal event");
                                    }
                                },
                                Some(Err(error)) => {
                                    invalid_events += 1;
                                    crate::service_log!("WARN", "Invalid FeedHandlers signal event: {error:#}");
                                }
                                None => break,
                            }
                        }
                    }
                }
                if connection_started.elapsed() >= Duration::from_secs(60) {
                    retry_delay = Duration::from_secs(1);
                }
                crate::service_log!(
                    "WARN",
                    "FeedHandlers RSocket signal-event stream disconnected; reconnecting in {}s",
                    retry_delay.as_secs()
                );
            }
            Err(error) => crate::service_log!(
                "WARN",
                "Could not connect to FeedHandlers signal-event stream: {error:#}; retrying in {}s",
                retry_delay.as_secs()
            ),
        }
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
    }
}

fn drain_latest(map: &DashMap<String, serde_json::Value>) -> Vec<(String, serde_json::Value)> {
    // Snapshot and sort keys before removing entries. Each shard guard is short
    // lived, and output order stays consistent with the previous BTreeMap queue.
    let mut keys = map
        .iter()
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.into_iter()
        .filter_map(|key| map.remove(&key).map(|(_, value)| (key, value)))
        .collect()
}

async fn run_ui_snapshot_push_worker(
    pending_snapshots: Arc<DashMap<String, serde_json::Value>>,
    notify: Arc<Notify>,
) {
    let client = POSITION_MANAGEMENT_CLIENT.get_or_init(reqwest::Client::new);
    let base =
        std::env::var("FEED_HANDLERS_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_owned());
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        let snapshots = drain_latest(&pending_snapshots);
        if snapshots.is_empty() {
            notified.await;
            continue;
        };
        for (_, snapshot) in snapshots {
            if let Err(error) = client
                .post(format!("{base}/internal/signals/snapshot"))
                .timeout(std::time::Duration::from_millis(250))
                .json(&snapshot)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
            {
                crate::service_log!(
                    "WARN",
                    "Signal snapshot fanout to FeedHandlers failed: {error}"
                );
            }
        }
    }
}

async fn run_signal_push_worker(
    pending_signals: Arc<DashMap<String, serde_json::Value>>,
    notify: Arc<Notify>,
) {
    let client = POSITION_MANAGEMENT_CLIENT.get_or_init(reqwest::Client::new);
    let base =
        std::env::var("AUTO_TRADER_URL").unwrap_or_else(|_| "http://127.0.0.1:3005".to_owned());
    let mut retry_delay = std::time::Duration::from_millis(250);
    let mut endpoint_available = true;
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        let dashboards = drain_latest(&pending_signals);
        if dashboards.is_empty() {
            notified.await;
            continue;
        };
        let mut failed = BTreeMap::new();
        for (symbol, dashboard) in dashboards {
            let result = client
                .post(format!("{base}/internal/auto-trader/signal"))
                .timeout(std::time::Duration::from_secs(2))
                .json(&dashboard)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            if let Err(error) = result {
                if endpoint_available {
                    crate::service_log!(
                        "WARN",
                        "Auto-trader signal delivery unavailable; retaining latest signal per symbol and retrying: {error}"
                    );
                    endpoint_available = false;
                }
                failed.insert(symbol, dashboard);
            }
        }
        if !failed.is_empty() {
            for (symbol, dashboard) in failed {
                pending_signals.entry(symbol).or_insert(dashboard);
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(5));
        } else {
            if !endpoint_available {
                crate::service_log!("INFO", "Auto-trader signal delivery recovered");
            }
            endpoint_available = true;
            retry_delay = std::time::Duration::from_millis(250);
        }
    }
}

async fn ingest_signal_event(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
    Json(request): Json<SignalEventRequest>,
) -> Result<Json<Option<SignalSnapshot>>, (StatusCode, String)> {
    if request.symbol.is_empty() || !request.symbol.is_ascii() || request.symbol.len() > 32 {
        return Err((StatusCode::BAD_REQUEST, "Invalid signal symbol".to_owned()));
    }
    // Nonblocking snapshot read: market feed handling never waits on RSS/model work.
    let current_news = Arc::clone(&runtime.news.borrow());
    let current_news_weight = runtime.news_learning.try_lock().map(|learning| learning.weight).unwrap_or(0.0);
    let (snapshot, records, market) = {
        let event_timestamp = match &request.event {
            SignalFeedEvent::Price { timestamp, .. }
            | SignalFeedEvent::Trade { timestamp, .. }
            | SignalFeedEvent::Book { timestamp, .. } => *timestamp,
        };
        // Preserve the original event ordering and atomic BTC/ETH context snapshot.
        // The sharded caches below are independent of this stateful event sequence.
        let mut engines = runtime.engines.lock().await;
        let btc_context = engines.get("BTC-USDT").map_or((None, None), |reference| {
            reference.return_context(event_timestamp)
        });
        let eth_context = engines.get("ETH-USDT").map_or((None, None), |reference| {
            reference.return_context(event_timestamp)
        });
        let engine = engines.entry(request.symbol.clone()).or_default();
        let mut records = Vec::new();
        let mut market = None;
        let snapshot = match request.event {
            SignalFeedEvent::Price {
                timestamp,
                price,
                volatility_1m,
            } => {
                let (price_count_before, previous_price) = engine.price_state();
                let outcomes = engine.on_price(timestamp, price, volatility_1m);
                let (price_count_after, latest_price) = engine.price_state();
                let accepted = price.is_finite()
                    && price > 0.0
                    && latest_price == Some((timestamp, price))
                    && (price_count_after > price_count_before || previous_price != latest_price);
                if !accepted && signal_diagnostic_due(&runtime, &request.symbol, timestamp)
                {
                    let reason = if !price.is_finite() || price <= 0.0 {
                        "invalid_price"
                    } else {
                        "out_of_order_timestamp"
                    };
                    crate::service_log!(
                        "WARN",
                        "Signal price sample rejected: symbol={} reason={} timestamp={} previous_timestamp={:?} price={} volatility_1m={}",
                        request.symbol,
                        reason,
                        timestamp,
                        previous_price.map(|sample| sample.0),
                        price,
                        volatility_1m
                    );
                }
                for outcome in outcomes {
                    if outcome.legacy.horizon_seconds == 60 {
                        if let Ok(mut learning) = runtime.news_learning.try_lock() {
                            if let Some(vote) = learning.pending.remove(&(outcome.legacy.symbol.clone(), outcome.legacy.signal_timestamp)) {
                                learning.samples += 1;
                                if vote * outcome.legacy.return_percent > 0.0 { learning.correct += 1; }
                                if learning.samples >= 30 {
                                    let accuracy = learning.correct as f64 / learning.samples as f64;
                                    learning.weight = (0.20 * ((accuracy - 0.45) / 0.20)).clamp(0.0, 0.20);
                                }
                            }
                        }
                    }
                    records.push(StoredSignalRecord::Outcome(outcome.legacy));
                    records.push(StoredSignalRecord::Evaluation(outcome.evaluation));
                }
                let should_compute_snapshot = accepted && engine.live_snapshot_due(timestamp);
                let mut snapshot = should_compute_snapshot
                    .then(|| engine.snapshot(&request.symbol, timestamp))
                    .flatten();
                if should_compute_snapshot && snapshot.is_none()
                    && signal_diagnostic_due(&runtime, &format!("missing:{}", request.symbol), timestamp)
                {
                    crate::service_log!(
                        "WARN",
                        "Signal snapshot unavailable: symbol={} timestamp={} diagnostics={}",
                        request.symbol,
                        timestamp,
                        engine.input_diagnostic(timestamp)
                    );
                } else if should_compute_snapshot
                    && snapshot.is_some()
                    && signal_diagnostic_due(&runtime, &format!("generated:{}", request.symbol), timestamp)
                {
                    let age_ms = now_ms().saturating_sub(timestamp);
                    crate::service_log!(
                        "INFO",
                        "Signal snapshot generated: symbol={} event_timestamp={} age_ms={} diagnostics={}",
                        request.symbol,
                        timestamp,
                        age_ms,
                        engine.input_diagnostic(timestamp)
                    );
                }
                let news_vote;
                if let Some(snapshot) = snapshot.as_mut() {
                    news_vote = apply_news_signal(snapshot, &current_news, now_ms(), current_news_weight);
                    // The pushed decision needs the latest available market cache on
                    // every price event, regardless of the research sampling cadence.
                    if let Some(book_snapshot) =
                        engine.book_snapshot(&request.symbol, snapshot.timestamp)
                    {
                        let levels = |levels: Vec<(f64, f64)>| {
                            levels.into_iter()
                            .map(|(price, size)| serde_json::json!({"price": price, "size": size}))
                            .collect::<Vec<_>>()
                        };
                        market = Some(serde_json::json!({
                            request.symbol.clone(): {
                                "bids": levels(book_snapshot.bid_levels.clone()),
                                "asks": levels(book_snapshot.ask_levels.clone()),
                                "updated_at_ms": book_snapshot.timestamp
                            }
                        }));
                        if engine.research_sample_due(snapshot.timestamp) {
                            let features = engine.research_features(
                                &request.symbol,
                                snapshot.timestamp,
                                btc_context,
                                eth_context,
                            );
                            records.push(StoredSignalRecord::BookSnapshot(book_snapshot));
                            records.push(StoredSignalRecord::ResearchFeatures(features));
                            engine.track(snapshot);
                            if let Some(vote) = news_vote {
                                if let Ok(mut learning) = runtime.news_learning.try_lock() {
                                    learning.pending.insert((snapshot.symbol.clone(), snapshot.timestamp), vote);
                                }
                            }
                            records.push(StoredSignalRecord::Snapshot(snapshot.clone()));
                        }
                    } else if engine.research_sample_due(snapshot.timestamp) {
                        // Preserve feature and snapshot samples when a fresh book is
                        // unavailable; depth data is optional in the research schema.
                        let features = engine.research_features(
                            &request.symbol,
                            snapshot.timestamp,
                            btc_context,
                            eth_context,
                        );
                        records.push(StoredSignalRecord::ResearchFeatures(features));
                        engine.track(snapshot);
                        if let Some(vote) = news_vote { if let Ok(mut learning) = runtime.news_learning.try_lock() { learning.pending.insert((snapshot.symbol.clone(), snapshot.timestamp), vote); } }
                        records.push(StoredSignalRecord::Snapshot(snapshot.clone()));
                    }
                }
                snapshot
            }
            SignalFeedEvent::Trade {
                timestamp,
                price,
                size,
                is_buy,
            } => {
                engine.on_trade(timestamp, price, size, is_buy);
                None
            }
            SignalFeedEvent::Book {
                timestamp,
                best_bid,
                best_ask,
                bid_sizes,
                ask_sizes,
                bid_levels,
                ask_levels,
            } => {
                engine.on_book(
                    timestamp,
                    best_bid,
                    best_ask,
                    &bid_sizes,
                    &ask_sizes,
                    &bid_levels,
                    &ask_levels,
                );
                None
            }
        };
        (snapshot, records, market)
    };
    if !records.is_empty() {
        let store = runtime.store.clone();
        tokio::spawn(async move {
            if let Err(error) = store.append_records(records).await {
                crate::service_log!(
                    "ERROR",
                    "Could not persist signal event off hot path: {error}"
                );
            }
        });
    }
    if let Some(snapshot) = snapshot.as_ref() {
        let markets = market.unwrap_or_else(|| serde_json::json!({}));
        let missed_opportunity_bands = runtime
            .store
            .missed_opportunity_bands()
            .await
            .unwrap_or_else(|error| {
                crate::service_log!(
                    "WARN",
                    "Could not refresh skipped-signal counterfactual outcomes: {error}"
                );
                runtime.store.cached_missed_opportunity_bands()
            });
        let dashboard = serde_json::json!({
            "best": [snapshot],
            "walkForward": runtime.store.cached_walk_forward(),
            "missedOpportunityBands": missed_opportunity_bands,
            "markets": markets
        });
        runtime
            .signal_push_cache
            .insert(snapshot.symbol.clone(), dashboard);
        runtime.signal_push_notify.notify_one();
        let snapshot_json = serde_json::to_value(snapshot).unwrap_or(serde_json::Value::Null);
        runtime
            .ui_snapshot_cache
            .insert(snapshot.symbol.clone(), snapshot_json);
        runtime.ui_snapshot_notify.notify_one();
    }
    Ok(Json(snapshot))
}

#[derive(Clone, Deserialize)]
struct Position {
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    entry_fees: f64,
    estimated_exit_fee_rate: f64,
}
#[derive(Clone, Deserialize)]
struct BookLevel {
    price: f64,
    size: f64,
}
#[derive(Deserialize)]
struct PositionMarketInput {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
    price_samples: Vec<(u64, f64)>,
}
#[derive(Deserialize)]
struct PositionInsightsInput {
    positions: Vec<Position>,
    markets: HashMap<String, PositionMarketInput>,
}
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum TradeSide {
    Buy,
    Sell,
}
#[derive(Serialize)]
struct CloseInsight {
    symbol: String,
    close_side: TradeSide,
    close_quantity: f64,
    close_price: Option<f64>,
    current_price: Option<f64>,
    breakeven_price: Option<f64>,
    close_pnl: Option<f64>,
    close_fee: Option<f64>,
    depth_covers_position: bool,
    close_depth_quantity: f64,
    profitable_close_probability_5m: Option<f64>,
    probability_samples: usize,
}
#[derive(Serialize)]
struct CloseInsightsResponse {
    insights: Vec<CloseInsight>,
}

async fn get_position_insights() -> Result<Json<CloseInsightsResponse>, (StatusCode, String)> {
    let base = std::env::var("POSITION_MANAGEMENT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3002".to_owned());
    let source: PositionInsightsInput = POSITION_MANAGEMENT_CLIENT
        .get_or_init(reqwest::Client::new)
        .get(format!("{base}/internal/insights-inputs"))
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Position Management request failed: {error}"),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Position Management returned an error: {error}"),
            )
        })?
        .json()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid position insight input: {error}"),
            )
        })?;
    let insights = source
        .positions
        .into_iter()
        .filter(|p| p.quantity.abs() > 1e-12)
        .map(|position| {
            let market = source.markets.get(&position.symbol);
            let closing_long = position.quantity > 0.0;
            let close_side = if closing_long {
                TradeSide::Sell
            } else {
                TradeSide::Buy
            };
            let levels = market
                .map(|m| if closing_long { &m.bids } else { &m.asks })
                .cloned()
                .unwrap_or_default();
            let mut remaining = position.quantity.abs();
            let mut notional: f64 = 0.0;
            for level in &levels {
                let qty = remaining.min(level.size);
                notional += qty * level.price;
                remaining -= qty;
                if remaining <= 1e-12 {
                    break;
                }
            }
            let close_depth_quantity = (position.quantity.abs() - remaining).max(0.0);
            let depth_covers_position = remaining <= 1e-12;
            let current_price = market.and_then(|m| {
                let bid = m.bids.first()?.price;
                let ask = m.asks.first()?.price;
                let mid = (bid + ask) / 2.0;
                (bid.is_finite() && ask.is_finite() && bid > 0.0 && ask >= bid && mid.is_finite())
                    .then_some(mid)
            });
            let close_price = if depth_covers_position && position.quantity.abs() > 1e-12 {
                let average = notional / position.quantity.abs();
                (notional.is_finite() && notional > 0.0 && average.is_finite() && average > 0.0)
                    .then_some(average)
            } else {
                None
            };
            let breakeven_price = {
                let quantity = position.quantity.abs();
                let fee_rate = position.estimated_exit_fee_rate;
                let (numerator, denominator) = if closing_long {
                    (
                        position.average_entry_price * quantity + position.entry_fees,
                        quantity * (1.0 - fee_rate),
                    )
                } else {
                    (
                        position.average_entry_price * quantity - position.entry_fees,
                        quantity * (1.0 + fee_rate),
                    )
                };
                (numerator.is_finite()
                    && denominator.is_finite()
                    && numerator > 0.0
                    && denominator > 0.0)
                    .then_some(numerator / denominator)
                    .filter(|price| price.is_finite() && *price > 0.0)
            };
            let close_fee = close_price
                .map(|price| position.quantity.abs() * price * position.estimated_exit_fee_rate);
            let close_pnl = close_price
                .map(|price| {
                    (price - position.average_entry_price) * position.quantity
                        - position.entry_fees
                        - position.quantity.abs() * price * position.estimated_exit_fee_rate
                })
                .filter(|pnl| pnl.is_finite());
            let samples = market.map(|m| m.price_samples.as_slice()).unwrap_or(&[]);
            let probability = if samples.len() >= 31 && close_price.is_some() {
                let returns: Vec<f64> = samples
                    .windows(2)
                    .filter_map(|pair| {
                        let dt = pair[1].0.saturating_sub(pair[0].0) as f64;
                        (dt > 0.0 && pair[0].1 > 0.0 && pair[1].1 > 0.0)
                            .then_some((pair[1].1 / pair[0].1).ln() / (dt / 10_000.0))
                    })
                    .collect();
                if returns.len() >= 30 {
                    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
                    let variance = returns.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                        / (returns.len() - 1) as f64;
                    let projected_mean = close_price.unwrap().ln() + mean * 30.0;
                    let projected_sd = (variance * 30.0).sqrt();
                    if projected_sd > 0.0 {
                        let below = normal_cdf(
                            (position.average_entry_price.ln() - projected_mean) / projected_sd,
                        );
                        Some(if closing_long { 1.0 - below } else { below })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            CloseInsight {
                symbol: position.symbol,
                close_side,
                close_quantity: position.quantity.abs(),
                close_price,
                current_price,
                breakeven_price,
                close_pnl,
                close_fee,
                depth_covers_position,
                close_depth_quantity,
                profitable_close_probability_5m: probability,
                probability_samples: samples.len(),
            }
        })
        .collect();
    Ok(Json(CloseInsightsResponse { insights }))
}

fn normal_cdf(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let erf = 1.0
        - (((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t)
            * (-x * x).exp();
    0.5 * (1.0 + sign * erf)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let signal_store = SignalStore::from_environment()?;
    daily_summary::initialize().await?;
    daily_summary::spawn_nightly();
    tokio::spawn(poll_external_market_features(signal_store.clone()));
    let warm_store = signal_store.clone();
    tokio::spawn(async move {
        if let Err(error) = warm_store.walk_forward().await {
            crate::service_log!("WARN", "Initial walk-forward cache warm failed: {error}");
        }
        if let Err(error) = warm_store.missed_opportunity_bands().await {
            crate::service_log!(
                "WARN",
                "Initial skipped-signal counterfactual cache warm failed: {error}"
            );
        }
    });
    let signal_push_cache = Arc::new(DashMap::new());
    let signal_push_notify = Arc::new(Notify::new());
    tokio::spawn(run_signal_push_worker(
        Arc::clone(&signal_push_cache),
        Arc::clone(&signal_push_notify),
    ));
    let ui_snapshot_cache = Arc::new(DashMap::new());
    let ui_snapshot_notify = Arc::new(Notify::new());
    tokio::spawn(run_ui_snapshot_push_worker(
        Arc::clone(&ui_snapshot_cache),
        Arc::clone(&ui_snapshot_notify),
    ));
    let initial_news = Arc::new(NewsFeedSnapshot {
        source: "CoinDesk RSS".to_owned(),
        stale: true,
        ..NewsFeedSnapshot::default()
    });
    let (news_tx, news_rx) = watch::channel(initial_news);
    tokio::spawn(poll_news_feed(news_tx));
    let signal_runtime = Arc::new(SignalRuntime {
        store: signal_store.clone(),
        engines: Mutex::new(HashMap::new()),
        signal_push_cache,
        signal_push_notify,
        ui_snapshot_cache,
        ui_snapshot_notify,
        news: news_rx,
        news_learning: Mutex::new(NewsLearning { weight: 0.20, ..NewsLearning::default() }),
        snapshot_diagnostic_at: DashMap::new(),
    });
    tokio::spawn(run_feed_signal_consumer(Arc::clone(&signal_runtime)));
    let app = Router::new()
        .merge(daily_summary::router())
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/positions/insights", get(get_position_insights))
        .route(
            "/internal/signals/event",
            axum::routing::post(ingest_signal_event).with_state(signal_runtime),
        )
        .merge(signal_store.router());
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3004)).await?;
    crate::service_log!("INFO", "Insights service listening on http://0.0.0.0:3004");
    axum::serve(listener, app).await?;
    Ok(())
}
async fn process_cpu_seconds() -> String {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields = stat
        .rsplit_once(')')
        .map(|(_, fields)| fields)
        .unwrap_or("");
    let values: Vec<u64> = fields
        .split_whitespace()
        .filter_map(|value| value.parse().ok())
        .collect();
    values
        .get(11)
        .zip(values.get(12))
        .map(|(user, system)| (*user + *system) as f64 / 100.0)
        .unwrap_or(0.0)
        .to_string()
}
