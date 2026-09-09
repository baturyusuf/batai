import { RESOURCE_STATUSES } from './constants.mjs';

export class ResourceManager {
  constructor({ store, events, agentRegistry, sessionManager, unknownQuotaRetryMs = 15 * 60 * 1000 }) {
    this.store = store;
    this.events = events;
    this.agentRegistry = agentRegistry;
    this.sessionManager = sessionManager;
    this.timers = new Map();
    this.onResumeRequested = null;
    this.unknownQuotaRetryMs = unknownQuotaRetryMs;
  }

  async refresh(agentId) {
    const agent = this.agentRegistry.get(agentId);
    if (!agent) throw new Error(`Unknown agent: ${agentId}`);
    const adapter = this.sessionManager.adapterFor(agent);
    const state = await adapter.getUsageState(agent);
    const normalized = {
      status: RESOURCE_STATUSES.includes(state.status) ? state.status : 'UNKNOWN',
      resetAt: state.resetAt ?? null,
      details: state.details ?? {}
    };
    this.store.setResourceState(agentId, normalized);
    this.events.publish('RESOURCE_STATUS_CHANGED', { source: agentId, payload: normalized });
    return normalized;
  }

  markUnavailable(agentId, { status = 'RATE_LIMITED', resetAt = null, details = {} } = {}) {
    const state = { status, resetAt, details };
    this.store.setResourceState(agentId, state);
    this.events.publish(status === 'RATE_LIMITED' ? 'AGENT_RATE_LIMITED' : 'RESOURCE_STATUS_CHANGED', { source: agentId, payload: state });
    if (status === 'RATE_LIMITED') {
      const retryAt = resetAt ?? new Date(Date.now() + this.unknownQuotaRetryMs).toISOString();
      this.scheduleResumeCheck(agentId, retryAt);
    }
    return state;
  }

  markRateLimited(agentId, options = {}) {
    return this.markUnavailable(agentId, { ...options, status:'RATE_LIMITED' });
  }

  scheduleResumeCheck(agentId, resetAt) {
    const existing = this.timers.get(agentId);
    if (existing) clearTimeout(existing);
    const delay = Math.max(0, new Date(resetAt).getTime() - Date.now());
    const timer = setTimeout(async () => {
      this.timers.delete(agentId);
      const state = await this.refresh(agentId).catch(() => ({ status: 'UNKNOWN' }));
      if (state.status === 'AVAILABLE' && this.onResumeRequested) await this.onResumeRequested(agentId);
    }, Math.min(delay, 2_147_000_000));
    timer.unref?.();
    this.timers.set(agentId, timer);
  }

  dispose() {
    for (const timer of this.timers.values()) clearTimeout(timer);
    this.timers.clear();
  }
}
