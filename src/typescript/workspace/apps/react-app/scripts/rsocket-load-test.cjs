const fs = require('node:fs');
const os = require('node:os');
const { BufferEncoders, RSocketClient } = require('rsocket-core');
const WebSocketClient = require('rsocket-websocket-client').default;

const url = getArgument('url', 'ws://127.0.0.1:10000');
const targets = getArgument('levels', '1,50,100,200,400')
    .split(',')
    .map(Number);
const durationMs = Number(getArgument('duration-ms', '10000'));
const serverPid = Number(getArgument('server-pid', '0'));
const requestBatchSize = 500;
const setup = {
    keepAlive: 30000,
    lifetime: 120000,
    dataMimeType: 'application/octet-stream',
    metadataMimeType: 'text/plain',
};

function getArgument(name, fallback) {
    const prefix = `--${name}=`;
    const value = process.argv.find((argument) => argument.startsWith(prefix));
    return value ? value.slice(prefix.length) : fallback;
}

function sleep(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

function processStats(pid) {
    if (!pid) return undefined;
    const stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(')') + 2).trim().split(/\s+/);
    const status = fs.readFileSync(`/proc/${pid}/status`, 'utf8');
    const residentKb = Number(status.match(/^VmRSS:\s+(\d+)\s+kB$/m)?.[1] ?? 0);
    return {
        cpuTicks: Number(fields[11]) + Number(fields[12]),
        residentKb,
    };
}

function connectClient() {
    return new Promise((resolve, reject) => {
        const transport = new WebSocketClient(
            { url, wsCreator: (address) => new WebSocket(address) },
            BufferEncoders,
        );
        const client = new RSocketClient({ setup, transport });
        let settled = false;
        const timeout = setTimeout(() => {
            if (!settled) {
                settled = true;
                client.close();
                reject(new Error(`Timed out connecting to ${url}`));
            }
        }, 15000);

        client.connect().subscribe({
            onError: (error) => {
                if (!settled) {
                    settled = true;
                    clearTimeout(timeout);
                    reject(error);
                }
            },
            onComplete: (socket) => {
                let subscription;
                const entry = { client, socket, messages: 0, bytes: 0, error: undefined };
                socket.requestStream({ data: Buffer.from('okx') }).subscribe({
                    onSubscribe: (request) => {
                        subscription = request;
                        request.request(requestBatchSize);
                        if (!settled) {
                            settled = true;
                            clearTimeout(timeout);
                            entry.close = () => {
                                subscription.cancel();
                                socket.close();
                                client.close();
                            };
                            resolve(entry);
                        }
                    },
                    onNext: (payload) => {
                        const length = payload.data?.length ?? 0;
                        if (length !== 48) {
                            entry.error = `Unexpected SBE payload length: ${length}`;
                            return;
                        }
                        entry.messages += 1;
                        entry.bytes += length;
                        if (entry.messages % requestBatchSize === 0) {
                            subscription.request(requestBatchSize);
                        }
                    },
                    onError: (error) => {
                        entry.error = error.message;
                        if (!settled) {
                            settled = true;
                            clearTimeout(timeout);
                            reject(error);
                        }
                    },
                    onComplete: () => {
                        entry.error = 'RSocket stream completed';
                    },
                });
            },
        });
    });
}

function percentile(sortedValues, quantile) {
    return sortedValues[Math.min(
        sortedValues.length - 1,
        Math.floor(sortedValues.length * quantile),
    )];
}

async function main() {
    if (targets.some((target) => !Number.isInteger(target) || target < 1)
        || !Number.isFinite(durationMs) || durationMs < 1000) {
        throw new Error('Use positive integer levels and --duration-ms >= 1000');
    }
    if (serverPid && !fs.existsSync(`/proc/${serverPid}/stat`)) {
        throw new Error(`No Linux process found for --server-pid=${serverPid}`);
    }

    const clients = [];
    const clockTicks = 100;
    console.log(`Connecting to ${url}; levels=${targets.join(',')}; duration=${durationMs}ms`);

    try {
        for (const target of targets) {
            const startedConnecting = performance.now();
            while (clients.length < target) {
                const batch = Math.min(25, target - clients.length);
                const outcomes = await Promise.allSettled(
                    Array.from({ length: batch }, () => connectClient()),
                );
                clients.push(...outcomes
                    .filter((outcome) => outcome.status === 'fulfilled')
                    .map((outcome) => outcome.value));
                const failure = outcomes.find((outcome) => outcome.status === 'rejected');
                if (failure) throw failure.reason;
                process.stdout.write(`\rConnected ${clients.length}/${target}`);
            }
            const connectSeconds = (performance.now() - startedConnecting) / 1000;
            await sleep(1000);

            const countsBefore = clients.map((client) => client.messages);
            const bytesBefore = clients.reduce((sum, client) => sum + client.bytes, 0);
            const serverBefore = processStats(serverPid);
            const clientCpuBefore = process.cpuUsage();
            const started = performance.now();
            await sleep(durationMs);
            const elapsedSeconds = (performance.now() - started) / 1000;
            const clientCpu = process.cpuUsage(clientCpuBefore);
            const serverAfter = processStats(serverPid);
            const counts = clients.map((client, index) => client.messages - countsBefore[index]);
            const sortedCounts = [...counts].sort((left, right) => left - right);
            const messages = counts.reduce((sum, count) => sum + count, 0);
            const bytes = clients.reduce((sum, client) => sum + client.bytes, 0) - bytesBefore;
            const cpuPercent = serverBefore && serverAfter
                ? ((serverAfter.cpuTicks - serverBefore.cpuTicks) / clockTicks / elapsedSeconds) * 100
                : undefined;
            const clientCpuPercent =
                ((clientCpu.user + clientCpu.system) / 1_000_000 / elapsedSeconds) * 100;
            const errors = clients.filter((client) => client.error).length;

            console.log(`\n${target} subscribers (${connectSeconds.toFixed(1)}s to add target)`);
            console.log(`  received: ${messages} messages (${(messages / elapsedSeconds).toFixed(0)}/s aggregate)`);
            console.log(`  per client: min ${sortedCounts[0]}, p50 ${percentile(sortedCounts, 0.5)}, `
                + `p95 ${percentile(sortedCounts, 0.95)}, max ${sortedCounts.at(-1)} per ${elapsedSeconds.toFixed(1)}s`);
            console.log(`  payload data: ${(bytes / elapsedSeconds / 1_000_000).toFixed(2)} MB/s `
                + `(${(bytes * 8 / elapsedSeconds / 1_000_000).toFixed(2)} Mb/s)`);
            console.log(`  server RSS: ${serverAfter?.residentKb ?? 'n/a'} KiB`
                + (serverBefore ? ` (${serverAfter.residentKb - serverBefore.residentKb >= 0 ? '+' : ''}`
                    + `${serverAfter.residentKb - serverBefore.residentKb} KiB during phase)` : ''));
            console.log(`  server CPU: ${cpuPercent === undefined ? 'n/a (pass --server-pid)' : `${cpuPercent.toFixed(1)}% of one core`}`);
            console.log(`  load-generator CPU: ${clientCpuPercent.toFixed(1)}% of one core; `
                + `${os.cpus().length} logical CPUs available`);
            console.log(`  clients with stream errors: ${errors}`);

            for (const client of clients) {
                if (client.error) console.error(`  client stream error: ${client.error}`);
            }
        }
    } finally {
        for (const client of clients) client.close();
        const cleanupDeadline = setTimeout(() => process.exit(process.exitCode ?? 0), 2000);
        cleanupDeadline.unref();
    }
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
