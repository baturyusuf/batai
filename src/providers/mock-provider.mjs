import { ProviderAdapter } from './provider-adapter.mjs';
import { ResourceUnavailableError } from '../core/errors.mjs';
import { newId } from '../core/ids.mjs';

export class MockProviderAdapter extends ProviderAdapter {
  constructor({ latencyMs = 15 } = {}) {
    super('mock');
    this.latencyMs = latencyMs;
    this.sessions = new Map();
    this.calls = [];
    this.usage = new Map();
  }

  async createSession(agent) {
    const id = newId(`SESSION-${agent.id}`);
    this.sessions.set(id, { agentId: agent.id, turns: [] });
    return { sessionId: id };
  }

  async resumeSession(sessionId) {
    if (!this.sessions.has(sessionId)) this.sessions.set(sessionId, { agentId: 'unknown', turns: [] });
    return { sessionId };
  }

  setUsage(agentId, state) { this.usage.set(agentId, state); }

  async getUsageState(agent) {
    return this.usage.get(agent.id) ?? { status: 'AVAILABLE', resetAt: null };
  }

  async sendTask({ sessionId, agent, task }) {
    const usage = await this.getUsageState(agent);
    if (usage.status === 'RATE_LIMITED' || usage.status === 'OFFLINE' || usage.status === 'AUTH_REQUIRED') {
      throw new ResourceUnavailableError(`Mock provider unavailable for ${agent.id}`, usage);
    }
    await new Promise(resolve => setTimeout(resolve, this.latencyMs));
    const result = {
      agent_id: agent.id,
      task_id: task.id,
      session_id: sessionId,
      summary: `Mock agent ${agent.id} completed: ${task.objective}`,
      artifacts: [],
      completed_at: new Date().toISOString()
    };
    this.calls.push({ agentId: agent.id, taskId: task.id, sessionId });
    const session = this.sessions.get(sessionId);
    session?.turns.push({ task: task.id, result });
    return result;
  }
}
