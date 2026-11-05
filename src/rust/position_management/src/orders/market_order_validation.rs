struct MarketOrderOptions {
    maker_fee_rate: f64,
    taker_fee_rate: f64,
    post_only: bool,
    limit: bool,
    fill_or_kill: bool,
    limit_price: Option<f64>,
}

fn validate_market_order_options(
    request: &NewOrder,
) -> Result<MarketOrderOptions, (StatusCode, String)> {
    let fee_rates = configured_fee_rates();
    let maker_fee_rate = fee_rates.maker;
    let taker_fee_rate = fee_rates.taker;
    let is_post_only = matches!(request.order_type, OrderType::PostOnlyLimit);
    let is_limit = matches!(
        request.order_type,
        OrderType::Limit | OrderType::PostOnlyLimit
    );
    let is_fok = matches!(request.order_type, OrderType::Fok);
    let limit_price = if is_limit {
        let p = request.limit_price.unwrap_or(f64::NAN);
        if !p.is_finite() || p <= 0.0 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Limit price must be finite and greater than zero".into(),
            ));
        }
        Some(p)
    } else {
        None
    };
    if request
        .expires_at
        .is_some_and(|expires| expires <= now_ms().unwrap_or_default())
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Good-till-date must be in the future".into(),
        ));
    }
    if request.expires_at.is_some() && !is_limit {
        return Err((
            StatusCode::BAD_REQUEST,
            "Good-till-date is supported only for limit orders".into(),
        ));
    }
    if request
        .time_in_force
        .as_deref()
        .is_some_and(|tif| tif != "gtd" && tif != "gtc")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Supported time-in-force values are gtc and gtd".into(),
        ));
    }
    if request.time_in_force.as_deref() == Some("gtd") && request.expires_at.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "GTD limit order requires expires_at".into(),
        ));
    }
    if request.time_in_force.as_deref() == Some("gtc") && request.expires_at.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "GTC order cannot include expires_at".into(),
        ));
    }
    Ok(MarketOrderOptions {
        maker_fee_rate,
        taker_fee_rate,
        post_only: is_post_only,
        limit: is_limit,
        fill_or_kill: is_fok,
        limit_price,
    })
}

#[cfg(test)]
mod market_order_validation_tests {
    use super::*;

    fn request(order_type: &str, limit_price: Option<f64>, time_in_force: Option<&str>, expires_at: Option<u64>) -> NewOrder {
        serde_json::from_value(serde_json::json!({
            "symbol": "BTC-USDT", "side": "buy", "order_type": order_type,
            "quantity": 1.0, "limit_price": limit_price,
            "time_in_force": time_in_force, "expires_at": expires_at
        })).unwrap()
    }

    #[test]
    fn validates_limit_price_and_limit_order_time_in_force_rules() {
        let invalid = request("limit", None, None, None);
        assert_eq!(validate_market_order_options(&invalid).err().unwrap().0, StatusCode::BAD_REQUEST);

        let gtd_without_expiry = request("limit", Some(100.0), Some("gtd"), None);
        assert_eq!(validate_market_order_options(&gtd_without_expiry).err().unwrap().0, StatusCode::BAD_REQUEST);

        let unsupported_tif = request("limit", Some(100.0), Some("ioc"), None);
        assert_eq!(validate_market_order_options(&unsupported_tif).err().unwrap().0, StatusCode::BAD_REQUEST);

        let expiry_on_market = request("market", None, None, Some(u64::MAX));
        assert_eq!(validate_market_order_options(&expiry_on_market).err().unwrap().0, StatusCode::BAD_REQUEST);

        let expired = request("limit", Some(100.0), Some("gtd"), Some(1));
        assert_eq!(validate_market_order_options(&expired).err().unwrap().0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn valid_post_only_limit_produces_limit_options() {
        let valid = request("post-only-limit", Some(100.0), Some("gtc"), None);
        let options = validate_market_order_options(&valid).unwrap();
        assert!(options.post_only);
        assert!(options.limit);
        assert!(!options.fill_or_kill);
        assert_eq!(options.limit_price, Some(100.0));
        assert!(options.maker_fee_rate.is_finite());
        assert!(options.taker_fee_rate.is_finite());
    }
}
