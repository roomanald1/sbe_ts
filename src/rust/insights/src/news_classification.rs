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

fn matched_news_keywords(text: &str) -> Vec<String> {
    const TERMS: [&str; 48] = [
        "APPROVES", "APPROVAL", "ADOPTS", "LAUNCHES", "SURGES", "RALLIES", "GAINS",
        "INFLOWS", "PARTNERSHIP", "INTEGRATES", "RECORD HIGH", "BREAKS OUT", "GOES LIVE",
        "UPGRADE", "OVERHAUL", "HACK", "EXPLOIT", "BREACH", "LAWSUIT", "BANNED",
        "PLUNGES", "FALLS", "DROPS", "OUTFLOWS", "LIQUIDATION", "BANKRUPT", "DELIST",
        "DELAYED", "SUED", "BAN", "TAX", "TAXES", "TAXATION", "LEVY", "DUTY", "DUTIES",
        "OBJECTION", "OPPOSITION", "PUSHBACK", "RESISTANCE", "LEVERAGE", "VOLATILITY",
        "CRASH", "RISK", "ATTACK", "OUTAGE", "POSTPONED", "FAILS TO LAUNCH",
    ];
    let upper = text.to_ascii_uppercase();
    TERMS.iter().filter(|term| upper.contains(**term)).map(|term| term.to_ascii_lowercase()).collect()
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
    NewsArticle { title: title.to_owned(), article_text: String::new(), url, published_at, received_at_ms: now, matched_asset: asset.to_owned(), direction: direction.to_owned(), score, signal_reason, sentiment_model: "keyword_fallback".to_owned(), sentiment_confidence: 0.0, matched_keywords: matched_news_keywords(title) }
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
        article.matched_keywords = matched_news_keywords(&format!("{} {}", article.title, article.article_text));
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
    let pre_news_score = snapshot.score;
    // Headline sentiment is noisy, so its learned contribution is capped at 20%.
    snapshot.score = (snapshot.score * (1.0 - weight) + news_score * weight).clamp(-1.0, 1.0);
    snapshot.direction = if snapshot.score > 0.2 {
        "bullish".to_owned()
    } else if snapshot.score < -0.2 {
        "bearish".to_owned()
    } else {
        "neutral".to_owned()
    };
    snapshot.news_attribution = Some(NewsAttribution {
        pre_news_score,
        post_news_score: snapshot.score,
        aggregate_score: news_score,
        weight,
        article_count: relevant.len(),
        source: news.source.clone(),
        articles: relevant.iter().map(|article| NewsHeadlineAttribution {
            title: article.title.clone(),
            url: article.url.clone(),
            published_at: article.published_at.clone(),
            received_at_ms: article.received_at_ms,
            matched_asset: article.matched_asset.clone(),
            direction: article.direction.clone(),
            score: article.score,
            sentiment_model: article.sentiment_model.clone(),
            sentiment_confidence: article.sentiment_confidence,
            matched_keywords: article.matched_keywords.clone(),
            signal_reason: article.signal_reason.clone(),
        }).collect(),
    });
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
