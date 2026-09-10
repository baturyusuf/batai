import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { BataiRuntime } from './core/batai-runtime.mjs';
import { MockProviderAdapter } from './providers/mock-provider.mjs';
import { CodexCliAdapter } from './providers/codex-cli-adapter.mjs';
import { ClaudeCliAdapter } from './providers/claude-cli-adapter.mjs';
import { OllamaAdapter } from './providers/ollama-adapter.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = process.env.BATAI_PROJECT_ROOT ? path.resolve(process.env.BATAI_PROJECT_ROOT) : path.resolve(__dirname, '..');
const port = Number(process.env.BATAI_PORT ?? 4317);
const adapters = new Map([
  ['mock', new MockProviderAdapter()],
  ['codex-cli', new CodexCliAdapter({ projectRoot })],
  ['claude-cli', new ClaudeCliAdapter({ projectRoot })],
  ['ollama', new OllamaAdapter()]
]);
const runtime = new BataiRuntime({ projectRoot, adapters });
await runtime.start();

const uiRoot = path.join(__dirname, 'ui');

function json(res, status, value) {
  const body = JSON.stringify(value, null, 2);
  res.writeHead(status, { 'content-type': 'application/json; charset=utf-8', 'content-length': Buffer.byteLength(body) });
  res.end(body);
}

async function readJson(req) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  if (!chunks.length) return {};
  try {
    return JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch {
    const error = new Error('Request body must contain valid JSON');
    error.name = 'InvalidJson';
    throw error;
  }
}

const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, `http://${req.headers.host ?? 'localhost'}`);
    if (url.pathname === '/api/state' && req.method === 'GET') return json(res, 200, runtime.state());

    if (url.pathname === '/api/providers' && req.method === 'GET') {
      const providers = [];
      for (const [name, adapter] of runtime.adapters.entries()) {
        let usage = { status:'UNKNOWN' }; let models = [];
        try { usage = await adapter.getUsageState({ id:'probe', provider:name }); } catch (error) { usage = { status:'OFFLINE', details:{ message:error.message } }; }
        try { models = await adapter.listModels(); } catch {}
        providers.push({ name, usage, models });
      }
      return json(res, 200, providers);
    }

    if (url.pathname === '/api/agents' && req.method === 'POST') {
      const input = await readJson(req);
      return json(res, 201, runtime.directorTools.createAgent(input));
    }

    if (url.pathname === '/api/tasks' && req.method === 'POST') {
      const input = await readJson(req);
      const created = runtime.directorTools.assignTask(input);
      return json(res, 201, created);
    }


    if (url.pathname === '/api/god/messages' && req.method === 'POST') {
      const input = await readJson(req);
      return json(res, 201, runtime.authority.submitGodMessage(input));
    }

    if (url.pathname === '/api/decisions' && req.method === 'POST') {
      const input = await readJson(req);
      return json(res, 201, runtime.authority.createDecision(input));
    }

    const decisionMatch = url.pathname.match(/^\/api\/decisions\/([^/]+)\/resolve$/);
    if (decisionMatch && req.method === 'POST') {
      const input = await readJson(req);
      return json(res, 200, runtime.authority.resolveDecision(decodeURIComponent(decisionMatch[1]), input.value, { decidedBy:'GOD' }));
    }

    const approveMatch = url.pathname.match(/^\/api\/tasks\/([^/]+)\/approve$/);
    if (approveMatch && req.method === 'POST') {
      return json(res, 200, await runtime.directorTools.approveTask(decodeURIComponent(approveMatch[1])));
    }

    if (url.pathname === '/api/health') return json(res, 200, { ok: true, projectRoot });

    const relative = url.pathname === '/' ? 'index.html' : url.pathname.replace(/^\//, '');
    const safe = path.normalize(relative).replace(/^\.\.(\/|\\|$)/, '');
    const file = path.join(uiRoot, safe);
    if (!file.startsWith(uiRoot) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404); return res.end('Not found');
    }
    const ext = path.extname(file);
    const type = ext === '.html' ? 'text/html' : ext === '.css' ? 'text/css' : ext === '.js' ? 'text/javascript' : 'text/plain';
    res.writeHead(200, { 'content-type': `${type}; charset=utf-8` });
    fs.createReadStream(file).pipe(res);
  } catch (error) {
    const clientError = error.name === 'ValidationError' || error.name === 'InvalidJson';
    json(res, clientError ? 400 : 500, { error: error.name, message: error.message, details: error.details });
  }
});

server.listen(port, '127.0.0.1', () => {
  console.log(`Batai running at http://127.0.0.1:${port}`);
  console.log(`Project: ${projectRoot}`);
});

async function shutdown() {
  server.close();
  await runtime.close();
  process.exit(0);
}
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
