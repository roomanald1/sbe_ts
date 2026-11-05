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
      if (!['system','infra'].includes(project) || !['start','stop','restart'].includes(action)) return send(res,400,{error:'Invalid action'});
      if (project === 'infra' && service && !['prometheus','loki','alloy','node-exporter','app-exporter','grafana'].includes(service)) return send(res,400,{error:'Unknown infra service'});
      if (project === 'system' && service && !services.includes(service)) return send(res,400,{error:'Unknown system service'});
      const file = project === 'system' ? 'system/compose.yml' : 'monitoring/docker-compose.yml';
      let args;
      if (action === 'start') args = ['up','-d',...(service ? [service] : [])];
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
    if (req.method === 'GET' && url.pathname.startsWith('/benchmarks/')) {
      const relativePath = decodeURIComponent(url.pathname.slice('/benchmarks'.length));
      const file = resolve('/app/benchmarks', `.${relativePath}${relativePath.endsWith('/') ? 'index.html' : ''}`);
      if (!file.startsWith(`/app/benchmarks${sep}`)) return send(res,403,{error:'Forbidden'});
      const content = await readFile(file);
      const type = file.endsWith('.html') ? 'text/html' : file.endsWith('.css') ? 'text/css' : file.endsWith('.js') ? 'text/javascript' : 'application/octet-stream';
      res.writeHead(200, {'content-type':type}); return res.end(content);
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
