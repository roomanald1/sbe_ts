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
  onSetTab,
  onSetGroupBy,
}: {
  state: CapitalAllocationWidgetState;
  onSetTab: (tab: CapitalAllocationWidgetState['tab']) => void;
  onSetGroupBy: (groupBy: CapitalAllocationWidgetState['groupBy']) => void;
}) {
  const autonomous = state.capitalAllocation?.model === 'shared_pool';
  return (
    <OperatorPanel
      hideTitle
      title="Autonomous capital allocation"
      context="Paper · shared pool"
      status={
        <OperatorStatus
          tone={state.loading ? 'neutral' : autonomous ? 'positive' : 'warning'}
        >
          {state.loading
            ? 'Loading'
            : autonomous
              ? 'Autonomous'
              : 'Allocation unavailable'}
        </OperatorStatus>
      }
    >
      <CapitalAllocationView
        state={state}
        onSetTab={onSetTab}
        onSetGroupBy={onSetGroupBy}
      />
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
  return (
    <CapitalAllocationPanel
      state={state}
      onSetTab={(tab) => model.setTab(tab)}
      onSetGroupBy={(groupBy) => model.setGroupBy(groupBy)}
    />
  );
}
