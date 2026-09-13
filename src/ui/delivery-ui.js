export const DELIVERY_STAGES = [
  ['implementation',['WORKTREE','CHANGES_READY','COMMITTED','PUSHED','PULL_REQUEST_OPEN','REVIEW','CI_PENDING','MERGE_READY','MERGED','CLOSED']],
  ['commit',['COMMITTED','PUSHED','PULL_REQUEST_OPEN','REVIEW','CI_PENDING','MERGE_READY','MERGED','CLOSED']],
  ['push',['PUSHED','PULL_REQUEST_OPEN','REVIEW','CI_PENDING','MERGE_READY','MERGED','CLOSED']],
  ['pull request',['PULL_REQUEST_OPEN','REVIEW','CI_PENDING','MERGE_READY','MERGED','CLOSED']],
  ['review',['MERGE_READY','MERGED','CLOSED']],
  ['CI',['MERGE_READY','MERGED','CLOSED']],
  ['merge',['MERGED','CLOSED']]
];

export function deliveryTimeline(delivery) {
  if (!delivery) return DELIVERY_STAGES.map(([name])=>({name,state:'pending'}));
  return DELIVERY_STAGES.map(([name,complete])=>({
    name,
    state: complete.includes(delivery.state) ? 'complete'
      : delivery.state === 'BLOCKED' || delivery.state === 'FAILED' ? 'blocked'
      : currentStage(name,delivery.state) ? 'active' : 'pending'
  }));
}

function currentStage(name,state) {
  return ({WORKTREE:'implementation',CHANGES_READY:'implementation',COMMITTED:'commit',PUSHED:'push',PULL_REQUEST_OPEN:'pull request',REVIEW:'review',CI_PENDING:'CI',MERGE_READY:'merge'}[state] === name);
}

export function nextDeliveryAction(delivery, owner) {
  if (!delivery) return owner ? 'prepare' : null;
  return ({WORKTREE:'commit',CHANGES_READY:'commit',COMMITTED:'push',PUSHED:'create-pr',
    PULL_REQUEST_OPEN:'sync',REVIEW:'sync',CI_PENDING:'sync',MERGE_READY:delivery.mergeDecisionId?'merge':'approve-merge',
    MERGED:'cleanup'}[delivery.state] ?? null);
}
