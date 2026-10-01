export type SymbolPriceTick = {
    symbol: string;
    price: number;
    direction: 'up' | 'down';
    receivedAt: number;
};

const asciiDecoder = new TextDecoder('ascii');

// Reusable buffer for symbol (fixed 8 bytes)
const symbolBuf = new Uint8Array(8);

export function decodeSymbolPrice(
    data: unknown,
    symbols?: Map<string, SymbolPriceTick>,
): SymbolPriceTick | undefined {
    if (!(data instanceof Uint8Array)) return;

    // Reuse DataView by storing it on the Uint8Array
    let view = (data as any)._view;
    if (!view) {
        view = new DataView(data.buffer, data.byteOffset, data.byteLength);
        (data as any)._view = view;
    }

    const templateId = view.getUint16(2, true);
    const schemaId   = view.getUint16(4, true);

    if (templateId !== SYMBOL_PRICE_TEMPLATE_ID ||
        schemaId   !== SYMBOL_PRICE_SCHEMA_ID) {
        return;
    }

    // symbol decode (no allocations)
    const symbolStart = HEADER_LENGTH;
    for (let i = 0; i < 8; i++) symbolBuf[i] = data[symbolStart + i];

    let symbol = asciiDecoder.decode(symbolBuf);
    const nullPos = symbol.indexOf('\0');
    if (nullPos !== -1) symbol = symbol.slice(0, nullPos);

    const priceOffset = HEADER_LENGTH + 8;
    const price = view.getFloat64(priceOffset, true);
    const prev = symbols?.get(symbol);
    const previousPrice = prev?.price;
    const direction: SymbolPriceTick['direction'] = previousPrice === undefined || price === previousPrice
        ? (prev?.direction) ?? 'up'
        : price > previousPrice ? 'up' : 'down';

    const tick = {
        symbol,
        price,
        direction,
        receivedAt: Date.now(),
    };

    symbols?.set(symbol, tick);
    return tick;
}
const HEADER_LENGTH = 8;
const SYMBOL_PRICE_TEMPLATE_ID = 1;
const SYMBOL_PRICE_SCHEMA_ID = 1;
