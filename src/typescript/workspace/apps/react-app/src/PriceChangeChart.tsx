import { useMemo, useState } from 'react';
import { createPriceChartData, type PriceChanges } from './price_change_data';
import { PriceChangeChartView } from './PriceChangeChartView';

type PriceChangeChartProps = {
    changes: PriceChanges;
    onSelectSymbol?: (symbol: string) => void;
    initialSymbol?: string;
};

export function PriceChangeChart({ changes, onSelectSymbol, initialSymbol }: PriceChangeChartProps) {
    const [selectedSymbol, setSelectedSymbol] = useState(initialSymbol ?? '');
    const data = useMemo(
        () => createPriceChartData(changes.ticks, changes.histories, selectedSymbol),
        [changes, selectedSymbol],
    );

    return (
        <PriceChangeChartView
            data={data}
            onSymbolChange={setSelectedSymbol}
            onSelectSymbol={onSelectSymbol}
        />
    );
}
