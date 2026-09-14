import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const app = await readFile(new URL('../src/ui/app.js', import.meta.url), 'utf8');
const html = await readFile(new URL('../src/ui/index.html', import.meta.url), 'utf8');

test('meeting UI exposes bounded context provenance and exact execution gates', () => {
  assert.match(app, /meeting\.contextPackages/);
  assert.match(app, /source\.reasonIncluded/);
  assert.match(app, /meeting\.executionGates/);
  assert.match(app, /Estimated context\/input usage is separate from actual provider usage/);
  assert.match(html, /name="explicitFiles"/);
  assert.match(html, /name="contextTokens"[^>]+max="6000"/);
  assert.match(html, /name="contextFiles"[^>]+max="12"/);
});
