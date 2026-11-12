import http from 'node:http';
import net from 'node:net';
import { readFile, stat } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const exec = promisify(execFile);
const root = process.env.WORKSPACE_ROOT || '/workspace';
const homepageDist = resolve(root, 'src/typescript/workspace/apps/homepage/dist');
const services = ['feed-handlers','binance-feed','position-management','execution','insights','insights-maintenance','auto-trader','news-sentiment','learning','web'];
const send = (res, code, value, type='application/json') => { res.writeHead(code, {'content-type': `${type}; charset=utf-8`}); res.end(type.startsWith('application/json') ? JSON.stringify(value) : value); };
const contentType = (file) => ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon' }[file.slice(file.lastIndexOf('.'))] || 'application/octet-stream');
const compose = async (project, file, args) => {
  const composeArgs = [...args];
  const targetedScoutService = composeArgs.some((arg) => arg.startsWith('strategy-x-scout-'));
  if (project === 'sbe-system' && (process.env.SCOUT_WORKERS_ENABLED === 'true' || targetedScoutService) && !composeArgs.includes('--profile')) composeArgs.unshift('--profile', 'scout-workers');
  const { stdout, stderr } = await exec('docker', ['compose','-p',project,'-f',file,'--progress=quiet',...composeArgs], {
    cwd:root,
    timeout: 15 * 60 * 1000,
    maxBuffer: 5 * 1024 * 1024,
    env: {...process.env, REPO_ROOT: process.env.DOCKER_WORKSPACE_ROOT || process.env.WORKSPACE_ROOT || root}
  });
  return stdout || stderr || 'Done';
};
const getStrategyServices = async () => (await compose('sbe-system','system/compose.yml',['--profile','scout-workers','config','--services'])).split('\n').filter((service) => service.startsWith('strategy-x-live-') || service.startsWith('strategy-x-scout-'));
http.createServer(async (req,res) => {
  try {
    const url = new URL(req.url, 'http://localhost');
    if (req.method === 'GET' && url.pathname === '/api/status') {
      const [system, infra] = await Promise.all([
        compose('sbe-system','system/compose.yml',['ps','--format','json']),
        compose('sbe-infra','monitoring/docker-compose.yml',['ps','--format','json'])
      ]);
      return send(res,200,{system,infra});
    }
    if (req.method === 'GET' && url.pathname === '/api/liveness') {
      return send(res,200,livenessResult);
    }
    if (req.method === 'POST' && url.pathname === '/api/liveness/run') {
      livenessResult = await runLivenessChecks();
      return send(res,livenessResult.ok ? 200 : 503,livenessResult);
    }
    if (req.method === 'POST' && url.pathname === '/api/action') {
      let body=''; for await (const chunk of req) body += chunk;
      const {project, action, service} = JSON.parse(body);
      if (project === 'all' && !service && ['start','stop','restart'].includes(action)) {
        const verb = action === 'start' ? 'up' : action === 'stop' ? 'stop' : 'restart';
        const flags = action === 'start' ? ['-d','--remove-orphans'] : [];
        const first = await compose('sbe-system','system/compose.yml',[verb,...flags]);
        const second = await compose('sbe-infra','monitoring/docker-compose.yml',[verb,...flags]);
        return send(res,200,{output:`${first}\n${second}`});
      }
      if (!['system','infra'].includes(project) || !['start','stop','restart','rebuild-restart'].includes(action)) return send(res,400,{error:'Invalid action'});
      if (project === 'infra' && service && !['prometheus','loki','alloy','node-exporter','app-exporter','grafana'].includes(service)) return send(res,400,{error:'Unknown infra service'});
      if (project === 'system' && service && !services.includes(service) && !(await getStrategyServices()).includes(service)) return send(res,400,{error:'Unknown system service'});
      const file = project === 'system' ? 'system/compose.yml' : 'monitoring/docker-compose.yml';
      let args;
      if (action === 'rebuild-restart') {
        if (!service) return send(res,400,{error:'Rebuild and restart requires a single service'});
        args = ['up','-d','--build','--no-deps',service];
      }
      else if (action === 'start') {
        args = ['up','-d',...(project === 'system' && !service ? ['--remove-orphans'] : []),...(service ? [service] : [])];
        if (project === 'system' && service === 'learning' && process.env.SCOUT_WORKERS_ENABLED !== 'false') {
          const scoutWorkers = (await getStrategyServices()).filter((name) => name.startsWith('strategy-x-scout-'));
          args.push(...scoutWorkers);
        }
      }
      else if (action === 'stop') args = ['stop',...(service ? [service] : [])];
      else args = ['restart',...(service ? [service] : [])];
      return send(res,200,{output:await compose(project === 'system' ? 'sbe-system' : 'sbe-infra',file,args)});
    }
    if (req.method === 'GET' && (url.pathname === '/' || url.pathname.startsWith('/assets/'))) {
      const requested = url.pathname === '/' ? 'index.html' : decodeURIComponent(url.pathname.slice(1));
      const file = resolve(homepageDist, requested);
      if (!file.startsWith(`${homepageDist}${sep}`)) return send(res,403,{error:'Forbidden'});
      const info = await stat(file);
      if (!info.isFile()) return send(res,404,{error:'Not found'});
      res.writeHead(200, {'content-type': `${contentType(file)}; charset=utf-8`});
      return res.end(await readFile(file));
    }
    if (req.method === 'GET' && url.pathname.startsWith('/coverage/')) {
      const relativePath = decodeURIComponent(url.pathname.slice('/coverage'.length));
      const file = resolve('/app/coverage', `.${relativePath}${relativePath.endsWith('/') ? 'index.html' : ''}`);
      if (!file.startsWith(`/app/coverage${sep}`)) return send(res,403,{error:'Forbidden'});
      const content = await readFile(file);
      const type = file.endsWith('.html') ? 'text/html' : file.endsWith('.css') ? 'text/css' : file.endsWith('.js') ? 'text/javascript' : 'application/octet-stream';
      res.writeHead(200, {'content-type':type}); return res.end(content);
    }
    if (req.method === 'GET' && (url.pathname === '/benchmarks' || url.pathname.startsWith('/benchmarks/'))) {
      const upstream = http.request({
        hostname: 'benchmarks-ui',
        port: 5175,
        path: `${url.pathname}${url.search}`,
        method: req.method,
        headers: { ...req.headers, host: 'localhost:5175' },
      }, (response) => {
        res.writeHead(response.statusCode || 502, response.headers);
        response.pipe(res);
      });
      upstream.on('error', (error) => {
        if (res.headersSent) return res.destroy(error);
        const relativePath = decodeURIComponent(url.pathname.slice('/benchmarks'.length));
        const file = resolve('/app/benchmarks', `.${relativePath}${relativePath.endsWith('/') ? 'index.html' : ''}`);
        if (!file.startsWith(`/app/benchmarks${sep}`)) return send(res,403,{error:'Forbidden'});
        readFile(file).then((content) => {
          const type = file.endsWith('.html') ? 'text/html' : file.endsWith('.css') ? 'text/css' : file.endsWith('.js') ? 'text/javascript' : 'application/octet-stream';
          res.writeHead(200, {'content-type':type});
          res.end(content);
        }).catch(() => send(res,502,{error:`Benchmarks dev server unavailable: ${error.message}`}));
      });
      req.pipe(upstream);
      return;
    }
    if (req.method === 'GET' && !url.pathname.startsWith('/api/')) {
      const index = resolve(homepageDist, 'index.html');
      const info = await stat(index);
      if (info.isFile()) {
        res.writeHead(200, {'content-type':'text/html; charset=utf-8'});
        return res.end(await readFile(index));
      }
    }
    send(res,404,{error:'Not found'});
  } catch (error) { send(res,500,{error:error.message}); }
}).listen(8080,'0.0.0.0');

let livenessResult = { ok: false, checkedAt: null, checks: [], summary: 'Not run yet' };

async function runLivenessChecks() {
  const host = process.env.LIVENESS_HOST || 'host.docker.internal';
  const get = async (port, path) => {
    const response = await fetch(`http://${host}:${port}${path}`, { signal: AbortSignal.timeout(5000) });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const contentType = response.headers.get('content-type') || '';
    return contentType.includes('json') ? response.json() : response.text();
  };
  const checkTcp = (port) => new Promise((resolve, reject) => {
    const socket = net.createConnection({ host, port });
    socket.setTimeout(5000);
    socket.once('connect', () => { socket.destroy(); resolve('Accepting connections'); });
    socket.once('timeout', () => socket.destroy(new Error('Connection timed out')));
    socket.once('error', reject);
  });
  const getHtml = (port, path = '/') => new Promise((resolve, reject) => {
    const request = http.get({ hostname: host, port, path, headers: { host: `localhost:${port}` }, timeout: 5000 }, (response) => {
      let body = '';
      response.setEncoding('utf8');
      response.on('data', (chunk) => { body += chunk; });
      response.on('end', () => response.statusCode === 200 ? resolve(body) : reject(new Error(`HTTP ${response.statusCode}`)));
    });
    request.on('timeout', () => request.destroy(new Error('Request timed out')));
    request.on('error', reject);
  });
  const checks = [];
  const check = async (name, fn) => {
    try { checks.push({ name, ok: true, detail: await fn() }); }
    catch (error) { checks.push({ name, ok: false, detail: error.message }); }
  };
  for (const [name, port] of [['Feed handlers',3001],['Binance feed',3012],['Execution',3003],['Insights',3004],['News sentiment',3006],['Learning',3010],['Position management',3002],['AutoTrader',3005]]) {
    await check(`${name} health`, async () => {
      const result = await get(port, '/health');
      if (result !== 'ok' && result?.status !== 'ok') throw new Error('Unexpected health response');
      return 'Healthy';
    });
  }
  await check('System containers', async () => {
    const lines = (await compose('sbe-system', 'system/compose.yml', ['ps', '--format', 'json'])).split('\n').filter(Boolean);
    const containers = lines.flatMap((line) => {
      try { return [JSON.parse(line)]; }
      catch { return []; }
    });
    const running = new Set(containers.filter((container) => container.State === 'running').map((container) => container.Service));
    const strategyServices = await getStrategyServices();
    const required = ['feed-handlers','binance-feed','position-management','execution','insights','insights-maintenance','auto-trader','news-sentiment','learning','web', ...strategyServices.filter((service) => service.startsWith('strategy-x-live-'))];
    if (process.env.SCOUT_WORKERS_ENABLED === 'true') {
      required.push(...strategyServices.filter((service) => service.startsWith('strategy-x-scout-')));
    }
    const missing = required.filter((service) => !running.has(service));
    if (missing.length) throw new Error(`Not running: ${missing.join(', ')}`);
    const unhealthy = containers
      .filter((container) => required.includes(container.Service) && container.Health && container.Health !== 'healthy')
      .map((container) => `${container.Service} (${container.Health})`);
    if (unhealthy.length) throw new Error(`Not healthy: ${unhealthy.join(', ')}`);
    return `All ${required.length} required service containers running`;
  });
  await check('React web app', async () => {
    const html = await getHtml(5173);
    if (!html.includes('<div id="root">') || !html.includes('vite')) throw new Error('Web app shell is malformed');
    return 'React app shell served successfully';
  });
  await check('Benchmarks web app', async () => {
    const html = await getHtml(5175, '/benchmarks/');
    if (!html.includes('<div id="root">')) throw new Error('Benchmarks app shell is malformed');
    return 'Benchmarks app shell served successfully';
  });
  await check('Insights feed event ingestion', async () => {
    const status = await get(3004, '/internal/signal-feed-status');
    if (status.connected !== true) throw new Error('Insights is disconnected from the FeedHandlers signal stream');
    if (!Number.isFinite(status.last_event_age_ms)) throw new Error('Insights has not received a valid signal feed event');
    if (status.last_event_age_ms > 30_000) throw new Error(`Insights has received no signal feed events for ${Math.round(status.last_event_age_ms / 1000)}s`);
    if (!Number.isFinite(status.last_price_age_ms)) throw new Error('Insights has not received a price event');
    if (status.last_price_age_ms > 30_000) throw new Error(`Insights has received no price events for ${Math.round(status.last_price_age_ms / 1000)}s`);
    return `Connected · price ${Math.round(status.last_price_age_ms / 1000)}s ago · ${status.price_events.toLocaleString()} price events received`;
  });
  await check('RSocket WebSocket listeners', async () => {
    const ports = [10001, 10004, 10005, 10006, 10008, 10010, 10011];
    await Promise.all(ports.map(checkTcp));
    await compose('sbe-system', 'system/compose.yml', ['exec', '-T', 'insights', 'bash', '-c', '</dev/tcp/127.0.0.1/10007']);
    await compose('sbe-system', 'system/compose.yml', ['exec', '-T', 'news-sentiment', 'python', '-c', "import socket; socket.create_connection(('127.0.0.1', 10012), timeout=2).close()"]);
    return String(ports.length + 2) + ' RSocket listeners accept connections';
  });
  await check('News sentiment service health', async () => {
    const result = await get(3006, '/health');
    if (result?.status !== 'ok') throw new Error('News sentiment service is not healthy');
    return `${result.model} loaded`;
  });
  await check('Insights maintenance run', async () => {
    const raw = await compose('sbe-system', 'system/compose.yml', ['exec', '-T', 'insights-maintenance', 'cat', '/data/maintenance_status.json']);
    const status = JSON.parse(raw);
    const heartbeatAgeMs = Date.now() - Number(status.heartbeat_at_ms);
    if (!Number.isFinite(heartbeatAgeMs) || heartbeatAgeMs < -5000 || heartbeatAgeMs > 90_000) throw new Error('Maintenance heartbeat is stale');
    if (status.last_run_success !== true) throw new Error(status.last_error || 'No successful maintenance run recorded');
    return `Last run succeeded · ${status.consecutive_failures} consecutive failures`;
  });
  await check('Monitoring stack and scrape targets', async () => {
    const [prometheus, loki, grafana, targets] = await Promise.all([
      get(9090, '/-/healthy'), get(3100, '/loki/api/v1/label/service_name/values'), get(3000, '/api/health'), get(9090, '/api/v1/query?query=up'),
    ]);
    if (typeof prometheus !== 'string' || !prometheus.toLowerCase().includes('healthy')) throw new Error('Prometheus is not healthy');
    if (loki.status !== 'success' || !Array.isArray(loki.data) || loki.data.length === 0) throw new Error('Loki has no indexed application log labels');
    if (grafana.database !== 'ok') throw new Error('Grafana database is not ready');
    const jobs = new Set((targets.data?.result || []).filter((item) => item.value?.[1] === '1').map((item) => item.metric.job));
    if (!jobs.has('application') || !jobs.has('host') || !jobs.has('prometheus')) throw new Error('Prometheus scrape targets are not all healthy');
    return `Prometheus, Grafana, and scrape targets operational · Loki indexed ${loki.data.length} log services`;
  });
  await check('Infrastructure containers', async () => {
    const lines = (await compose('sbe-infra', 'monitoring/docker-compose.yml', ['ps', '--format', 'json'])).split('\n').filter(Boolean);
    const running = new Set(lines.map((line) => {
      try { const container = JSON.parse(line); return container.State === 'running' ? container.Service : ''; }
      catch { return ''; }
    }));
    const required = ['service-control','benchmarks-ui','prometheus','loki','alloy','node-exporter','app-exporter','grafana'];
    const missing = required.filter((service) => !running.has(service));
    if (missing.length) throw new Error(`Not running: ${missing.join(', ')}`);
    return `All ${required.length} infrastructure containers running`;
  });
  const failed = checks.filter(({ ok }) => !ok).length;
  return { ok: failed === 0, checkedAt: new Date().toISOString(), checks, summary: failed ? `${failed} of ${checks.length} checks failed` : `${checks.length} checks passed` };
}
