import http from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const exec = promisify(execFile);
const root = process.env.WORKSPACE_ROOT || '/workspace';
const homepageDist = resolve(root, 'src/typescript/workspace/apps/homepage/dist');
const services = ['feed-handlers','position-management','execution','insights','insights-maintenance','live-signals','auto-trader','news-sentiment','learning','scouts','web'];
const send = (res, code, value, type='application/json') => { res.writeHead(code, {'content-type': `${type}; charset=utf-8`}); res.end(type.startsWith('application/json') ? JSON.stringify(value) : value); };
const contentType = (file) => ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon' }[file.slice(file.lastIndexOf('.'))] || 'application/octet-stream');
const compose = async (project, file, args) => {
  const { stdout, stderr } = await exec('docker', ['compose','-p',project,'-f',file,'--progress=quiet',...args], {
    cwd:root,
    timeout: 15 * 60 * 1000,
    maxBuffer: 5 * 1024 * 1024,
    env: {...process.env, REPO_ROOT: process.env.DOCKER_WORKSPACE_ROOT || process.env.WORKSPACE_ROOT || root}
  });
  return stdout || stderr || 'Done';
};
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
        const flags = action === 'start' ? ['-d'] : [];
        const first = await compose('sbe-system','system/compose.yml',[verb,...flags]);
        const second = await compose('sbe-infra','monitoring/docker-compose.yml',[verb,...flags]);
        return send(res,200,{output:`${first}\n${second}`});
      }
      if (!['system','infra'].includes(project) || !['start','stop','restart','rebuild-restart'].includes(action)) return send(res,400,{error:'Invalid action'});
      if (project === 'infra' && service && !['prometheus','loki','alloy','node-exporter','app-exporter','grafana'].includes(service)) return send(res,400,{error:'Unknown infra service'});
      if (project === 'system' && service && !services.includes(service)) return send(res,400,{error:'Unknown system service'});
      const file = project === 'system' ? 'system/compose.yml' : 'monitoring/docker-compose.yml';
      let args;
      if (action === 'rebuild-restart') {
        if (!service) return send(res,400,{error:'Rebuild and restart requires a single service'});
        args = ['up','-d','--build','--no-deps',service];
      }
      else if (action === 'start') args = ['up','-d',...(service ? [service] : [])];
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
  const post = async (port, path, body, timeoutMs = 5000) => {
    const response = await fetch(`http://${host}:${port}${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.json();
  };
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
  for (const [name, port] of [['Feed handlers',3001],['Execution',3003],['Insights',3004],['Live signals',3007],['News sentiment',3006],['Learning',3010],['Scouts',3011],['Position management',3002],['AutoTrader',3005]]) {
    await check(`${name} health`, async () => {
      const result = await get(port, '/health');
      if (result !== 'ok' && result?.status !== 'ok') throw new Error('Unexpected health response');
      return 'Healthy';
    });
  }
  await check('System containers', async () => {
    const lines = (await compose('sbe-system', 'system/compose.yml', ['ps', '--format', 'json'])).split('\n').filter(Boolean);
    const running = new Set(lines.map((line) => {
      try { const container = JSON.parse(line); return container.State === 'running' ? container.Service : ''; }
      catch { return ''; }
    }));
    const missing = ['feed-handlers','position-management','execution','insights','insights-maintenance','live-signals','auto-trader','news-sentiment','learning','scouts','web'].filter((service) => !running.has(service));
    if (missing.length) throw new Error(`Not running: ${missing.join(', ')}`);
    return 'All 11 service containers running';
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
  await check('Live signal computation', async () => {
    const snapshots = await get(3007, '/signals/latest');
    if (!Array.isArray(snapshots) || snapshots.length === 0) throw new Error('No live signal snapshots produced');
    const latest = snapshots.reduce((newest, item) => Math.max(newest, Number(item.timestamp) || 0), 0);
    const ageMs = Date.now() - latest;
    if (latest === 0 || ageMs < -5000 || ageMs > 60_000) throw new Error(`Latest signal is stale (${Math.round(ageMs / 1000)}s)`);
    const valid = snapshots.filter((item) => typeof item.symbol === 'string' && Number.isFinite(item.score) && item.components && typeof item.components === 'object');
    if (valid.length === 0) throw new Error('Live signal payloads have no valid symbol, score, or components');
    return `${valid.length} valid live snapshots · newest ${Math.max(0, Math.round(ageMs / 1000))}s old`;
  });
  await check('Insights signal pipeline', async () => {
    const dashboard = await get(3004, '/signals/dashboard');
    if (!Array.isArray(dashboard.best) || dashboard.best.length === 0) throw new Error('Insights dashboard has no computed signals');
    const valid = dashboard.best.filter((item) => typeof item.symbol === 'string' && Number.isFinite(item.timestamp) && Number.isFinite(item.score));
    if (valid.length === 0) throw new Error('Insights dashboard signals are malformed');
    const newest = valid.reduce((timestamp, item) => Math.max(timestamp, item.timestamp), 0);
    const ageMs = Date.now() - newest;
    if (ageMs < -5000 || ageMs > 30_000) throw new Error(`Newest Insights signal is stale (${Math.round(ageMs / 1000)}s)`);
    return `${valid.length} computed records · newest ${Math.max(0, Math.round(ageMs / 1000))}s old`;
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
  await check('Position management API', async () => {
    const result = await get(3002, '/positions');
    if (!Array.isArray(result.positions)) throw new Error('Positions response is malformed');
    return `Readable · ${result.positions.length} positions`;
  });
  await check('Position management account and scheduler APIs', async () => {
    const [account, vwap] = await Promise.all([get(3002, '/positions/account'), get(3002, '/positions/vwap')]);
    if (!Number.isFinite(account.equity) || !Number.isFinite(account.cash)) throw new Error('Account balances are malformed');
    if (!Array.isArray(vwap.runs)) throw new Error('VWAP scheduler status is malformed');
    return `Account balances valid · ${vwap.runs.length} VWAP runs tracked`;
  });
  await check('Execution matching and gateway', async () => {
    const result = await post(3003, '/internal/orders/match', {
      side: 'buy', quantity: 3, limit_price: 101,
      book: { bids: [], asks: [{ price: 100, size: 1 }, { price: 101, size: 2 }, { price: 102, size: 5 }] },
    });
    if (result.filled_quantity !== 3 || result.remaining_quantity !== 0 || result.fills?.length !== 2 || Math.abs(result.average_execution_price - (302 / 3)) > 1e-9) throw new Error('Order matcher returned an incorrect limit fill');
    const gateway = await get(3003, '/positions/vwap');
    if (!Array.isArray(gateway.runs)) throw new Error('Execution gateway did not proxy VWAP status correctly');
    return 'Limit matching correct · Position Management proxy responds';
  });
  await check('AutoTrader enabled and operational', async () => {
    const result = await get(3005, '/positions/auto-trader');
    if (result.enabled !== true) throw new Error('AutoTrader is disabled');
    if (result.audit_healthy !== true || result.audit_failure_latched) throw new Error('Decision audit is unhealthy');
    const decisionAgeMs = Date.now() - Number(result.last_decision_at);
    if (!Number.isFinite(decisionAgeMs) || decisionAgeMs < -5000 || decisionAgeMs > 300_000 || !Number.isFinite(result.decisions_since_trade) || result.decisions_since_trade <= 0) throw new Error('AutoTrader decision loop has not processed a recent cycle');
    return `Enabled · ${result.strategies.length} strategies · ${result.decisions_since_trade.toLocaleString()} decision cycles · audit healthy`;
  });
  await check('AutoTrader-to-Scouts strategy evaluation', async () => {
    const parameters = await get(3005, '/positions/auto-trader/parameters');
    const profile = parameters.strategy_profiles?.find((item) => item.strategy === 'live-momentum');
    if (!profile || !Array.isArray(profile.parameters)) throw new Error('No learned live-momentum profile available');
    const profileParameters = Object.fromEntries(profile.parameters.map((item) => [item.key, item.value]));
    const momentumWindow = profileParameters.momentum_lookback_seconds;
    const trendWindow = profileParameters.trend_lookback_seconds;
    if (!Number.isFinite(momentumWindow) || !Number.isFinite(trendWindow) || !Number.isFinite(profileParameters.minimum_momentum)) throw new Error('Live-momentum profile is missing required bounded parameters');
    const testSignal = {
      symbol: profile.symbol,
      direction: 'neutral',
      timestamp: Date.now(),
      score: 0.9,
      strength: 0.9,
      components: {
        momentum_by_window: [{ window_seconds: momentumWindow, value: Math.max(0.5, profileParameters.minimum_momentum + 0.1) }],
        trend_volatility_units_by_window: [{ window_seconds: trendWindow, value: 0.5 }],
      },
    };
    const candidates = await post(3011, '/live-candidates', {
      signals: [testSignal],
      profiles: [{ symbol: profile.symbol, strategy: profile.strategy, parameters: profileParameters }],
    });
    if (!Array.isArray(candidates) || candidates.length !== 1) throw new Error('Live-momentum evaluator did not produce the expected scout candidate');
    const candidate = candidates[0];
    if (candidate.strategy !== profile.strategy || candidate.direction !== 'bullish' || candidate.strategy_decision?.scout_only !== false) throw new Error('Scout candidate lost strategy attribution or direction');
    return `Bounded ${profile.strategy} profile evaluated · candidate attribution and direction valid`;
  });
  await check('Scout pipeline configured', async () => {
    const [status, configuration] = await Promise.all([get(3011, '/status'), get(3011, '/configuration')]);
    if (!Array.isArray(configuration.strategies) || configuration.strategies.length === 0) throw new Error('No scout strategies configured');
    if (!Number.isFinite(status.active_samples) || !Number.isFinite(status.learning_samples)) throw new Error('Scout status is malformed');
    return `${configuration.strategies.length} strategies · ${status.active_samples} active samples · ${status.learning_samples} outcomes`;
  });
  await check('Learning decision profiles and bounds', async () => {
    const [snapshot, autoTrader] = await Promise.all([
      get(3010, '/decision-snapshot'),
      get(3005, '/positions/auto-trader/parameters'),
    ]);
    if (!Array.isArray(snapshot.profiles) || !snapshot.strategy_size_multipliers || typeof snapshot.strategy_size_multipliers !== 'object') throw new Error('Learning decision snapshot is malformed');
    const multipliers = Object.values(snapshot.strategy_size_multipliers);
    if (multipliers.some((value) => !Number.isFinite(value) || value < 0.7 || value > 2.5)) throw new Error('Learning produced an invalid strategy size multiplier');
    const activeProfile = autoTrader.strategy_profiles?.find((item) => item.strategy === 'live-momentum');
    const learnedProfile = snapshot.profiles.find((item) => item.strategy === activeProfile?.strategy && item.symbol === activeProfile?.symbol);
    if (!activeProfile || !learnedProfile || !learnedProfile.parameters || !Array.isArray(activeProfile.parameters)) throw new Error('Learning profile is not available to AutoTrader');
    let compared = 0;
    for (const parameter of activeProfile.parameters) {
      const learnedValue = learnedProfile.parameters[parameter.key];
      if (!Number.isFinite(parameter.value) || !Number.isFinite(parameter.min) || !Number.isFinite(parameter.max) || parameter.value < parameter.min - 1e-9 || parameter.value > parameter.max + 1e-9) throw new Error(`AutoTrader parameter ${parameter.key} is outside its learned bounds`);
      if (Number.isFinite(learnedValue)) {
        if (Math.abs(learnedValue - parameter.value) > 1e-9) throw new Error(`AutoTrader did not receive Learning's ${parameter.key} value`);
        compared += 1;
      }
    }
    if (compared === 0) throw new Error('No learned parameters could be compared with AutoTrader');
    return `${snapshot.profiles.length} learned profiles · ${compared} bounded values match AutoTrader`;
  });
  await check('News sentiment inference', async () => {
    const results = await post(3006, '/classify', [{ title: 'Bitcoin adoption rises as the network reaches a new milestone', article_text: 'The update reports continued growth in usage and investment.' }], 15_000);
    const result = results?.[0];
    if (!result || !['bullish', 'bearish', 'neutral'].includes(result.direction) || !Number.isFinite(result.score) || !Number.isFinite(result.confidence) || result.confidence < 0 || result.confidence > 1) throw new Error('Sentiment model returned an invalid classification');
    return `${result.direction} classification · confidence ${(result.confidence * 100).toFixed(1)}%`;
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
