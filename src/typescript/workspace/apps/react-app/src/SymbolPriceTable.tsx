import { memo } from 'react';
import type { ConnectionState } from './connection';
import type { SymbolPriceTick } from './types/symbol_price';

const priceFormatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
});

const timeFormatter = new Intl.DateTimeFormat(undefined, {
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit',
});

type SymbolPriceTableProps = {
    ticks: SymbolPriceTick[];
    state: ConnectionState;
};

const SymbolPriceRow = memo(function SymbolPriceRow({ tick }: { tick: SymbolPriceTick }) {
    return (
        <tr>
            <td><span className="symbol-tag">{tick.symbol}</span></td>
            <td className={`price-cell price-${tick.direction}`}>{priceFormatter.format(tick.price)}</td>
            <td className="time-cell">{timeFormatter.format(tick.receivedAt)}</td>
        </tr>
    );
});

export function SymbolPriceTable({ ticks, state }: SymbolPriceTableProps) {
    return (
        <section className="feed-section" aria-label="Live symbol price events">
            <div className="feed-toolbar">
                <div>
                    <span className="live-indicator" />
                    <span className="toolbar-title">Incoming events</span>
                </div>
                <span className="toolbar-meta">LATEST TICKS</span>
            </div>
            <div className="table-scroll">
                <table>
                    <thead>
                        <tr>
                            <th scope="col">SYMBOL</th>
                            <th scope="col">PRICE</th>
                            <th scope="col">RECEIVED AT</th>
                        </tr>
                    </thead>
                    <tbody>
                        {ticks.length === 0 ? (
                            <tr>
                                <td className="empty-state" colSpan={3}>
                                    {state === 'connected' ? 'Waiting for the first SBE message…' : 'Connecting to the publisher…'}
                                </td>
                            </tr>
                        ) : ticks.map(tick => <SymbolPriceRow key={tick.symbol} tick={tick} />)}
                    </tbody>
                </table>
            </div>
        </section>
    );
}