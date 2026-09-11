import test from 'node:test';
import assert from 'node:assert/strict';
import {
  conflictMessage, createMutationRequest, openCount, relationshipIsEditable,
  validFunctionsForSeniority
} from '../src/ui/governance-ui.js';

test('agent form only offers valid function and seniority combinations', () => {
  assert.equal(validFunctionsForSeniority('INTERN').some(([id]) => id === 'PRODUCT_MANAGER'), false);
  assert.equal(validFunctionsForSeniority('SENIOR').some(([id]) => id === 'SOFTWARE_ARCHITECTURE'), true);
});

test('mutation requests carry optimistic revision and GOD project scope', () => {
  const request = createMutationRequest(7, {type:'PAUSE_AGENT', data:{agentId:'nova'}}, 'maintenance');
  assert.equal(request.expectedRevision, 7);
  assert.equal(request.actor.id, 'god');
  assert.equal(request.reason, 'maintenance');
});

test('only persistent organizational edge types are editable', () => {
  assert.equal(relationshipIsEditable({type:'ADVISORY', persistent:true}), true);
  assert.equal(relationshipIsEditable({type:'REPORTING', persistent:true}), false);
  assert.equal(relationshipIsEditable({type:'HANDOFF', persistent:false}), false);
});

test('decision badges and conflict copy reflect durable state', () => {
  assert.equal(openCount([{status:'OPEN'}, {status:'APPROVED'}, {status:'PENDING'}]), 2);
  assert.match(conflictMessage('organization revision conflict: expected 1, current 2'), /changed elsewhere/i);
});
