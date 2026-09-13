import test from 'node:test';
import assert from 'node:assert/strict';
import {deliveryTimeline,nextDeliveryAction} from '../src/ui/delivery-ui.js';

test('delivery timeline distinguishes completed, active and pending stages',()=>{
  const timeline=deliveryTimeline({state:'CI_PENDING'});
  assert.equal(timeline.find(item=>item.name==='push').state,'complete');
  assert.equal(timeline.find(item=>item.name==='CI').state,'active');
  assert.equal(timeline.find(item=>item.name==='merge').state,'pending');
});

test('delivery actions never prepare an unassigned task and require explicit merge approval',()=>{
  assert.equal(nextDeliveryAction(null,null),null);
  assert.equal(nextDeliveryAction(null,'nova'),'prepare');
  assert.equal(nextDeliveryAction({state:'MERGE_READY',mergeDecisionId:null},'nova'),'approve-merge');
  assert.equal(nextDeliveryAction({state:'MERGE_READY',mergeDecisionId:'GOD'},'nova'),'merge');
});
