pub(super) fn should_exit_when_insight_inactive(active: bool) -> bool {
    !active
}
pub(super) fn should_exit_when_disabled(enabled: bool) -> bool {
    !enabled
}
pub(crate) fn uses_protective_stops() -> bool {
    false
}
pub(super) fn description() -> &'static str {
    "exit when the 20-day breakout or BTC SMA200 regime becomes inactive, after the minimum 72-hour hold"
}
