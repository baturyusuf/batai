import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import assert from 'node:assert/strict';

const root = path.resolve(import.meta.dirname, '..');
const fixture = JSON.parse(fs.readFileSync(path.join(root, 'specs/control-plane-contract-v1.json'), 'utf8'));

test('golden external control-plane contract preserves legacy Node routes and MCP names', () => {
  const server = fs.readFileSync(path.join(root, 'src/server.mjs'), 'utf8');
  const mcp = fs.readFileSync(path.join(root, 'src/control/mcp-server.mjs'), 'utf8');
  for (const route of fixture.legacyHttpRoutes) {
    const [, pathname] = route.split(' ');
    const stablePart = pathname.split('/:')[0];
    assert.ok(server.includes(stablePart), route);
  }
  for (const tool of fixture.legacyMcpTools) assert.ok(mcp.includes(`name:'${tool}'`), tool);
});

test('production convenience scripts launch the Rust control plane', () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
  assert.match(pkg.scripts.start, /batai-control -- serve/);
  assert.match(pkg.scripts.mcp, /batai-control -- mcp/);
  assert.match(pkg.scripts.daemon, /batai-control -- daemon/);
  assert.doesNotMatch(pkg.scripts.start, /node src\/server\.mjs/);
  assert.doesNotMatch(pkg.scripts.mcp, /node src\/control\/mcp-server\.mjs/);
});

test('desktop exposes daemon connection and recovery states', () => {
  const html = fs.readFileSync(path.join(root, 'src/ui/index.html'), 'utf8');
  const app = fs.readFileSync(path.join(root, 'src/ui/app.js'), 'utf8');
  assert.match(html, /data-state="STARTING"/);
  for (const state of ['STARTING', 'CONNECTED', 'RECOVERING', 'STOPPING', 'DISCONNECTED', 'ERROR']) {
    assert.ok(app.includes(state), state);
  }
  assert.match(app, /batai:\/\/daemon-status/);
});
