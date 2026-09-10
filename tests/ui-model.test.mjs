import test from 'node:test';
import assert from 'node:assert/strict';
import { authModeForProvider } from '../src/ui/provider-auth.js';

test('local providers use local authentication metadata', () => {
  assert.equal(authModeForProvider('mock'), 'local');
  assert.equal(authModeForProvider('ollama'), 'local');
});

test('CLI providers use subscription authentication metadata', () => {
  assert.equal(authModeForProvider('codex-cli'), 'subscription');
  assert.equal(authModeForProvider('claude-cli'), 'subscription');
});
