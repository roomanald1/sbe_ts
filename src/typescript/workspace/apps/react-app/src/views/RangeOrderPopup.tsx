import { useState, type FormEvent } from 'react';
import { createPortal } from 'react-dom';

type RangeOrderRequest = { symbol: string; lower: number; upper: number };
type OrderResult = { side: 'buy' | 'sell'; ok: boolean; message: string };

export function RangeOrderPopup({ request, onClose }: { request: RangeOrderRequest; onClose: () => void }) {
  const [quantity, setQuantity] = useState('');
  const [lower, setLower] = useState(String(request.lower));
  const [upper, setUpper] = useState(String(request.upper));
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<OrderResult[]>([]);
  const [error, setError] = useState('');

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const parsedQuantity = Number(quantity);
    const buyPrice = Number(lower);
    const sellPrice = Number(upper);
    if (!Number.isFinite(parsedQuantity) || parsedQuantity <= 0 || !Number.isFinite(buyPrice) || buyPrice <= 0 || !Number.isFinite(sellPrice) || sellPrice <= buyPrice) {
      setError('Enter a positive quantity and valid prices with the sell above the buy.');
      return;
    }
    setBusy(true);
    setError('');
    setResults([]);
    const placements = await Promise.all(([
      { side: 'buy' as const, limit_price: buyPrice },
      { side: 'sell' as const, limit_price: sellPrice },
    ]).map(async ({ side, limit_price }): Promise<OrderResult> => {
      try {
        const response = await fetch('/positions/orders', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ symbol: request.symbol, side, order_type: 'limit', quantity: parsedQuantity, limit_price }),
        });
        if (!response.ok) return { side, ok: false, message: (await response.text()).trim() || 'HTTP ' + response.status };
        const order = await response.json() as { id?: string; status?: string };
        return { side, ok: true, message: ('Order ' + (order.id ?? '') + ' ' + (order.status ?? 'submitted')).trim() };
      } catch (reason) {
        return { side, ok: false, message: reason instanceof Error ? reason.message : 'Request failed' };
      }
    }));
    setResults(placements);
    setBusy(false);
  };

  return createPortal(<div className="trade-popup-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget && !busy) onClose(); }}>
    <section className="trade-popup range-order-popup" role="dialog" aria-modal="true" aria-label={'Range orders for ' + request.symbol}>
      <header><div><span>RANGE ORDERS</span><h2>{request.symbol}</h2></div><button type="button" aria-label="Close range order dialog" onClick={onClose} disabled={busy}>×</button></header>
      <form onSubmit={(event) => void submit(event)}>
        <p className="range-order-note">Review both independent limit orders. They are not linked as OCO; either or both can fill. The sell may require existing base-currency inventory.</p>
        <label>Quantity for each order<input type="number" min="0" step="any" value={quantity} onChange={(event) => setQuantity(event.target.value)} required /></label>
        <label>Buy limit at lower range bound<input type="number" min="0" step="any" value={lower} onChange={(event) => setLower(event.target.value)} required /></label>
        <label>Sell limit at upper range bound<input type="number" min="0" step="any" value={upper} onChange={(event) => setUpper(event.target.value)} required /></label>
        {error && <p className="positions-error" role="alert">{error}</p>}
        {results.length > 0 && <ul className="range-order-results">{results.map((result) => <li key={result.side} className={result.ok ? 'range-order-success' : 'positions-error'}>{result.side.toUpperCase()}: {result.message}</li>)}</ul>}
        <footer><button type="button" onClick={onClose} disabled={busy}>{results.length ? 'Done' : 'Cancel'}</button><button type="submit" disabled={busy}>{busy ? 'Submitting…' : 'Place both limit orders'}</button></footer>
      </form>
    </section>
  </div>, document.body);
}
