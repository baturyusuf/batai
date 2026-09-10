import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

const repositoryRoot = path.resolve(import.meta.dirname, '..');

async function getFreePort() {
  const server = net.createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  server.close();
  await once(server, 'close');
  return port;
}

async function startServer(projectRoot, port) {
  const child = spawn(process.execPath, ['src/server.mjs'], {
    cwd: repositoryRoot,
    env: { ...process.env, BATAI_PROJECT_ROOT: projectRoot, BATAI_PORT: String(port) },
    stdio: ['ignore', 'pipe', 'pipe']
  });

  let output = '';
  child.stdout.setEncoding('utf8');
  child.stderr.setEncoding('utf8');
  child.stdout.on('data', chunk => { output += chunk; });
  child.stderr.on('data', chunk => { output += chunk; });

  const deadline = Date.now() + 5000;
  while (!output.includes('Batai running at')) {
    if (child.exitCode !== null) throw new Error(`Server exited during startup:\n${output}`);
    if (Date.now() >= deadline) throw new Error(`Server startup timed out:\n${output}`);
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  return child;
}

async function stopServer(child) {
  if (child.exitCode !== null) return;
  child.kill('SIGTERM');
  await once(child, 'exit');
}

test('HTTP API reports malformed JSON as a client error', async () => {
  const projectRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'batai-server-'));
  const port = await getFreePort();
  const child = await startServer(projectRoot, port);

  try {
    const response = await fetch(`http://127.0.0.1:${port}/api/agents`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{broken'
    });
    const body = await response.json();

    assert.equal(response.status, 400);
    assert.equal(body.error, 'InvalidJson');
    assert.equal(body.message, 'Request body must contain valid JSON');
  } finally {
    await stopServer(child);
    fs.rmSync(projectRoot, { recursive: true, force: true });
  }
});

test('HTTP API reports invalid agent data as a validation error', async () => {
  const projectRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'batai-server-'));
  const port = await getFreePort();
  const child = await startServer(projectRoot, port);

  try {
    const response = await fetch(`http://127.0.0.1:${port}/api/agents`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{}'
    });
    const body = await response.json();

    assert.equal(response.status, 400);
    assert.equal(body.error, 'ValidationError');
    assert.ok(body.details.includes('id is required'));
  } finally {
    await stopServer(child);
    fs.rmSync(projectRoot, { recursive: true, force: true });
  }
});
