export function formatBytes(value) {
  return value == null ? '—' : `${(Number(value) / 1024 ** 3).toFixed(1)} GB`;
}

export function portfolioGroups(resources = []) {
  const groups = {local:[],subscription:[],payg:[],other:[]};
  for (const resource of resources) {
    if (resource.billingMode === 'LOCAL') groups.local.push(resource);
    else if (['SUBSCRIPTION_QUOTA','NATIVE_SUBSCRIPTION_CLIENT'].includes(resource.billingMode)) groups.subscription.push(resource);
    else if (resource.billingMode === 'PAYG') groups.payg.push(resource);
    else groups.other.push(resource);
  }
  return groups;
}

export function routingSummary(decision) {
  if (!decision || decision.outcome !== 'SELECTED') return 'No suitable resource';
  return `${decision.selectedResourceId} · ${decision.selectedModel ?? 'provider managed'}`;
}
