import { useEffect, useMemo } from 'react';
import {
  OperatorPanel,
  OperatorStatus,
  useBehaviorSnapshot,
} from '@workspace/ui-platform';
import { sharedAutoTraderSettingsModel } from '../models/AutoTraderModel';
import {
  CapitalAllocationWidgetModel,
  type CapitalAllocationWidgetState,
} from '../models/CapitalAllocationWidgetModel';
import { CapitalAllocationView } from './AutoTraderAllocationView';

export function CapitalAllocationPanel({
  state,
}: {
  state: CapitalAllocationWidgetState;
}) {
  const autonomous = state.capitalAllocation?.model === 'shared_pool';
  return (
    <OperatorPanel
      title="Capital allocation"
      context="All eligible strategies, symbols and horizons"
      status={
        <OperatorStatus tone={autonomous ? 'positive' : 'warning'}>
          {autonomous ? 'Autonomous' : 'Allocation contract unavailable'}
        </OperatorStatus>
      }
    >
      <CapitalAllocationView
        allocation={state.capitalAllocation}
        availableNotional={state.availableNotional}
      />
      <p>
        Global order, daily loss and position limits remain in Auto Trader
        Controls.
      </p>
    </OperatorPanel>
  );
}

export function CapitalAllocationWidget({
  widgetId = 'capital-allocation',
}: {
  widgetId?: string;
}) {
  const model = useMemo(
    () =>
      new CapitalAllocationWidgetModel(widgetId, sharedAutoTraderSettingsModel),
    [widgetId],
  );
  useEffect(() => {
    model.start();
    return () => model.stop();
  }, [model]);
  const state = useBehaviorSnapshot(model.state$);
  return <CapitalAllocationPanel state={state} />;
}
