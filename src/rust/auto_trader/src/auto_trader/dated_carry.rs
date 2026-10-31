// Paper-only dated futures cash-and-carry. It never submits exchange orders.
const CARRY_COST_GATE_PERCENT: f64 = 0.5;
const CARRY_ENTRY_SLIPPAGE: f64 = 0.0005;
const CARRY_DAY_MS: u64 = 86_400_000;

#[derive(Clone, Serialize, Deserialize)]
struct DatedCarryCandidate {
    spot_symbol: String,
    future_symbol: String,
    expiry_ms: u64,
    days_to_expiry: f64,
    quantity: f64,
    spot_ask: f64,
    future_bid: f64,
    #[serde(default)]
    spot_bid: f64,
    #[serde(default)]
    future_ask: f64,
    basis_percent: f64,
    net_after_budget_percent: f64,
    eligible: bool,
    reason: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct DatedCarryPosition {
    spot_symbol: String,
    future_symbol: String,
    expiry_ms: u64,
    entry_ms: u64,
    quantity: f64,
    spot_entry_price: f64,
    future_entry_price: f64,
    entry_fees: f64,
    status: String,
    #[serde(default)]
    spot_exit_twap: Option<f64>,
    #[serde(default)]
    settlement_price: Option<f64>,
    #[serde(default)]
    net_pnl_usdt: Option<f64>,
    #[serde(default)]
    settlement_source: Option<String>,
    #[serde(default)]
    spot_mark_bid: Option<f64>,
    #[serde(default)]
    future_mark_ask: Option<f64>,
    #[serde(default)]
    spot_leg_pnl_usdt: Option<f64>,
    #[serde(default)]
    future_leg_pnl_usdt: Option<f64>,
    #[serde(default)]
    estimated_exit_fees_usdt: Option<f64>,
    #[serde(default)]
    estimated_exit_slippage_usdt: Option<f64>,
    #[serde(default)]
    marked_net_pnl_usdt: Option<f64>,
    #[serde(default)]
    mark_checked_at_ms: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct DatedCarryBook {
    enabled: bool,
    max_spot_notional_usdt: f64,
    #[serde(default)]
    candidates: Vec<DatedCarryCandidate>,
    #[serde(default)]
    positions: Vec<DatedCarryPosition>,
    #[serde(default)]
    checked_at_ms: u64,
    #[serde(default)]
    last_error: Option<String>,
}

impl Default for DatedCarryBook {
    fn default() -> Self {
        Self {
            enabled: false,
            max_spot_notional_usdt: 1_000.0,
            candidates: Vec::new(),
            positions: Vec::new(),
            checked_at_ms: 0,
            last_error: None,
        }
    }
}

fn carry_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn update_dated_carry_mark(
    position: &mut DatedCarryPosition,
    spot_mark_bid: f64,
    future_mark_ask: f64,
    checked_at_ms: u64,
) {
    if !spot_mark_bid.is_finite() || spot_mark_bid <= 0.0
        || !future_mark_ask.is_finite() || future_mark_ask <= 0.0
    {
        return;
    }
    let quantity = position.quantity;
    let spot_leg_pnl = quantity * (spot_mark_bid - position.spot_entry_price);
    let future_leg_pnl = quantity * (position.future_entry_price - future_mark_ask);
    let exit_fees = quantity * (0.001 * spot_mark_bid + 0.0005 * future_mark_ask);
    let exit_slippage = quantity * CARRY_ENTRY_SLIPPAGE * (spot_mark_bid + future_mark_ask);
    let net = spot_leg_pnl + future_leg_pnl - position.entry_fees - exit_fees - exit_slippage;
    if [spot_leg_pnl, future_leg_pnl, exit_fees, exit_slippage, net].iter().any(|value| !value.is_finite()) {
        return;
    }
    position.spot_mark_bid = Some(spot_mark_bid);
    position.future_mark_ask = Some(future_mark_ask);
    position.spot_leg_pnl_usdt = Some(spot_leg_pnl);
    position.future_leg_pnl_usdt = Some(future_leg_pnl);
    position.estimated_exit_fees_usdt = Some(exit_fees);
    position.estimated_exit_slippage_usdt = Some(exit_slippage);
    position.marked_net_pnl_usdt = Some(net);
    position.mark_checked_at_ms = Some(checked_at_ms);
}

fn dated_carry_path(settings_path: &PathBuf) -> PathBuf {
    settings_path.with_extension("dated_carry.json")
}

fn save_dated_carry(path: &PathBuf, book: &DatedCarryBook) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(book).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("dated_carry.tmp");
    std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, path).map_err(|error| error.to_string())
}

async fn get_dated_carry(State(state): State<AutoTraderState>) -> Json<DatedCarryBook> {
    let mut book = state.dated_carry.lock().await.clone();
    let settings = state.settings.lock().await;
    book.enabled = state.enabled.load(Ordering::Relaxed) && settings.strategies.contains(&Strategy::DatedCarry);
    Json(book)
}

#[derive(Deserialize)]
struct DatedCarryUpdate {
    max_spot_notional_usdt: f64,
}

async fn update_dated_carry(
    State(state): State<AutoTraderState>,
    Json(update): Json<DatedCarryUpdate>,
) -> Result<Json<DatedCarryBook>, (StatusCode, String)> {
    if !update.max_spot_notional_usdt.is_finite()
        || !(100.0..=10_000.0).contains(&update.max_spot_notional_usdt)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Paper spot notional must be between 100 and 10,000 USDT".into(),
        ));
    }
    let _work = state.dated_carry_work_lock.lock().await;
    let mut book = state.dated_carry.lock().await;
    book.max_spot_notional_usdt = update.max_spot_notional_usdt;
    save_dated_carry(&state.dated_carry_path, &book)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let mut response = book.clone();
    drop(book);
    let settings = state.settings.lock().await;
    response.enabled = state.enabled.load(Ordering::Relaxed) && settings.strategies.contains(&Strategy::DatedCarry);
    Ok(Json(response))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DatedCarrySignal {
    Scan {
        emitted_at_ms: u64,
        candidates: Vec<DatedCarryCandidate>,
        last_error: Option<String>,
    },
    Entry {
        emitted_at_ms: u64,
        candidate: DatedCarryCandidate,
    },
    Settlement {
        emitted_at_ms: u64,
        spot_symbol: String,
        future_symbol: String,
        expiry_ms: u64,
        spot_exit_twap: f64,
        settlement_price: f64,
        settlement_source: String,
    },
}

async fn receive_dated_carry_signal(
    State(state): State<AutoTraderState>,
    Json(signal): Json<DatedCarrySignal>,
) -> Result<StatusCode, (StatusCode, String)> {
    let _work = state.dated_carry_work_lock.lock().await;
    let now_ms = carry_now_ms();
    let mut book = state.dated_carry.lock().await;
    let mut changed = false;
    let mut audit_payload = None;
    match signal {
        DatedCarrySignal::Scan { emitted_at_ms, candidates, last_error } => {
            if emitted_at_ms > now_ms + 5_000 || now_ms.saturating_sub(emitted_at_ms) > 120_000 {
                return Err((StatusCode::BAD_REQUEST, "Carry scan update is stale".into()));
            }
            book.checked_at_ms = emitted_at_ms;
            book.last_error = last_error;
            if book.last_error.is_none() {
                for position in book.positions.iter_mut().filter(|position| position.status == "open") {
                    if let Some(candidate) = candidates.iter().find(|candidate| {
                        candidate.future_symbol == position.future_symbol
                            && candidate.spot_symbol == position.spot_symbol
                    }) {
                        update_dated_carry_mark(position, candidate.spot_bid, candidate.future_ask, emitted_at_ms);
                    }
                }
                book.candidates = candidates;
            }
            changed = true;
        }
        DatedCarrySignal::Entry { emitted_at_ms, candidate } => {
            if emitted_at_ms > now_ms + 5_000 || now_ms.saturating_sub(emitted_at_ms) > 120_000 {
                return Err((StatusCode::BAD_REQUEST, "Carry entry signal is stale".into()));
            }
            if !candidate.quantity.is_finite()
                || !candidate.spot_ask.is_finite()
                || !candidate.future_bid.is_finite()
                || !candidate.spot_bid.is_finite()
                || !candidate.future_ask.is_finite()
                || !candidate.basis_percent.is_finite()
                || !candidate.net_after_budget_percent.is_finite()
                || candidate.quantity <= 0.0
                || candidate.spot_ask <= 0.0
                || candidate.future_bid <= 0.0
                || candidate.spot_bid <= 0.0
                || candidate.future_ask <= 0.0
                || candidate.expiry_ms <= now_ms
                || candidate.spot_symbol != "BTCUSDT" && candidate.spot_symbol != "ETHUSDT"
                || !candidate.future_symbol.starts_with(&format!("{}_", candidate.spot_symbol))
            {
                return Err((StatusCode::BAD_REQUEST, "Invalid dated carry entry signal".into()));
            }
            let days = (candidate.expiry_ms - now_ms) as f64 / CARRY_DAY_MS as f64;
            if days < 7.0 {
                return Err((StatusCode::BAD_REQUEST, "Carry entry inside the 7-day expiry cutoff".into()));
            }
            book.checked_at_ms = emitted_at_ms;
            book.last_error = None;
            book.candidates.retain(|item| item.future_symbol != candidate.future_symbol);
            book.candidates.push(candidate.clone());
            let selected = {
                let settings = state.settings.lock().await;
                state.enabled.load(Ordering::Relaxed)
                    && settings.strategies.contains(&Strategy::DatedCarry)
            };
            if selected && candidate.eligible
                && candidate.net_after_budget_percent > 0.0
                && candidate.basis_percent > CARRY_COST_GATE_PERCENT
                && !book.positions.iter().any(|position| position.future_symbol == candidate.future_symbol)
            {
                let spot_entry = candidate.spot_ask * (1.0 + CARRY_ENTRY_SLIPPAGE);
                let future_entry = candidate.future_bid * (1.0 - CARRY_ENTRY_SLIPPAGE);
                if candidate.quantity * spot_entry > book.max_spot_notional_usdt + 0.01 {
                    return Err((StatusCode::BAD_REQUEST, "Carry signal exceeds the paper size limit".into()));
                }
                let fees = candidate.quantity * (0.001 * spot_entry + 0.0005 * future_entry);
                audit_payload = Some(serde_json::json!({
                    "decision_id": format!("dated-carry:{}", candidate.future_symbol),
                    "stage": "dated_carry_entry",
                    "timestamp": now_ms,
                    "symbol": candidate.spot_symbol,
                    "future_symbol": candidate.future_symbol,
                    "quantity": candidate.quantity,
                    "spot_entry_price": spot_entry,
                    "future_entry_price": future_entry,
                    "entry_fees": fees,
                    "signal_basis_percent": candidate.basis_percent,
                }));
                let mut position = DatedCarryPosition {
                    spot_symbol: candidate.spot_symbol,
                    future_symbol: candidate.future_symbol,
                    expiry_ms: candidate.expiry_ms,
                    entry_ms: now_ms,
                    quantity: candidate.quantity,
                    spot_entry_price: spot_entry,
                    future_entry_price: future_entry,
                    entry_fees: fees,
                    status: "open".into(),
                    spot_exit_twap: None,
                    settlement_price: None,
                    net_pnl_usdt: None,
                    settlement_source: None,
                    spot_mark_bid: None,
                    future_mark_ask: None,
                    spot_leg_pnl_usdt: None,
                    future_leg_pnl_usdt: None,
                    estimated_exit_fees_usdt: None,
                    estimated_exit_slippage_usdt: None,
                    marked_net_pnl_usdt: None,
                    mark_checked_at_ms: None,
                };
                update_dated_carry_mark(&mut position, candidate.spot_bid, candidate.future_ask, emitted_at_ms);
                book.positions.push(position);
                changed = true;
            }
        }
        DatedCarrySignal::Settlement {
            emitted_at_ms,
            spot_symbol,
            future_symbol,
            expiry_ms,
            spot_exit_twap,
            settlement_price,
            settlement_source,
        } => {
            if emitted_at_ms > now_ms + 5_000
                || expiry_ms > now_ms
                || !spot_exit_twap.is_finite()
                || !settlement_price.is_finite()
                || spot_exit_twap <= 0.0
                || settlement_price <= 0.0
                || !matches!(settlement_source.as_str(), "official" | "index_1m_estimate")
            {
                return Err((StatusCode::BAD_REQUEST, "Invalid dated carry settlement signal".into()));
            }
            if let Some(position) = book.positions.iter_mut().find(|position| {
                position.future_symbol == future_symbol
                    && position.spot_symbol == spot_symbol
                    && position.expiry_ms == expiry_ms
                    && !position.status.starts_with("closed")
            }) {
                let spot_exit = spot_exit_twap * (1.0 - CARRY_ENTRY_SLIPPAGE);
                let q = position.quantity;
                let exit_fees = q * (0.001 * spot_exit + 0.0005 * settlement_price
                    + CARRY_ENTRY_SLIPPAGE * settlement_price);
                let net = q * (position.future_entry_price - position.spot_entry_price
                    + spot_exit - settlement_price) - position.entry_fees - exit_fees;
                position.spot_exit_twap = Some(spot_exit);
                position.settlement_price = Some(settlement_price);
                position.settlement_source = Some(settlement_source.clone());
                position.net_pnl_usdt = Some(net);
                audit_payload = Some(serde_json::json!({
                    "decision_id": format!("dated-carry:{}", position.future_symbol),
                    "stage": "dated_carry_exit",
                    "timestamp": now_ms,
                    "symbol": position.spot_symbol,
                    "future_symbol": position.future_symbol,
                    "quantity": q,
                    "spot_exit_twap": spot_exit,
                    "settlement_price": settlement_price,
                    "settlement_source": settlement_source,
                    "net_pnl_usdt": net,
                }));
                position.status = if settlement_source == "official" { "closed" }
                    else { "closed_estimated" }.into();
                changed = true;
            }
        }
    }
    if changed {
        save_dated_carry(&state.dated_carry_path, &book)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }
    drop(book);
    if let Some(payload) = audit_payload {
        if let Err(error) = state.audit_tx.try_send(payload) {
            crate::service_log!("WARN", "Dated carry paper audit queue unavailable: {error}");
        }
    }
    Ok(StatusCode::ACCEPTED)
}
