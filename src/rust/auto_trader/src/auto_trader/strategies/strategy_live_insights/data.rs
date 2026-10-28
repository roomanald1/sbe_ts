use crate::auto_trader::LiveInsightField;

pub(super) const SUBSCRIPTIONS: &[LiveInsightField] = &[
    LiveInsightField::Momentum10s,
    LiveInsightField::Trend5m,
    LiveInsightField::TradeFlow,
    LiveInsightField::OrderBook,
    LiveInsightField::VwapDislocation,
    LiveInsightField::AbsorptionPressure,
];
