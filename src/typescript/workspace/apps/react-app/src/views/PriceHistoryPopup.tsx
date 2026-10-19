import { createPortal } from 'react-dom';
import type { PriceHistorySample } from '../price_change_data';
import type { SymbolPriceTick } from '../types/symbol_price';
import { PriceChangeChart } from './PriceChangeChart';

export function PriceHistoryPopup({ symbol, history, onClose }: { symbol: string; history: PriceHistorySample[]; onClose: () => void }) {
  const latest = history.at(-1);
  const tick: SymbolPriceTick = {
    symbol,
    price: latest?.price ?? 0,
    direction: latest && history[0] && latest.price < history[0].price ? 'down' : 'up',
    receivedAt: latest?.timestamp ?? Date.now(),
  };
  const changes = { ticks: [tick], histories: new Map([[symbol, history]]) };
  return createPortal(<div className="trade-popup-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
    <section className="trade-popup price-history-popup" role="dialog" aria-modal="true" aria-label={symbol + ' price trend'}>
      <header className="price-history-popup-header"><button type="button" aria-label="Close price chart" onClick={onClose}>×</button></header>
      <PriceChangeChart changes={changes} selectedSymbol={symbol} showSymbolSelector={false} />
    </section>
  </div>, document.body);
}
