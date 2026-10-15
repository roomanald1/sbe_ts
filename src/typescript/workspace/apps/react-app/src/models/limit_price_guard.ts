const MAX_LIMIT_MARK_DEVIATION = 10;

export function getLimitPriceMarkWarning(
  limitPriceInput: string,
  mark?: number,
): string | undefined {
  const limitPrice = Number(limitPriceInput);
  if (
    !Number.isFinite(limitPrice) ||
    limitPrice <= 0 ||
    mark === undefined ||
    !Number.isFinite(mark) ||
    mark <= 0
  )
    return undefined;

  const deviation = Math.max(limitPrice / mark, mark / limitPrice);
  return deviation >= MAX_LIMIT_MARK_DEVIATION
    ? 'Limit price is at least 10× away from the mark. Check that quantity and price are in the correct fields.'
    : undefined;
}
