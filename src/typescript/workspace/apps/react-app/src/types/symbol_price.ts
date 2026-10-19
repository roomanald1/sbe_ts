export type SymbolPriceTick = {
  symbol: string;
  price: number;
  direction: 'up' | 'down';
  volatility1m?: number;
  volatility5m?: number;
  receivedAt: number;
};

export type TradeTick = {
  symbol: string;
  tradeId: string;
  timestamp: number;
  price: number;
  size: number;
  side: 'buy' | 'sell';
};

export type BookLevel = {
  price: number;
  size: number;
  cumulativeSize: number;
  cumulativeNotional?: number;
};

export type BookDepth = {
  symbol: string;
  timestamp: number;
  bids: BookLevel[];
  asks: BookLevel[];
};

const asciiDecoder = new TextDecoder('ascii');
type CachedDataView = Uint8Array & { _view?: DataView };

// Reusable buffer for symbol (fixed 32 bytes)
const symbolBuf = new Uint8Array(32);

export function decodeSymbolPrice(
  data: unknown,
  symbols?: Map<string, SymbolPriceTick>,
): SymbolPriceTick | undefined {
  if (!(data instanceof Uint8Array)) return;
  return decodeSymbolPriceAt(data, symbols, 0);
}

export function decodeSymbolPriceBatch(
  data: unknown,
  symbols?: Map<string, SymbolPriceTick>,
): SymbolPriceTick[] {
  if (!(data instanceof Uint8Array)) return [];
  const ticks: SymbolPriceTick[] = [];
  for (let offset = 0; offset + SYMBOL_PRICE_MESSAGE_LENGTH <= data.byteLength; offset += SYMBOL_PRICE_MESSAGE_LENGTH) {
    const tick = decodeSymbolPriceAt(data, symbols, offset);
    if (tick) ticks.push(tick);
  }
  return ticks;
}

function decodeSymbolPriceAt(
  data: Uint8Array,
  symbols: Map<string, SymbolPriceTick> | undefined,
  offset: number,
): SymbolPriceTick | undefined {
  if (data.byteLength - offset < SYMBOL_PRICE_MESSAGE_LENGTH) return;

  // Reuse DataView by storing it on the Uint8Array
  const cachedData = data as CachedDataView;
  let view = cachedData._view;
  if (!view) {
    view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    cachedData._view = view;
  }

  const templateId = view.getUint16(offset + 2, true);
  const schemaId = view.getUint16(offset + 4, true);

  if (
    templateId !== SYMBOL_PRICE_TEMPLATE_ID ||
    schemaId !== SYMBOL_PRICE_SCHEMA_ID
  ) {
    return;
  }

  // symbol decode (no allocations)
  const symbolStart = offset + HEADER_LENGTH;
  for (let i = 0; i < 32; i++) symbolBuf[i] = data[symbolStart + i];

  let symbol = asciiDecoder.decode(symbolBuf).trimEnd();
  const nullPos = symbol.indexOf('\0');
  if (nullPos !== -1) symbol = symbol.slice(0, nullPos);

  const priceOffset = offset + HEADER_LENGTH + 32;
  const price = view.getFloat64(priceOffset, true);
  const rawVolatility1m = view.getFloat64(priceOffset + 8, true);
  const rawVolatility5m = view.getFloat64(priceOffset + 16, true);
  const volatility1m = Number.isFinite(rawVolatility1m) ? rawVolatility1m : undefined;
  const volatility5m = Number.isFinite(rawVolatility5m) ? rawVolatility5m : undefined;
  const prev = symbols?.get(symbol);
  const previousPrice = prev?.price;
  const direction: SymbolPriceTick['direction'] =
    previousPrice === undefined || price === previousPrice
      ? (prev?.direction ?? 'up')
      : price > previousPrice
        ? 'up'
        : 'down';

  const tick = {
    symbol,
    price,
    direction,
    volatility1m,
    volatility5m,
    receivedAt: Date.now(),
  };

  symbols?.set(symbol, tick);
  return tick;
}

export function decodeTradeTick(data: unknown): TradeTick | undefined {
  if (
    !(data instanceof Uint8Array) ||
    data.byteLength < TRADE_TICK_MESSAGE_LENGTH
  )
    return;

  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  if (
    view.getUint16(2, true) !== TRADE_TICK_TEMPLATE_ID ||
    view.getUint16(4, true) !== SYMBOL_PRICE_SCHEMA_ID ||
    view.getUint16(0, true) < TRADE_TICK_BLOCK_LENGTH
  ) {
    return;
  }

  const symbol = asciiDecoder
    .decode(data.subarray(HEADER_LENGTH, HEADER_LENGTH + SYMBOL_FIELD_LENGTH))
    .trimEnd();
  const tradeIdStart = HEADER_LENGTH + SYMBOL_FIELD_LENGTH;
  const tradeId = asciiDecoder
    .decode(data.subarray(tradeIdStart, tradeIdStart + TRADE_ID_FIELD_LENGTH))
    .trimEnd();
  const timestampOffset = tradeIdStart + TRADE_ID_FIELD_LENGTH;
  const timestamp = Number(view.getBigUint64(timestampOffset, true));
  const price = view.getFloat64(timestampOffset + 8, true);
  const size = view.getFloat64(timestampOffset + 16, true);
  const encodedSide = view.getUint8(timestampOffset + 24);
  if (
    !symbol ||
    !tradeId ||
    !Number.isSafeInteger(timestamp) ||
    !Number.isFinite(price) ||
    !Number.isFinite(size) ||
    (encodedSide !== 1 && encodedSide !== 2)
  ) {
    return;
  }

  return {
    symbol,
    tradeId,
    timestamp,
    price,
    size,
    side: encodedSide === 1 ? 'buy' : 'sell',
  };
}

export function decodeBookDepth(data: unknown): BookDepth | undefined {
  if (
    !(data instanceof Uint8Array) ||
    data.byteLength < BOOK_DEPTH_MESSAGE_LENGTH
  )
    return;

  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  if (
    view.getUint16(2, true) !== BOOK_DEPTH_TEMPLATE_ID ||
    view.getUint16(4, true) !== SYMBOL_PRICE_SCHEMA_ID ||
    view.getUint16(0, true) < BOOK_DEPTH_BLOCK_LENGTH
  ) {
    return;
  }

  const symbol = asciiDecoder
    .decode(data.subarray(HEADER_LENGTH, HEADER_LENGTH + 32))
    .trimEnd();
  const timestamp = Number(view.getBigUint64(HEADER_LENGTH + 32, true));
  if (!symbol || !Number.isSafeInteger(timestamp)) return;

  return {
    symbol,
    timestamp,
    bids: decodeBookSide(view, HEADER_LENGTH + 40),
    asks: decodeBookSide(view, HEADER_LENGTH + 40 + BOOK_DEPTH * 16),
  };
}

function decodeBookSide(view: DataView, firstPriceOffset: number): BookLevel[] {
  const levels: BookLevel[] = [];
  let cumulativeSize = 0;
  for (let index = 0; index < BOOK_DEPTH; index += 1) {
    const offset = firstPriceOffset + index * 16;
    const price = view.getFloat64(offset, true);
    const size = view.getFloat64(offset + 8, true);
    if (
      !Number.isFinite(price) ||
      !Number.isFinite(size) ||
      price <= 0 ||
      size <= 0
    )
      continue;
    cumulativeSize += size;
    levels.push({ price, size, cumulativeSize });
  }
  return levels;
}

const HEADER_LENGTH = 8;
const SYMBOL_PRICE_MESSAGE_LENGTH = HEADER_LENGTH + 32 + 24;
const SYMBOL_PRICE_TEMPLATE_ID = 1;
const SYMBOL_PRICE_SCHEMA_ID = 1;
const BOOK_DEPTH_TEMPLATE_ID = 2;
const TRADE_TICK_TEMPLATE_ID = 3;
const BOOK_DEPTH_BLOCK_LENGTH = 264;
const BOOK_DEPTH_MESSAGE_LENGTH = HEADER_LENGTH + BOOK_DEPTH_BLOCK_LENGTH;
const BOOK_DEPTH = 7;
const SYMBOL_FIELD_LENGTH = 32;
const TRADE_ID_FIELD_LENGTH = 32;
const TRADE_TICK_BLOCK_LENGTH = 89;
const TRADE_TICK_MESSAGE_LENGTH = HEADER_LENGTH + TRADE_TICK_BLOCK_LENGTH;
