//! VWAP execution scheduling primitives.
//!
//! The scheduler turns a parent quantity and historical volume curve into
//! child-order targets.  adds live-volume pacing and benchmark
//! tracking; an execution service is responsible for routing returned actions.

/// Number of bins used by the standard six-and-a-half-hour session curve.
pub const NUM_BINS: usize = 78;

#[derive(Clone, Copy, Debug)]
pub struct VwapConfig {
    pub parent_qty: f64,
    /// Per-bin historical weights. Values should be non-negative and sum to 1.
    pub historical_vwap_curve: [f64; NUM_BINS],
    /// Maximum fraction of observed bin volume to participate in (0, 1].
    pub max_participation_rate: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VwapState {
    pub current_bin: usize,
    pub executed_qty: f64,
    pub remaining_qty: f64,
}

#[derive(Clone, Debug)]
pub struct VwapScheduler {
    config: VwapConfig,
    curve_total: f64,
    state: VwapState,
}

impl VwapScheduler {
    pub fn new(config: VwapConfig) -> Result<Self, &'static str> {
        if !config.parent_qty.is_finite() || config.parent_qty <= 0.0 {
            return Err("Parent quantity must be finite and positive");
        }
        if config
            .historical_vwap_curve
            .iter()
            .any(|w| !w.is_finite() || *w < 0.0)
        {
            return Err("VWAP curve weights must be finite and non-negative");
        }
        let total: f64 = config.historical_vwap_curve.iter().sum();
        if !total.is_finite() || total <= 0.0 {
            return Err("VWAP curve must have a positive total weight");
        }
        if !config.max_participation_rate.is_finite()
            || !(0.0..=1.0).contains(&config.max_participation_rate)
            || config.max_participation_rate == 0.0
        {
            return Err("Maximum participation rate must be in (0, 1]");
        }
        Ok(Self {
            config,
            curve_total: total,
            state: VwapState {
                current_bin: 0,
                executed_qty: 0.0,
                remaining_qty: config.parent_qty,
            },
        })
    }

    pub fn state(&self) -> VwapState {
        self.state
    }

    /// Returns the scheduled cumulative target for this bin, capped by observed
    /// volume participation and the parent remainder.
    pub fn calculate_child_order(
        &self,
        live_bin_volume: f64,
        expected_bin_volume: f64,
        elapsed_bin_fraction: f64,
    ) -> f64 {
        let bin = self.state.current_bin;
        if bin >= NUM_BINS || self.state.remaining_qty <= 0.0 {
            return 0.0;
        }
        let planned_bin_qty = if expected_bin_volume.is_finite() && expected_bin_volume > 0.0 {
            expected_bin_volume * self.config.max_participation_rate
        } else {
            self.config.parent_qty * self.config.historical_vwap_curve[bin]
                / self.curve_total
        };
        let target = planned_bin_qty * elapsed_bin_fraction.clamp(0.0, 1.0);
        let cap = live_bin_volume * self.config.max_participation_rate;
        target.min(cap).min(self.state.remaining_qty)
    }

    pub fn on_fill(&mut self, filled_qty: f64) {
        let filled = filled_qty.max(0.0).min(self.state.remaining_qty);
        self.state.executed_qty += filled;
        self.state.remaining_qty -= filled;
    }

    pub fn advance_bin(&mut self) {
        self.state.current_bin = (self.state.current_bin + 1).min(NUM_BINS);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OrderAction {
    NewOrder { qty: f64 },
    ModifyOrder { new_qty: f64 },
    CancelOrder,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VwapBenchmark {
    pub market_num: f64,
    pub market_den: f64,
    pub exec_num: f64,
    pub exec_den: f64,
}

impl VwapBenchmark {
    pub fn update_market(&mut self, price: f64, qty: f64) {
        if price.is_finite() && price > 0.0 && qty.is_finite() && qty > 0.0 {
            self.market_num += price * qty;
            self.market_den += qty;
        }
    }
    pub fn update_exec(&mut self, price: f64, qty: f64) {
        if price.is_finite() && price > 0.0 && qty.is_finite() && qty > 0.0 {
            self.exec_num += price * qty;
            self.exec_den += qty;
        }
    }
    pub fn market_vwap(&self) -> f64 {
        if self.market_den > 0.0 {
            self.market_num / self.market_den
        } else {
            0.0
        }
    }
    pub fn execution_vwap(&self) -> f64 {
        if self.exec_den > 0.0 {
            self.exec_num / self.exec_den
        } else {
            0.0
        }
    }
    pub fn slippage(&self) -> f64 {
        self.execution_vwap() - self.market_vwap()
    }
}

/// Stateful tick-to-order adapter. Returned actions must be sent to an execution gateway.
#[derive(Clone, Debug)]
pub struct OrderManager {
    scheduler: VwapScheduler,
    expected_bin_volumes: [f64; NUM_BINS],
    raw_bin_volume: f64,
    filled_in_current_bin: f64,
    working_order_qty: f64,
    min_trade_threshold: f64,
    benchmark: VwapBenchmark,
    tick_counter: u64,
    bin_duration_secs: f64,
}

impl OrderManager {
    pub fn new(
        config: VwapConfig,
        expected_bin_volumes: [f64; NUM_BINS],
        min_trade_threshold: f64,
        bin_duration_secs: f64,
    ) -> Result<Self, &'static str> {
        if !bin_duration_secs.is_finite() || bin_duration_secs <= 0.0 {
            return Err("Bin duration must be finite and positive");
        }
        if !min_trade_threshold.is_finite() || min_trade_threshold < 0.0 {
            return Err("Trade threshold must be finite and non-negative");
        }
        if expected_bin_volumes
            .iter()
            .any(|volume| !volume.is_finite() || *volume < 0.0)
        {
            return Err("Expected bin volumes must be finite and non-negative");
        }
        Ok(Self {
            scheduler: VwapScheduler::new(config)?,
            expected_bin_volumes,
            raw_bin_volume: 0.0,
            filled_in_current_bin: 0.0,
            working_order_qty: 0.0,
            min_trade_threshold,
            benchmark: VwapBenchmark::default(),
            tick_counter: 0,
            bin_duration_secs,
        })
    }

    pub fn state(&self) -> VwapState {
        self.scheduler.state()
    }
    pub fn benchmark(&self) -> VwapBenchmark {
        self.benchmark
    }

    /// elapsed_bin_secs is elapsed time in the active bin.
    pub fn on_market_trade(
        &mut self,
        trade_qty: f64,
        trade_price: f64,
        elapsed_bin_secs: f64,
    ) -> Option<OrderAction> {
        if !trade_qty.is_finite()
            || trade_qty <= 0.0
            || !trade_price.is_finite()
            || trade_price <= 0.0
            || !elapsed_bin_secs.is_finite()
        {
            return None;
        }
        self.tick_counter = self.tick_counter.wrapping_add(1);
        if self.tick_counter & 0b1111 == 0 {
            self.benchmark.update_market(trade_price, trade_qty);
        }
        let bin = self.scheduler.state.current_bin;
        if bin >= NUM_BINS {
            return None;
        }
        self.raw_bin_volume += trade_qty;
        let elapsed_fraction = (elapsed_bin_secs / self.bin_duration_secs).clamp(0.0, 1.0);
        let target = self.scheduler.calculate_child_order(
            self.raw_bin_volume,
            self.expected_bin_volumes[bin],
            elapsed_fraction,
        );
        let net_target = (target - self.filled_in_current_bin).max(0.0);
        let drift = (net_target - self.working_order_qty).abs();
        if drift < self.min_trade_threshold {
            return None;
        }
        let action = match (self.working_order_qty, net_target) {
            (qty0, qty) if qty0 == 0.0 && qty > 0.0 => Some(OrderAction::NewOrder { qty }),
            (qty, qty0) if qty > 0.0 && qty0 == 0.0 => Some(OrderAction::CancelOrder),
            (_, qty) => Some(OrderAction::ModifyOrder { new_qty: qty }),
        };
        self.working_order_qty = net_target;
        action
    }

    pub fn on_execution_fill(&mut self, fill_price: f64, filled_qty: f64) {
        let actual = filled_qty.max(0.0).min(self.working_order_qty);
        self.scheduler.on_fill(actual);
        self.benchmark.update_exec(fill_price, actual);
        self.filled_in_current_bin += actual;
        self.working_order_qty -= actual;
    }

    pub fn on_order_complete(&mut self) {
        self.working_order_qty = 0.0;
    }

    pub fn on_bin_transition(&mut self) {
        self.scheduler.advance_bin();
        self.raw_bin_volume = 0.0;
        self.filled_in_current_bin = 0.0;
        self.working_order_qty = 0.0;
    }
}
