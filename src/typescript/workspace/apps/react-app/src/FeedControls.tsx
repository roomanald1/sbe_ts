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
    const [dataView, setDataView] = useState<'grid' | 'table' | 'chart' | 'book' | 'user'>('grid');
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
            </nav>

            {dataView === 'grid' && (
                <SymbolPriceGrid
                    ticks={priceChanges.ticks}
                    feedSource={feedSource}
                    onSelectSymbol={selectSymbol}
                />
            )}
            {dataView === 'table' && (
                <SymbolPriceTable
                    ticks={priceChanges.ticks}
                    state={connectionState}
                    onSelectSymbol={selectSymbol}
                />
            )}
            {dataView === 'chart' && (
                <PriceChangeChart
                    changes={priceChanges}
                    onSelectSymbol={selectSymbol}
                />
            )}
            {dataView === 'book' && feedSource === 'okx' && (
                <OrderBookPopup
                    mode="view"
                    symbol={selectedBookSymbol}
                    symbols={bookSymbols}
                    onSymbolChange={setBookSymbol}
                    connection={connection}
                />
            )}
            {dataView === 'user' && <Users />}
        </>
    );
}
