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
