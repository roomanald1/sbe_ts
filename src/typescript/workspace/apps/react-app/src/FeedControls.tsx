import { useState } from 'react';
import type { Connection, ConnectionState, FeedSource } from './connection';
import { SymbolPriceGrid } from './SymbolPriceGrid';
import { SymbolPriceTable } from './SymbolPriceTable';
import { PriceChangeChart } from './PriceChangeChart';
import { Users } from './Users';
import { OrderBookPopup } from './OrderBookPopup';
import { usePriceChanges } from './price_change_data';

type FeedControlsProps = {
    connection: Connection;
    feedSource: FeedSource;
    connectionState: ConnectionState;
    onFeedSourceChange: (source: FeedSource) => void;
    onSelectSymbol: (symbol: string) => void;
};

export function FeedControls({
    connection,
    feedSource,
    connectionState,
    onFeedSourceChange,
    onSelectSymbol,
}: FeedControlsProps) {
    const [dataView, setDataView] = useState<'all' | 'grid' | 'table' | 'chart' | 'book' | 'user'>('grid');
    const priceChanges = usePriceChanges(connection);
    const selectSymbol = feedSource === 'okx' ? onSelectSymbol : undefined;
    const [bookSymbol, setBookSymbol] = useState('');
    const bookSymbols = priceChanges.ticks.map(tick => tick.symbol);
    const selectedBookSymbol = bookSymbols.includes(bookSymbol) ? bookSymbol : bookSymbols[0] ?? '';

    return (
        <>
            <nav className="feed-controls" aria-label="Main navigation">
                <div className="view-switcher" role="group" aria-label="Main sections">
                    <button type="button" aria-pressed={dataView !== 'user'} onClick={() => setDataView('grid')}>Markets</button>
                    <button type="button" aria-pressed={dataView === 'user'} onClick={() => setDataView('user')}>Users</button>
                </div>
                {dataView !== 'user' && <div className="view-switcher" role="group" aria-label="Choose price feed">
                    <button
                        type="button"
                        aria-pressed={feedSource === 'okx'}
                        onClick={() => onFeedSourceChange('okx')}
                    >
                        OKX
                    </button>
                    <button
                        type="button"
                        aria-pressed={feedSource === 'synthetic'}
                        onClick={() => { onFeedSourceChange('synthetic'); if (dataView === 'book') setDataView('grid'); }}
                    >
                        Synthetic · Fast
                    </button>
                </div>}
                {dataView !== 'user' && <div className="view-switcher" role="group" aria-label="Choose market view">
                    <button
                        type="button"
                        aria-pressed={dataView === 'all'}
                        onClick={() => setDataView('all')}
                    >
                        All
                    </button>
                    <button
                        type="button"
                        aria-pressed={dataView === 'grid'}
                        onClick={() => setDataView('grid')}
                    >
                        Grid
                    </button>
                    <button
                        type="button"
                        aria-pressed={dataView === 'table'}
                        onClick={() => setDataView('table')}
                    >
                        Table
                    </button>
                    <button
                        type="button"
                        aria-pressed={dataView === 'chart'}
                        onClick={() => setDataView('chart')}
                    >
                        Chart
                    </button>
                    <button
                        type="button"
                        aria-pressed={dataView === 'book'}
                        onClick={() => setDataView('book')}
                        disabled={feedSource !== 'okx'}
                    >
                        Book
                    </button>
                </div>}
                {dataView === 'all' && <label className="chart-symbol-control all-symbol-control">
                    <span>SYMBOL</span>
                    <select aria-label="All views symbol" value={selectedBookSymbol} onChange={event => setBookSymbol(event.target.value)}>
                        {bookSymbols.map(item => <option key={item} value={item}>{item}</option>)}
                    </select>
                </label>}
            </nav>

            <div className={dataView === 'all' ? 'market-dashboard' : 'market-view-stack'}>
            {(dataView === 'grid' || dataView === 'all') && (
                <SymbolPriceGrid
                    ticks={priceChanges.ticks}
                    feedSource={feedSource}
                    compact={dataView === 'all'}
                />
            )}
            {(dataView === 'table' || dataView === 'all') && (
                <SymbolPriceTable
                    ticks={priceChanges.ticks}
                    state={connectionState}
                />
            )}
            {(dataView === 'chart' || dataView === 'all') && (
                <PriceChangeChart
                    changes={priceChanges}
                    onSelectSymbol={dataView === 'all' ? undefined : selectSymbol}
                    selectedSymbol={dataView === 'all' ? selectedBookSymbol : undefined}
                    onSymbolChange={dataView === 'all' ? setBookSymbol : undefined}
                    showSymbolSelector={dataView !== 'all'}
                />
            )}
            {(dataView === 'book' || dataView === 'all') && feedSource === 'okx' && (
                <OrderBookPopup
                    mode="view"
                    symbol={selectedBookSymbol}
                    symbols={bookSymbols}
                    onSymbolChange={dataView === 'all' ? undefined : setBookSymbol}
                    showSymbolSelector={dataView !== 'all'}
                    connection={connection}
                />
            )}
            {(dataView === 'book' || dataView === 'all') && feedSource !== 'okx' && (
                <section className="feed-section" aria-label="Order book">
                    <div className="book-popup-message">Order book depth is available with the OKX feed.</div>
                </section>
            )}
            </div>
            {dataView === 'user' && <Users />}
        </>
    );
}
