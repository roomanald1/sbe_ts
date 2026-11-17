import { useEffect, useState } from 'react';
import { paperExchangeFetch } from '../paper_exchange_api';
import {
  getInstrumentList,
  type InstrumentPrecision,
} from '../instrument_precision';
import { positionFetch } from '../position_api';

type AutoPosition = { symbol: string; quantity: number };
type OrderResult = {
  filled_quantity?: number;
  average_execution_price?: number;
};

async function waitForMarketBook(symbol: string): Promise<void> {
  const deadline = Date.now() + 10_000;
  let lastDetail = 'No fresh market book is available.';

  while (Date.now() < deadline) {
    const response = await paperExchangeFetch(
      `/internal/market-book?symbol=${encodeURIComponent(symbol)}`,
    );
    if (response.ok) return;

    const detail = (await response.text()).trim();
    if (response.status !== 503)
      throw new Error(
        detail || `Could not read the live market book (${response.status}).`,
      );

    lastDetail = detail || lastDetail;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  throw new Error(
    `Timed out waiting for a live market book for ${symbol}. ${lastDetail}`,
  );
}

async function readAutoPosition(symbol: string): Promise<number> {
  const response = await positionFetch('/positions?source=auto');
  if (!response.ok) throw new Error('Could not verify Auto positions.');
  const payload = (await response.json()) as { positions?: AutoPosition[] };
  return (
    payload.positions?.find((position) => position.symbol === symbol)
      ?.quantity ?? 0
  );
}

async function submitMarketOrder(
  symbol: string,
  side: 'buy' | 'sell',
  quantity: number,
  reduceOnly: boolean,
  clientOrderId: string,
): Promise<OrderResult> {
  const request = JSON.stringify({
    symbol,
    side,
    order_type: 'market',
    quantity,
    source: 'auto',
    reduce_only: reduceOnly,
    client_order_id: clientOrderId,
    measure_execution_quality: true,
  });
  for (let attempt = 0; ; attempt += 1) {
    const response = await paperExchangeFetch('/positions/orders', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: request,
    });
    if (response.ok) return (await response.json()) as OrderResult;

    const detail = (await response.text()).trim();
    if (detail.toLowerCase().includes('position ledger busy') && attempt < 5) {
      // Reuse the same client order ID so a delayed response cannot duplicate
      // a fill if the server committed before reporting contention.
      await new Promise((resolve) => setTimeout(resolve, 100 * (attempt + 1)));
      continue;
    }
    throw new Error(detail || `${side} order rejected (${response.status}).`);
  }
}

export function AutoTraderExecutionCheck() {
  const [instruments, setInstruments] = useState<InstrumentPrecision[]>([]);
  const [symbol, setSymbol] = useState('');
  const [side, setSide] = useState<'buy' | 'sell'>('buy');
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');

  useEffect(() => {
    let active = true;
    void getInstrumentList()
      .then(({ instruments: available }) => {
        if (!active) return;
        setInstruments(available);
        setSymbol((current) => current || available[0]?.symbol || '');
      })
      .catch(() => {
        if (active) setMessage('Instrument lot sizes are unavailable.');
      });
    return () => {
      active = false;
    };
  }, []);

  const selected = instruments.find(
    (instrument) => instrument.symbol === symbol,
  );

  async function runCheck() {
    if (!selected || busy) return;
    setBusy(true);
    setMessage('Checking that the symbol has no existing Auto position…');
    let entryFilled = 0;
    try {
      const existingQuantity = await readAutoPosition(symbol);
      if (Math.abs(existingQuantity) > 1e-12)
        throw new Error(
          `${symbol} already has an Auto position (${existingQuantity}). Close it before running the check.`,
        );

      const quantity = Number(selected.lotSz);
      if (!Number.isFinite(quantity) || quantity <= 0)
        throw new Error(`Invalid lot size for ${symbol}.`);

      setMessage(`Waiting for a live ${symbol} market book…`);
      await waitForMarketBook(symbol);

      const clientOrderId = `execution-check-${Date.now()}`;
      const entry = await submitMarketOrder(
        symbol,
        side,
        quantity,
        false,
        `${clientOrderId}-entry`,
      );
      entryFilled = entry.filled_quantity ?? 0;
      if (!Number.isFinite(entryFilled) || entryFilled <= 0)
        throw new Error(
          'Entry order filled nothing; no reverse order was needed.',
        );

      setMessage(
        `Entry filled ${entryFilled} ${symbol}; submitting reduce-only reverse…`,
      );
      const reverse = await submitMarketOrder(
        symbol,
        side === 'buy' ? 'sell' : 'buy',
        entryFilled,
        true,
        `${clientOrderId}-reverse`,
      );
      const remaining = await readAutoPosition(symbol);
      const reversed = reverse.filled_quantity ?? 0;
      if (Math.abs(remaining) <= 1e-12) {
        setMessage(
          `Execution check passed: opened ${entryFilled} and reversed ${reversed} ${symbol}; Auto position is zero.`,
        );
      } else {
        setMessage(
          `Reverse filled ${reversed} ${symbol}, but Auto position remains ${remaining}. Check the position and close it manually.`,
        );
      }
    } catch (error) {
      const detail =
        error instanceof Error ? error.message : 'Execution check failed.';
      setMessage(
        entryFilled > 0
          ? `Entry opened ${entryFilled} ${symbol}; verify and close any remaining Auto position. ${detail}`
          : detail,
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      className="auto-trader-execution-check"
      aria-label="Execution check"
    >
      <div>
        <strong>Execution check</strong>
        <span>Paper market order round trip using one exchange lot.</span>
      </div>
      <label>
        Instrument
        <select
          value={symbol}
          disabled={busy || instruments.length === 0}
          onChange={(event) => setSymbol(event.target.value)}
        >
          {instruments.map((instrument) => (
            <option key={instrument.symbol} value={instrument.symbol}>
              {instrument.symbol} · lot {instrument.lotSz}
            </option>
          ))}
        </select>
      </label>
      <label>
        First side
        <select
          value={side}
          disabled={busy}
          onChange={(event) => setSide(event.target.value as 'buy' | 'sell')}
        >
          <option value="buy">Buy, then sell</option>
          <option value="sell">Sell, then buy</option>
        </select>
      </label>
      <button
        type="button"
        disabled={busy || !selected}
        onClick={() => void runCheck()}
      >
        {busy ? 'Running…' : 'Run round trip'}
      </button>
      {message && <p role="status">{message}</p>}
    </section>
  );
}
