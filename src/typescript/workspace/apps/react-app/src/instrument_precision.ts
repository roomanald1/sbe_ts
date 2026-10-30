export type InstrumentPrecision = { symbol: string; tickSz: string; lotSz: string };
export type InstrumentList = { symbols: string[]; instruments: InstrumentPrecision[] };
let precisionCache: Map<string, InstrumentPrecision> | undefined;
let pending: Promise<Map<string, InstrumentPrecision>> | undefined;
let instrumentList: InstrumentList | undefined;
let instrumentListPending: Promise<InstrumentList> | undefined;
let instrumentListFetchedAt = 0;

export async function getInstrumentList(): Promise<InstrumentList> {
  if (instrumentList && Date.now() - instrumentListFetchedAt < 15 * 60 * 1000) return instrumentList;
  instrumentListPending ??= fetch('/instruments').then(async (response) => {
    if (!response.ok) throw new Error('Could not load instruments');
    const body = await response.json() as { symbols?: unknown; instruments?: unknown };
    const symbols = Array.isArray(body.symbols) ? body.symbols.filter((symbol): symbol is string => typeof symbol === 'string') : [];
    const instruments = Array.isArray(body.instruments) ? body.instruments as InstrumentPrecision[] : [];
    instrumentList = { symbols, instruments };
    instrumentListFetchedAt = Date.now();
    registerInstrumentPrecisions(instruments);
    return instrumentList;
  }).finally(() => { instrumentListPending = undefined; });
  return instrumentListPending;
}

export function registerInstrumentPrecisions(items: InstrumentPrecision[]): void {
  if (items.length > 0) precisionCache = new Map(items.map((item) => [item.symbol, item]));
}

export function getCachedInstrumentPrecision(symbol: string): InstrumentPrecision | undefined { return precisionCache?.get(symbol); }

export function formatForSymbol(value: number | undefined, symbol: string, kind: 'price' | 'quantity'): string {
  return formatInstrumentValue(value, kind, getCachedInstrumentPrecision(symbol));
}

export async function getInstrumentPrecision(symbol: string): Promise<InstrumentPrecision | undefined> {
  if (!precisionCache) {
    pending ??= getInstrumentList().then(() => precisionCache!).finally(() => { pending = undefined; });
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
