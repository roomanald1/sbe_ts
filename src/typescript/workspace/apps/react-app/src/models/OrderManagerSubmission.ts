import { alignsToStep, getInstrumentPrecision } from '../instrument_precision';
import type { OrderManagerState } from './OrderManagerTypes';

export type OrderSubmissionValues = {
  quantity: number;
  limitPrice: number;
  barrierPrice: number;
  durationMinutes: number;
  participationRate: number;
};

type ValidationResult = { values: OrderSubmissionValues } | { error: string } | undefined;

export async function validateOrderSubmission(state: OrderManagerState): Promise<ValidationResult> {
  const values = {
    quantity: Number(state.quantity),
    limitPrice: Number(state.limitPrice),
    barrierPrice: Number(state.barrierPrice),
    durationMinutes: Number(state.durationMinutes),
    participationRate: Number(state.participationRate) / 100,
  };
  const instrument = await getInstrumentPrecision(state.selectedSymbol);
  if (instrument && (
    !alignsToStep(values.quantity, instrument.lotSz) ||
    (state.orderType === 'limit' && !alignsToStep(values.limitPrice, instrument.tickSz)) ||
    (state.orderType === 'barrier' && !alignsToStep(values.barrierPrice, instrument.tickSz))
  )) return { error: 'Quantity or price does not match this instrument tick or lot size.' };

  const isInvalidVwap = state.orderType === 'vwap' && (
    !Number.isInteger(values.durationMinutes) || values.durationMinutes < 5 || values.durationMinutes > 390 ||
    !Number.isFinite(values.participationRate) || values.participationRate < 0.01 || values.participationRate > 1
  );
  const isInvalidPrice = state.orderType === 'limit'
    ? !Number.isFinite(values.limitPrice) || values.limitPrice <= 0
    : state.orderType === 'barrier' && (!Number.isFinite(values.barrierPrice) || values.barrierPrice <= 0);
  if (!state.selectedSymbol || !Number.isFinite(values.quantity) || values.quantity <= 0 || isInvalidPrice || isInvalidVwap) return;
  return { values };
}

export function orderSubmissionBody(state: OrderManagerState, values: OrderSubmissionValues) {
  const isVwap = state.orderType === 'vwap';
  return {
    symbol: state.selectedSymbol,
    side: state.side,
    ...(isVwap
      ? { duration_minutes: values.durationMinutes, max_participation_rate: values.participationRate }
      : { order_type: state.orderType }),
    quantity: values.quantity,
    ...(state.orderType === 'limit' ? { limit_price: values.limitPrice } : {}),
    ...(state.orderType === 'barrier'
      ? { trigger_price: values.barrierPrice, trigger_direction: state.barrierDirection }
      : {}),
  };
}
