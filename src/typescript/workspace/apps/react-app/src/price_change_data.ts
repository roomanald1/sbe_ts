import { useMemo } from 'react';
import { bufferTime, filter, map, of, switchMap, type Observable } from 'rxjs';
import type { Connection } from './connection';
import { decodeSymbolPrice, type SymbolPriceTick } from './types/symbol_price';
import { useObservable } from './use_observable';

export type PriceHistorySample = {
    timestamp: number;
    price: number;
};

export type PriceChanges = {
    ticks: SymbolPriceTick[];
    histories: Map<string, PriceHistorySample[]>;
};

export type PriceChartData = {
    symbols: string[];
    symbol: string;
    history: PriceHistorySample[];
    latestPrice?: number;
    priceChange?: number;
    percentChange?: number;
    minPrice?: number;
    maxPrice?: number;
    startTime?: number;
    endTime?: number;
};

const HISTORY_SAMPLE_INTERVAL_MS = 1_000;
const MAX_HISTORY_SAMPLES = 1_800;
const HISTORY_TRIM_BATCH_SIZE = 100;

function createPriceChangesObservable(connection: Connection): Observable<PriceChanges> {
    let symbols = new Map<string, SymbolPriceTick>();
    let symbolIndices = new Map<string, number>();
    const histories = new Map<string, PriceHistorySample[]>();
    const lastSampleTimes = new Map<string, number>();
    let ticks: SymbolPriceTick[] = [];

    return connection.connection_state.pipe(
        switchMap(state => state === 'connected'
            ? connection.data.pipe(
                map(data => decodeSymbolPrice(data, symbols)),
                filter((tick): tick is SymbolPriceTick => tick !== undefined),
                bufferTime(50),
                filter(changedTicks => changedTicks.length > 0),
                map(changedTicks => ({ reset: false as const, changedTicks })),
            )
            : of({ reset: true as const, changedTicks: [] as SymbolPriceTick[] })),
        map(update => {
            if (update.reset) {
                symbols = new Map<string, SymbolPriceTick>();
                symbolIndices = new Map<string, number>();
                ticks = [];
                return { ticks, histories };
            }

            update.changedTicks.forEach(tick => {
                let index = symbolIndices.get(tick.symbol);
                if (index === undefined) {
                    index = ticks.length;
                    symbolIndices.set(tick.symbol, index);
                }

                const latestTick = symbols.get(tick.symbol);
                if (!latestTick) return;
                if (ticks[index]?.price !== latestTick.price) {
                    ticks[index] = latestTick;
                }

                const lastSampleTime = lastSampleTimes.get(tick.symbol);
                if (lastSampleTime === undefined || tick.receivedAt - lastSampleTime >= HISTORY_SAMPLE_INTERVAL_MS) {
                    let history = histories.get(tick.symbol);
                    if (!history) {
                        history = [];
                        histories.set(tick.symbol, history);
                    }
                    history.push({ timestamp: tick.receivedAt, price: tick.price });
                    lastSampleTimes.set(tick.symbol, tick.receivedAt);
                    if (history.length >= MAX_HISTORY_SAMPLES + HISTORY_TRIM_BATCH_SIZE) {
                        history.splice(0, HISTORY_TRIM_BATCH_SIZE);
                    }
                }
            });
            return { ticks, histories };
        }),
    );
}

export function usePriceChanges(connection: Connection): PriceChanges {
    const observable = useMemo(() => createPriceChangesObservable(connection), [connection]);
    return useObservable(
        observable,
        { ticks: [], histories: new Map<string, PriceHistorySample[]>() },
        [observable],
    );
}

export function createPriceChartData(
    ticks: SymbolPriceTick[],
    histories: Map<string, PriceHistorySample[]>,
    selectedSymbol: string,
): PriceChartData {
    const symbols = ticks.map(tick => tick.symbol);
    const symbol = symbols.includes(selectedSymbol) ? selectedSymbol : symbols[0] ?? '';
    const history = symbol ? histories.get(symbol) ?? [] : [];
    const firstPrice = history[0]?.price;
    const latestPrice = ticks.find(tick => tick.symbol === symbol)?.price ?? history.at(-1)?.price;
    const priceChange = firstPrice !== undefined && latestPrice !== undefined
        ? latestPrice - firstPrice
        : undefined;
    const prices = history.map(sample => sample.price);

    return {
        symbols,
        symbol,
        history,
        latestPrice,
        priceChange,
        percentChange: firstPrice && priceChange !== undefined
            ? (priceChange / firstPrice) * 100
            : undefined,
        minPrice: prices.length ? Math.min(...prices) : undefined,
        maxPrice: prices.length ? Math.max(...prices) : undefined,
        startTime: history[0]?.timestamp,
        endTime: history.at(-1)?.timestamp,
    };
}
