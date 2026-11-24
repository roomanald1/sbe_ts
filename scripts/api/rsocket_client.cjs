const path = require('node:path');
const fs = require('node:fs');
const { createRequire } = require('node:module');

const app = path.resolve(
    __dirname,
    '../../src/typescript/workspace/apps/react-app'
);

const appRequire = createRequire(
    path.join(app, 'package.json')
);

const {
    BufferEncoders,
    RSocketClient
} = appRequire('rsocket-core');

const WebSocketClient =
    appRequire('rsocket-websocket-client').default;

const services = {
  "learning": 10110,
  "autotrader": 10009,
  "insights": 10111,
  "positions": 10105
};
const allowedRoutes = {
  "learning": [
    "/summary",
    "/profiles",
    "/decision-snapshot"
  ],
  "autotrader": [
    "/internal/learning-feedback-queues",
    "/internal/monitoring-metrics",
    "/internal/strategy/open-position-counts"
  ],
  "insights": [
    "/signals/dashboard",
    "/signals/best",
    "/signals/auto-trader-decision-quality",
    "/signals/auto-trader-execution-quality"
  ],
  "positions": [
    "/positions",
    "/positions/account",
    "/positions/realized-pnl",
    "/positions/fee-summary",
    "/positions/fee-rates",
    "/positions/usd-rates",
    "/positions/barriers",
    "/internal/persistence-timing"
  ]
};
const allowed = Object.fromEntries(Object.entries(allowedRoutes).map(([k,v]) => [k,new Set(v)]));

async function main() {
    const input = JSON.parse(fs.readFileSync(0, 'utf8'));

    const maxBytes = Number(input.max_bytes ?? 4000000);
    if (!Number.isInteger(maxBytes) ||
        maxBytes < 1 || maxBytes > 64000000) {
        throw new Error('Invalid response size limit');
    }

    const service = input.service;
    let route = input.route;

    if (!services[service] ||
        !allowed[service].has(route)) {
        throw new Error('Route not in read-only allowlist');
    }

    if (input.since !== undefined) {
        const permitted = new Set([
            '/signals/auto-trader-decision-quality',
            '/signals/auto-trader-execution-quality'
        ]);

        if (service !== 'insights' || !permitted.has(route)) {
            throw new Error('Time window unsupported for this route');
        }

        if (!Number.isSafeInteger(input.since) ||
            input.since < Date.now() - 31 * 86400000 ||
            input.since > Date.now()) {
            throw new Error('Invalid since timestamp');
        }

        route += `?since=${input.since}`;
    }

    const url = `ws://127.0.0.1:${services[service]}`;

    const request = service === 'learning'
        ? {
            operation: 'learning_route',
            request: {
                request_id: `${Date.now()}:${Math.random()}`,
                strategy: null,
                method: 'get',
                path: route,
                payload: null
            }
        }
        : {
            method: 'GET',
            path: route
        };

    const transport = new WebSocketClient(
        {
            url,
            wsCreator: address => new WebSocket(address)
        },
        BufferEncoders
    );

    const client = new RSocketClient({
        setup: {
            keepAlive: 30000,
            lifetime: 120000,
            dataMimeType: 'application/octet-stream',
            metadataMimeType: 'text/plain'
        },
        transport
    });

    let socket;
    let cancelRequest;
    let cancelConnect;
    let timer;

    const start = Date.now();

    try {
        const response = await new Promise((resolve, reject) => {
            timer = setTimeout(
                () => reject(new Error('RSocket timeout')),
                8000
            );

            client.connect().subscribe({
                onSubscribe: cancel => {
                    cancelConnect = cancel;
                },
                onError: reject,
                onComplete: connected => {
                    socket = connected;

                    socket.requestResponse({
                        data: Buffer.from(JSON.stringify(request))
                    }).subscribe({
                        onSubscribe: cancel => {
                            cancelRequest = cancel;
                        },
                        onError: reject,
                        onComplete: payload => {
                            try {
                                const raw = Buffer.from(
                                    payload.data || []
                                );

                                if (raw.length > maxBytes) {
                                    throw new Error(
                                        `Response exceeds limit: ${raw.length} > ${maxBytes} bytes`
                                    );
                                }

                                resolve({
                                    service,
                                    route,
                                    elapsed_ms: Date.now() - start,
                                    bytes: raw.length,
                                    response: JSON.parse(
                                        raw.toString('utf8')
                                    )
                                });
                            } catch (error) {
                                reject(error);
                            }
                        }
                    });
                }
            });
        });

        process.stdout.write(JSON.stringify(response) + '\n');

    } finally {
        clearTimeout(timer);

        try {
            if (typeof cancelRequest === 'function')
                cancelRequest();
        } catch {}

        try {
            if (typeof cancelConnect === 'function')
                cancelConnect();
        } catch {}

        try { socket?.close(); } catch {}
        try { client.close(); } catch {}
    }
}

main().catch(error => {
    console.error(JSON.stringify({
        status: 'BLOCKED',
        error: error.message
    }));
    process.exitCode = 1;
});
