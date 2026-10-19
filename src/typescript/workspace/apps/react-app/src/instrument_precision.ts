export type InstrumentPrecision = { symbol: string; tickSz: string; lotSz: string };
let precisionCache: Map<string, InstrumentPrecision> | undefined;
let pending: Promise<Map<string, InstrumentPrecision>> | undefined;

export function registerInstrumentPrecisions(items: InstrumentPrecision[]): void {
  if (items.length > 0) precisionCache = new Map(items.map((item) => [item.symbol, item]));
}

export function getCachedInstrumentPrecision(symbol: string): InstrumentPrecision | undefined { return precisionCache?.get(symbol); }

export function formatForSymbol(value: number | undefined, symbol: string, kind: 'price' | 'quantity'): string {
  return formatInstrumentValue(value, kind, getCachedInstrumentPrecision(symbol));
}

export async function getInstrumentPrecision(symbol: string): Promise<InstrumentPrecision | undefined> {
  if (!precisionCache) {
    pending ??= fetch('/instruments').then(async (response) => {
      if (!response.ok) throw new Error('Instrument precision unavailable');
      const body = await response.json() as { instruments?: InstrumentPrecision[] };
      registerInstrumentPrecisions(body.instruments ?? []);
      return precisionCache!;
    }).finally(() => { pending = undefined; });
    try { await pending; } catch { return undefined; }
  }
  return precisionCache?.get(symbol);
}

function fractionDigits(step: string): number {
  const normalized = step.toLowerCase();
  const [coefficient, exponentText] = normalized.split('e');
  const decimals = (coefficient.split('.')[1] ?? '').length;
  return Math.max(0, Math.min(12, decimals - Number(exponentText ?? 0)));
}

export function formatInstrumentValue(value: number | undefined, kind: 'price' | 'quantity', precision?: InstrumentPrecision): string {
  if (value == null || !Number.isFinite(value)) return '—';
  const step = kind === 'price' ? precision?.tickSz : precision?.lotSz;
  const digits = step ? fractionDigits(step) : (kind === 'price' ? (Math.abs(value) >= 1 ? 2 : 6) : 6);
  return new Intl.NumberFormat(undefined, { minimumFractionDigits: 0, maximumFractionDigits: digits }).format(value);
}

export function alignsToStep(value: number, stepText: string): boolean {
  const digits = fractionDigits(stepText);
  const scale = 10 ** digits;
  const step = Number(stepText) * scale;
  const scaled = value * scale;
  return Number.isFinite(step) && step > 0 && Math.abs(scaled / step - Math.round(scaled / step)) < 1e-8;
}
