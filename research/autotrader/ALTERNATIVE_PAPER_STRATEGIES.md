# Five alternative AutoTrader paper strategies

Implemented 31 October 2026. These are experimental paper implementations, not reproductions of the published returns and not authenticated exchange/on-chain trading.

## What is operational in source

- AutoTrader owns five independent, persisted USD paper accounts, each initially $1,000. All start disabled. Their cash is simulated and does not consume the existing spot account or change its strategies.
- AutoTrader's global on/off switch gates new trades. Existing positions still receive marks and official settlements while paused. Disabling an individual strategy requests an orderly exit on the next active, complete observation, except redemption claims, which must mature.
- Each account has a trade cap, daily **realized** loss cap, explicit costs, a position ledger, marked equity, drawdown, durable opportunity deduplication and a bounded decision history.
- The AutoTrader view contains “Five alternative paper strategies”, with independent settings, data readiness, AG Grid account/position/decision views.
- **FeedHandlers → Insights → AutoTrader** is the native data path. FeedHandlers owns all external Binance, Deribit and Ethereum HTTP/RPC calls, source timestamps and short-lived raw caches. Insights computes lagged residual scores/rankings, option regime and contract selection, staking route economics, Aave health/net liquidation edge, and adaptive LP ticks/inside-range fees.
- Native feeds run every 30 seconds, including while strategies are disabled, to establish readiness. Each builder has a 25-second deadline. Independent strategy errors publish explicit unavailability; rejected observations cannot partially mutate an account.
- The UI shows each current insight and feed readiness. Enabling an account requires a healthy native insight with source and computation timestamps within 120 seconds. Disabling stays available during an outage. Healthy data with no profitable opportunity remains ready and waits.
- Shorts use a configurable simulated borrow APR. Options and chain execution remain paper; no private keys, signed transactions or authenticated order submission are used.
- Running services and existing runtime ledgers were not restarted or reconfigured.

These changes require deployment/restart before running binaries use them; follow the root service instructions rather than independently cycling services.

## Exact implemented rules

### Residual momentum

Eight fixed assets: BTC, ETH, BNB, XRP, SOL, ADA, DOGE, LTC, quoted in USDT and treated as USD for this paper model. Require 45 synchronized, contiguous closed daily candles. The common market factor is the equal-weight return of the entire supplied universe.

For each of the final 14 returns, fit an intercept and market beta using the preceding 30 daily returns; score the sum of the 14 residuals. Thus no return is used to estimate its own beta. At the first observation in a new Monday UTC execution week, buy the two highest-ranked assets and short the two lowest-ranked assets if simulated borrowing is allowed for both. If both sides cannot be formed, hold cash. Equal dollar legs, gross exposure at most account equity, with a fee reserve and configurable maximum per leg. Use executable bid/ask and reject the entire rebalance if any required side lacks depth.

Charge fees on absolute traded notional and short borrow over elapsed calendar time, including downtime. Mark long inventory at bid and short liabilities at ask. Default fees 15 bp per side; borrow APR 10%. Exits/rebalances use the weekly target; disabling or exceeding the realized loss cap flattens at the next active valid observation.

The 14-day formation and 7-day holding periods match the source's locked headline specification. This implementation uses daily OHLC, lagged OLS estimates, its documented universe and explicit borrowing costs; the source uses hourly observations and a different research process. It is not a replication of the author's 41.6% annualized result. The retained daily proxy replay is described below.

### Filtered BTC covered calls

One linear BTC/USDC Deribit call and its fully backing BTC position per account. Entry requires 21–45 days to expiry, delta 0.10–0.25, strike above the executable spot ask, prior BTC close at or below its prior SMA50, and current BTC DVOL above its previous 30 completed daily DVOL closes. DVOL is an ATM volatility regime proxy, not a history of the selected call's own IV. Indicators must precede the observation and be no more than two days old.

Buy BTC at ask and sell the call at bid, with fees on both instruments. Cap the BTC purchase by the configured trade cap and available cash, round down to the published lot step, and enforce the published minimum. New covered-call accounts default to a $1,000 trade cap so 0.01 BTC linear lots can fit; other new accounts default to $100. Existing saved limits are preserved. Mark BTC at bid and the short call at ask. Close both legs when delta reaches 0.50, when individually disabled and AutoTrader is active, or at expiry. An expired call requires an explicit sourced settlement; no inferred/last-traded settlement is allowed.

The native collector uses Deribit linear BTC/USDC books and Binance BTCUSDC spot depth, preserving a consistent USDC numeraire (treated as USD in this paper account). Entry/buyback fees use the instrument's published taker rate, capped at 12.5% of premium. Delivery uses the official delivery index/date and a 1.5 bp underlying fee capped at 12.5% of intrinsic value for these non-daily expiries. These follow [Deribit's fee schedule](https://support.deribit.com/hc/en-us/articles/25944746248989-Fees). Legacy inverse fixtures can still be normalized for replay; native new entries use linear contracts.

### Staking mint / redemption arbitrage

Inputs describe executable route amounts for an exact ETH input and token quantity, not a midpoint discount. Require quoted net output minus all route costs to exceed the configured edge buffer. The input notional plus costs must fit the account and per-trade cap.

Atomic mint/sell routes realize the supplied route's simulated net result immediately. Delayed purchase/redemption routes debit capital and entry costs but carry the pending claim at purchase cost: projected output is never booked as profit. A separate sourced claim observation after the maturity time realizes the actual supplied proceeds and exit costs, even while AutoTrader is paused.

The pending claim is **not continuously marked**. ETH moves, slashing or failed redemption can reduce actual proceeds; cost carrying is not a valuation guarantee. Settlement identity is scoped to chain and opportunity. The native route is Ethereum mainnet Lido stETH mint → Curve ETH/stETH sale. FeedHandlers runs the exact amount through a funded, pinned-block read-only EVM simulation; Insights converts measured proceeds and gas into net edge, including 15 bp per side paper fiat conversion costs. This collector does not open delayed redemption claims. The delayed-claim engine remains available for explicitly supplied legacy observations. No simulated result is represented as a mined transaction.

### Liquidation keeper

Accept only unhealthy positions (health factor below one), successful atomic simulations and executable collateral-sale quotes. Net proceeds must clear gas, swap fees, builder payment, flash-loan costs if applicable, and the edge buffer. Require funded principal and fees within account cash and the trade cap.

Record the complete simulated repayment/sale/cost result once per chain/opportunity id. Oracle collateral valuation alone is not a valid sale quote. The native feed discovers Aave V3 borrowers from recent Borrow events (initial last 8,192 blocks), continuously persists its cursor, and checks up to 32 borrowers per poll from a bounded 5,000-account watchlist. It currently supports USDC collateral / WETH debt. Optional AAVE_PAPER_WATCHLIST addresses supplement discovery. This is a recent borrower subset, not a complete Aave index.
    
FeedHandlers supplies a paper WETH balance through eth_call state overrides, then executes Aave liquidation and a Uniswap V3 USDC→WETH sale atomically. Insights uses actual repayment, sale proceeds and measured gas, plus paper fiat conversion costs. A revert is never treated as a successful liquidation. Block hashes, coverage, health counts and simulation counts are exposed as insight features. No blockchain transaction is sent.

### Adaptive stablecoin liquidity

One pool per account. Derive the half-width as twice prior daily realized volatility, clamped to 0.05%–2%, using indicators available before the current observation. Allocate at most 25% of paper equity and the configured trade cap. Block entry when either stablecoin is more than 2% from $1, when pool and USD quotes disagree, or round-trip gas exceeds 1% of allocated capital.

Use canonical Uniswap v3 liquidity-to-token formulas, including one-sided holdings outside the range. Accrue only the change in **cumulative inside-range fees per normalized liquidity unit**, checked against the exact held bounds. Mark token amounts at observed USD prices and reserve the observed exit cost. Close when price leaves the range, a depeg occurs, or the strategy is disabled while AutoTrader is active. A new observation with a matching new-range fee baseline is required to re-enter.

The native collector reads the Ethereum USDC/USDT Uniswap V3 pool, token order/decimals, slot0, tick spacing/bitmap, global fee counters and initialized boundary ticks at one block. Insights derives lagged daily volatility from completed hourly USDCUSDT candles, snaps bounds outward to actual initialized ticks (maximum 5% from current price), and computes exact modulo-U256 inside-range growth. Positions persist those exact bounds/tick IDs.
    
Fee baselines persist before delivery; a failed delivery retains cumulative fee increments for retry. Only fee increments are converted using Chainlink token/USD marks, avoiding revaluation of previously accumulated counters. Missing baselines are explicitly flagged and do not invent historical fees. Gas uses live gas price and conservative mint/burn/approval budgets; LP gas is modeled, unlike measured staking/liquidation gas. Fee income is counterfactual for a small paper LP; adding real liquidity could change pool fee shares. Pool-wide fee income cannot be substituted. This is a simple volatility-adaptive engineering rule, not the cited paper's fitted ML allocator or its historical performance.

## Native feeds and configuration

FeedHandlers provides:
    
- GET /market-data/alternative/binance — allowlisted depth and hourly/daily klines.
- GET /market-data/alternative/deribit — allowlisted public instruments, books, DVOL and official deliveries.
- POST /market-data/alternative/ethereum — read-only RPC and bounded batches/log ranges.
- GET /market-data/alternative/simulation-capability — checks read-only state-override support.
- POST /market-data/alternative/staking-simulation and /liquidation-simulation — fixed reviewed contract routes with bounded paper funding; the client cannot provide arbitrary state overrides.

Defaults require no keys: BINANCE_MARKET_DATA_URL=https://data-api.binance.vision, DERIBIT_MARKET_DATA_URL=https://www.deribit.com, ETHEREUM_RPC_URL=https://ethereum.publicnode.com. A custom Ethereum provider must support eth_call state overrides, web3_sha3, batches and historical reads/logs at the indexed blocks. Public-provider rate limits/outages make readiness unavailable and prevent entry.

Insights uses FEED_HANDLERS_URL (default http://127.0.0.1:3001), AUTO_TRADER_URL (default http://127.0.0.1:3005), and ALTERNATIVE_INSIGHTS_STATE_FILE (otherwise a .alternative-sources.json sibling of SIGNALS_DB_FILE / SIGNALS_FILE, or signals.sqlite). Source state contains Aave cursors/watchlist and LP fee baselines. Corrupt source state stops the worker without overwriting it.

The read-only EVM helper is in [PaperLiquidator.sol](../../src/rust/feed_handlers/src/alternative_market_data/PaperLiquidator.sol). It is injected only into eth_call, never deployed. The checked-in runtime is reproducible with official checksum-pinned solc 0.8.30, optimizer 200, viaIR and Paris EVM:

    python3 src/rust/feed_handlers/src/alternative_market_data/build_simulator.py /path/to/solc-0.8.30

## API and propagation boundary

- GET /positions/auto-trader/alternatives — account state and global pause flag.
- PUT /positions/auto-trader/alternatives/{strategy} — full strategy configuration, with the following JSON shape:

    {
      "enabled": false,
      "max_trade_usd": 100,
      "daily_loss_limit_usd": 25,
      "spot_fee_bps": 15,
      "short_borrow_apr": 0.10,
      "min_edge_bps": 10
    }

- POST /internal/auto-trader/alternative-observation — normalized observation. Strategy ids are residual-momentum, filtered-covered-call, staking-redemption-arbitrage, liquidation-keeper, adaptive-stablecoin-liquidity.

The complete wire contract is in [types.rs](../../src/rust/auto_trader/src/auto_trader/alternative_strategies/types.rs). Concrete **synthetic** lifecycle inputs for all five are in [alternative_observations.jsonl](./fixtures/alternative_observations.jsonl). The fixture's old timestamps are intentionally accepted only by offline replay, not the live endpoint.

Observations require id, source, observed_at_ms, strategy and data. The live endpoint rejects observations over 60 seconds old or from the future. Bid/ask observations must be no more than 30 seconds old. Histories must have contiguous, aligned prior timestamps. Updates are applied to a clone, atomically persisted and only then published; a bad observation or failed write cannot partially mutate the account. Corrupt persisted accounts fail closed without overwriting the damaged file.

Opportunity ids must identify the economic action (for example chain/loan/oracle-update or chain/route/block), not a random id generated for each poll. Observations are monotonic per strategy; retries of recent identical ids are idempotent. Successfully processed chain opportunities remain deduplicated across restarts. At 100,000 unique opportunities, new chain entries stop until an account is deliberately archived; settlements remain available. Recent decisions are bounded to 1,000 events/account.

POST /internal/auto-trader/alternative-insights accepts a current batch with as_of_ms, exactly five insights, and up to 32 normalized observations. Each insight contains strategy, ready, signal, source_timestamp_ms, computed_at_ms, summary and features. Duplicate/stale status batches are rejected. Observations apply independently on cloned sleeves; a rejected one marks that strategy unavailable without suppressing other accounts. A successful batch atomically persists feed status and accounting state together.

Insights audits the accepted readiness status and propagated/rejected observations. No optional external observation-URL collector is required. The legacy single-observation endpoint remains for trusted local/manual replay integration; the native worker uses the batch endpoint.

## Persistence and accounting limits

The alternative book is stored next to the configured AutoTrader binary state with the extension .alternatives.json. Tests use temporary paths and do not touch running accounts. Each account is isolated; these balances are not actual USD holdings and are not included in the existing spot P&L or budget tables.

The realized daily loss cap does not cap unrealized losses. Residual shorts have no intraday liquidation model. The spot market proxy is Binance, not OKX. No synthetic exchange outages, probabilistic partial fills, borrowing recalls or blockchain inclusion model are inferred from unavailable data. Under stale/missing observations, account equity is stale; pending redemptions are carried at cost and can be optimistic.

## Offline replay and validation

From the WSL repository root:

    cargo test --manifest-path src/rust/auto_trader/Cargo.toml
    cargo test --manifest-path src/rust/insights/Cargo.toml --bin insights alternative_insights
    cargo test --manifest-path src/rust/feed_handlers/Cargo.toml --bin feed_handlers alternative_market_data
    ./nx typecheck react-app

An explicit live read-only smoke test starts an ephemeral test listener using the production FeedHandlers router, computes all five Insights, and passes their observations into the actual paper engines. It does not cycle application services or submit orders:

    cargo test --manifest-path src/rust/insights/Cargo.toml --bin insights native_feeds_to_insights_to_paper_strategies -- --ignored --nocapture

The 31 October 2026 run passed all five data paths. Covered calls and LP were eligible; staking waited on negative net edge and Aave found no unhealthy loan among 32 scanned accounts. This verifies connectivity/normalization, not historical profitability or a successful real liquidation. Unit tests cover stale data, publication rejection isolation, readiness gating, lot sizes, tick bounds, fee counter wrapping and persistence.
    
Replay a captured normalized JSONL file without starting any service:

    cargo run --manifest-path src/rust/auto_trader/Cargo.toml -- \
      --replay-alternatives research/autotrader/fixtures/alternative_observations.jsonl \
      /tmp/alternative-paper-fixture-result.json

Replay enables only its local in-memory $1,000 accounts, uses the documented default parameters, and creates a new result file without overwriting an existing one. It preserves original observation timestamps and does not liquidate remaining positions at the end or fill missing marks. Inspect open positions and freshness before interpreting the ending equity.

The fixture is an accounting smoke test, **not historical market data or evidence of profitability**. Meaningful historical strategy evaluation still needs point-in-time source histories and untouched forward evaluation.

## Historical proxy result for the implemented residual engine

The implementation replay uses the retained eight-coin daily archive, next-open
execution with a modeled 5 bp half-spread, 15 bp fees per side and 10% short borrow
APR. Each period starts a fresh $1,000 account with a $100 cap per leg.

| Period | Net marked return | Maximum drawdown |
| --- | ---: | ---: |
| 2021–2022 | +23.36% | -37.87% |
| 2023–2024 | +6.17% | -25.60% |
| 2025–30 October 2026 | -0.81% | -8.53% |

These are the final **14-day formation / seven-day hold** results. An earlier
21-day prototype was corrected after checking the source's locked specification;
it is not the implemented rule. The eight-asset daily universe, lagged regression,
fixed-dollar position caps and explicit borrow cost still differ from the source.

Reproduce with:

    python3 research/autotrader/residual_momentum_implementation_replay.py

[Full results and input hashes](./residual_momentum_implementation_results.json)
include costs, marked open positions and a higher-exposure buy-and-hold comparator.
This is an overlapping, survivorship-biased discovery screen with modeled fills,
not a clean holdout. The recent period is negative. **Keep this candidate disabled;
the implementation does not establish a durable profitable edge.**
