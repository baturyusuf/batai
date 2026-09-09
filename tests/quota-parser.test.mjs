import test from 'node:test';
import assert from 'node:assert/strict';
import { parseResetAtFromText } from '../src/providers/quota-parser.mjs';

test('quota parser handles relative reset windows', () => {
  const now = Date.parse('2026-09-09T12:00:00Z');
  assert.equal(parseResetAtFromText('Usage limit reached. Try again in 2h 15m.', now), '2026-09-09T14:15:00.000Z');
});

test('quota parser handles ISO timestamps', () => {
  assert.equal(parseResetAtFromText('limit reached; reset at 2026-09-09T18:30:00Z'), '2026-09-09T18:30:00.000Z');
});
