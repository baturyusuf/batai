export class SessionManager {
  constructor({ adapters, store }) {
    this.adapters = adapters;
    this.store = store;
    this.sessionsByAgent = new Map();
    for (const session of store?.listSessions?.() ?? []) this.sessionsByAgent.set(session.agentId, session);
  }

  adapterFor(agent) {
    const adapter = this.adapters.get(agent.provider);
    if (!adapter) throw new Error(`Provider adapter not registered: ${agent.provider}`);
    return adapter;
  }

  async ensureSession(agent) {
    let existing = this.sessionsByAgent.get(agent.id);
    if (existing) return existing;
    existing = this.store?.getSession?.(agent.id);
    if (existing) {
      await this.adapterFor(agent).resumeSession(existing.sessionId, agent);
      this.sessionsByAgent.set(agent.id, existing);
      return existing;
    }
    const adapter = this.adapterFor(agent);
    const { sessionId, metadata = {} } = await adapter.createSession(agent);
    const session = { sessionId, provider: agent.provider, agentId: agent.id, metadata };
    this.sessionsByAgent.set(agent.id, session);
    this.store?.upsertSession?.(agent.id, agent.provider, sessionId, metadata);
    return session;
  }

  async resume(agent) {
    const session = this.sessionsByAgent.get(agent.id) ?? this.store?.getSession?.(agent.id);
    if (!session) return this.ensureSession(agent);
    await this.adapterFor(agent).resumeSession(session.sessionId, agent);
    this.sessionsByAgent.set(agent.id, session);
    return session;
  }

  get(agentId) { return this.sessionsByAgent.get(agentId) ?? this.store?.getSession?.(agentId) ?? null; }
}
