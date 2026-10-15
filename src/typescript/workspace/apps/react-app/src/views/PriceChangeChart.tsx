import { useEffect, useMemo } from 'react';
import type { PriceChanges } from '../price_change_data';
import { PriceChangeChartModel } from '../models/PriceChangeChartModel';
import { useBehaviorSnapshot } from '../use_behavior_snapshot';
import { PriceChangeChartView } from './PriceChangeChartView';

type PriceChangeChartProps = {
  changes: PriceChanges;
  initialSymbol?: string;
  selectedSymbol?: string;
  onSymbolChange?: (symbol: string) => void;
  showSymbolSelector?: boolean;
  onTrade?: (side: 'buy' | 'sell') => void;
};
export function PriceChangeChart({
  changes,
  initialSymbol,
  selectedSymbol,
  onSymbolChange,
  showSymbolSelector = true,
  onTrade,
}: PriceChangeChartProps) {
  const model = useMemo(
    () =>
      new PriceChangeChartModel({
        changes,
        initialSymbol,
        selectedSymbol,
        onSymbolChange,
      }),
    [],
  );
  const data = useBehaviorSnapshot(model.state$, (state) => state.data);
  const chartSize = useBehaviorSnapshot(model.state$, (state) => state.chartSize);
  useEffect(() => {
    model.updateProps({
      changes,
      initialSymbol,
      selectedSymbol,
      onSymbolChange,
    });
  }, [model, changes, initialSymbol, selectedSymbol, onSymbolChange]);
  return (
    <PriceChangeChartView
      data={data}
      chartSize={chartSize}
      onResize={(width, height) => model.setChartSize(width, height)}
      onSymbolChange={(symbol) => model.selectSymbol(symbol)}
      showSymbolSelector={showSymbolSelector}
      onTrade={onTrade}
    />
  );
}


