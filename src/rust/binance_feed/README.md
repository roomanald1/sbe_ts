# Binance Feed

Binance Feed retrieves daily candles from Binance's public market-data API. Insights is its only application caller and connects over RSocket; no internal HTTP candle API is provided. The HTTP listener on port `3012` exposes only service health and generated operational documentation. The RSocket WebSocket listener defaults to port `10002` (`RSOCKET_PORT` can override it) and is intended for internal service traffic.

## RSocket contract

Use request-response with JSON payloads:

```json
{"type":"daily_candles","symbols":["BTCUSDT","ETHUSDT"]}
```

The response is a JSON object keyed by the requested symbol. Each value is an array of completed daily candles with `close_time_ms`, `high`, and `close`. Supported symbols are the daily strategy universe configured by the Insights application. Unsupported symbols and upstream Binance failures are returned as RSocket errors.
