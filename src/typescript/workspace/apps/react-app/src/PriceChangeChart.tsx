import { useMemo, useState } from 'react';
import { createPriceChartData, type PriceChanges } from './price_change_data';
import { PriceChangeChartView } from './PriceChangeChartView';

type PriceChangeChartProps = {
    changes: PriceChanges;
    onSelectSymbol?: (symbol: string) => void;
    initialSymbol?: string;
    selectedSymbol?: string;
    onSymbolChange?: (symbol: string) => void;
    showSymbolSelector?: boolean;
};

export function PriceChangeChart({ changes, onSelectSymbol, initialSymbol, selectedSymbol: controlledSymbol, onSymbolChange, showSymbolSelector = true }: PriceChangeChartProps) {
    const [localSymbol, setLocalSymbol] = useState(initialSymbol ?? '');
    const selectedSymbol = controlledSymbol ?? localSymbol;
    const changeSymbol = onSymbolChange ?? setLocalSymbol;
    const data = useMemo(
        () => createPriceChartData(changes.ticks, changes.histories, selectedSymbol),
        [changes, selectedSymbol],
    );

    return (
        <PriceChangeChartView
            data={data}
            onSymbolChange={changeSymbol}
            onSelectSymbol={onSelectSymbol}
            showSymbolSelector={showSymbolSelector}
        />
    );
}
