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

export const CAPABILITY_DIMENSIONS = ['coding','debugging','planning','architecture','tool_use','instruction_following','long_context','test_generation','review','research','speed','reliability'];

export function confidenceLabel(dimension) {
  if (!dimension || (dimension.estimatedScore == null && dimension.score == null)) return 'Unknown';
  return String(dimension.confidenceBand ?? 'VERY_LOW').toLowerCase().replaceAll('_',' ').replace(/\b\w/g, value => value.toUpperCase());
}

export function capabilityEvidenceRows(profile) {
  if (!profile?.dimensions) return [];
  return Object.entries(profile.dimensions).sort(([left],[right]) => left.localeCompare(right)).map(([dimension,value]) => ({
    dimension,
    score:value.estimatedScore,
    routingEstimate:value.routingEstimate,
    confidence:value.confidence,
    confidenceBand:value.confidenceBand,
    samples:value.sampleCount ?? 0,
    disagreement:Boolean(value.disagreement),
    sources:Object.entries(value.evidenceComposition ?? {}).map(([source,count]) => ({source,count}))
  }));
}

export function calibrationPercent(value) {
  return value == null ? '—' : `${Math.round(Number(value) * 1000) / 10}%`;
}

export function replaySummary(result) {
  if (!result) return 'Replay unavailable';
  const original=result.originalResourceId ?? 'No selection';
  const next=result.replayResourceId ?? 'No selection';
  return `${original} → ${next}`;
}
