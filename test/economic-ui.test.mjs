import test from 'node:test';
import assert from 'node:assert/strict';
import {capabilityEvidenceRows,confidenceLabel,formatBytes,portfolioGroups,replaySummary,routingSummary} from '../src/ui/economic-ui.js';

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

test('capability evidence keeps confidence, samples and disagreement visible',()=>{
  const rows=capabilityEvidenceRows({dimensions:{coding:{estimatedScore:78,routingEstimate:72,confidence:.7,confidenceBand:'HIGH',sampleCount:14,disagreement:true,evidenceComposition:{BATAI_BENCHMARK:1,REAL_TASK_HISTORY:14}}}});
  assert.equal(rows[0].samples,14);
  assert.equal(rows[0].disagreement,true);
  assert.equal(confidenceLabel(rows[0]),'High');
  assert.deepEqual(rows[0].sources.map(item=>item.source),['BATAI_BENCHMARK','REAL_TASK_HISTORY']);
});

test('offline replay labels unknown selection without inventing an outcome',()=>{
  assert.equal(replaySummary({originalResourceId:'codex-native',replayResourceId:'kimi-plan'}),'codex-native → kimi-plan');
  assert.equal(replaySummary({originalResourceId:null,replayResourceId:null}),'No selection → No selection');
});
