export type SymbolPriceTick = {
    symbol: string;
    price: number;
    receivedAt: Date;
};

const asciiDecoder = new TextDecoder('ascii');

export function decodeSymbolPrice(
    data: unknown,
    symbols?: Map<string, SymbolPriceTick>,
): SymbolPriceTick | undefined {
    if (!(data instanceof Uint8Array)) {
       console.error('Expected binary SBE payload (Uint8Array)');
       return undefined;
    }
    if (data.byteLength < HEADER_LENGTH + SYMBOL_PRICE_BLOCK_LENGTH) {
        console.log(`SBE payload too short: received ${data.byteLength} bytes`);
        return undefined;
    }

    const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    const blockLength = view.getUint16(0, true);
    const templateId = view.getUint16(2, true);
    const schemaId = view.getUint16(4, true);

    if (templateId !== SYMBOL_PRICE_TEMPLATE_ID || schemaId !== SYMBOL_PRICE_SCHEMA_ID) {
        throw new Error(`Unexpected SBE message (schema ${schemaId}, template ${templateId})`);
    }
    if (blockLength < SYMBOL_PRICE_BLOCK_LENGTH || data.byteLength < HEADER_LENGTH + blockLength) {
        throw new RangeError(`Unexpected SBE block length: ${blockLength}`);
    }

    const symbolBytes = data.subarray(HEADER_LENGTH, HEADER_LENGTH + 8);
    const symbol = asciiDecoder.decode(symbolBytes).replace(/\0+$/, '');

    const tick = {
        symbol,
        price: view.getFloat64(HEADER_LENGTH + 8, true),
        receivedAt: new Date(),
    };

    symbols?.set(symbol, tick);
    return tick;
}

const HEADER_LENGTH = 8;
const SYMBOL_PRICE_BLOCK_LENGTH = 16;
const SYMBOL_PRICE_TEMPLATE_ID = 1;
const SYMBOL_PRICE_SCHEMA_ID = 1;
