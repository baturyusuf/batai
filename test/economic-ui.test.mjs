import test from 'node:test';
import assert from 'node:assert/strict';
import {formatBytes,portfolioGroups,routingSummary} from '../src/ui/economic-ui.js';

test('economic UI preserves unknown values instead of inventing numbers',()=>{
  assert.equal(formatBytes(null),'—');
  assert.equal(routingSummary(null),'No suitable resource');
});

test('portfolio groups by billing contract rather than provider name',()=>{
  const groups=portfolioGroups([{id:'a',provider:'same',billingMode:'SUBSCRIPTION_QUOTA'},{id:'b',provider:'same',billingMode:'PAYG'},{id:'c',provider:'ollama',billingMode:'LOCAL'}]);
  assert.deepEqual(groups.subscription.map(item=>item.id),['a']);
  assert.deepEqual(groups.payg.map(item=>item.id),['b']);
  assert.deepEqual(groups.local.map(item=>item.id),['c']);
});

test('routing explanation names selected resource and verified model',()=>{
  assert.equal(routingSummary({outcome:'SELECTED',selectedResourceId:'kimi-personal-membership',selectedModel:'k3'}),'kimi-personal-membership · k3');
});
