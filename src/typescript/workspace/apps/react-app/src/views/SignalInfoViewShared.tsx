import { useEffect, useMemo } from 'react';
import { useBehaviorSnapshot } from '@workspace/ui-platform';
import {
  ClientSideRowModelModule,
  ModuleRegistry,
  themeQuartz,
} from 'ag-grid-community';
import { SignalInfoModel } from '../models/SignalInfoModel';
ModuleRegistry.registerModules([ClientSideRowModelModule]);
export const gridTheme = themeQuartz.withParams({
  accentColor: '#8296ad',
  backgroundColor: '#0d0f12',
  borderColor: '#242a31',
  foregroundColor: '#e7e7e7',
  headerBackgroundColor: '#11151a',
  headerTextColor: '#a0a6ad',
  rowHoverColor: '#141b24',
});
export const percent = (value?: number | null) =>
  value == null ? 'N/A' : value.toFixed(3) + '%';
export const ratio = (value?: number | null) =>
  value == null ? 'N/A' : (value * 100).toFixed(1) + '%';
export const horizonLabel = (value: unknown) =>
  (
    ({
      60: '1 minute',
      300: '5 minutes',
      900: '15 minutes',
      3600: '1 hour',
    }) as Record<number, string>
  )[Number(value)] ?? String(value) + ' seconds';
export const signedValueClasses = {
  'signal-value-positive': (p: { value: unknown }) =>
    typeof p.value === 'number' && p.value > 0,
  'signal-value-negative': (p: { value: unknown }) =>
    typeof p.value === 'number' && p.value < 0,
};
export const directionBadge = ({ value }: { value: unknown }) => {
  const d =
    value === 'bullish' || value === 'bearish' || value === 'ranging'
      ? value
      : 'neutral';
  return (
    <span className={'signal-badge signal-badge-' + d}>
      {String(value ?? 'neutral').toUpperCase()}
    </span>
  );
};
export const strengthRenderer = ({ value }: { value: unknown }) => {
  const n = typeof value === 'number' ? Math.max(0, Math.min(1, value)) : 0;
  const l = n >= 0.5 ? 'high' : n >= 0.25 ? 'medium' : 'low';
  return (
    <span className={'signal-strength signal-strength-' + l}>
      <span className="signal-strength-track">
        <span style={{ width: n * 100 + '%' }} />
      </span>
      <b>{(n * 100).toFixed(0)}%</b>
    </span>
  );
};
export function useSignalInfo() {
  const model = useMemo(() => new SignalInfoModel(), []);
  const state = useBehaviorSnapshot(model.state$);
  useEffect(() => {
    void model.start();
    return () => model.stop();
  }, [model]);
  return state;
}
