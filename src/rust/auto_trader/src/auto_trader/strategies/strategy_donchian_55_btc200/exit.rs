pub(super) fn should_exit_when_insight_inactive(entry_insight_active: bool) -> bool {
    !entry_insight_active
}
pub(super) fn should_exit_when_disabled(enabled: bool) -> bool {
    !enabled
}
pub(super) fn description() -> &'static str {
    "exit when BTC regime or the 55-day breakout insight becomes inactive"
}
