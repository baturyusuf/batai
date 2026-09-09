import { ProviderAdapter } from './provider-adapter.mjs';
import { newId } from '../core/ids.mjs';
import { ResourceUnavailableError } from '../core/errors.mjs';

export class OllamaAdapter extends ProviderAdapter {
  constructor({ baseUrl = 'http://127.0.0.1:11434', timeoutMs = 20 * 60 * 1000 } = {}) {
    super('ollama');
    this.baseUrl = baseUrl.replace(/\/$/, '');
    this.timeoutMs = timeoutMs;
    this.sessions = new Map();
  }

  async #request(path, options = {}) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs); timer.unref?.();
    try {
      const response = await fetch(`${this.baseUrl}${path}`, { ...options, signal:controller.signal, headers:{ 'content-type':'application/json', ...(options.headers ?? {}) } });
      if (!response.ok) throw new Error(`Ollama HTTP ${response.status}: ${await response.text()}`);
      return await response.json();
    } finally { clearTimeout(timer); }
  }

  async authenticate() { return (await this.getUsageState()).status === 'AVAILABLE' ? { ok:true } : { ok:false }; }

  async listModels() {
    try {
      const value = await this.#request('/api/tags');
      return (value.models ?? []).map(m => m.name);
    } catch { return []; }
  }

  async createSession(agent) {
    const sessionId = newId(`OLLAMA-${agent.id}`);
    this.sessions.set(sessionId, []);
    return { sessionId };
  }

  async resumeSession(sessionId) {
    if (!this.sessions.has(sessionId)) this.sessions.set(sessionId, []);
    return { sessionId };
  }

  async getUsageState() {
    try { await this.#request('/api/tags'); return { status:'AVAILABLE' }; }
    catch (error) { return { status:'OFFLINE', details:{ message:error.message } }; }
  }

  async sendTask({ sessionId, agent, task }) {
    const state = await this.getUsageState();
    if (state.status !== 'AVAILABLE') throw new ResourceUnavailableError('Ollama is unavailable', { status:'OFFLINE' });
    const messages = this.sessions.get(sessionId) ?? [];
    const content = `BATAI TASK ${task.id}\n\nObjective: ${task.objective}\n\nAcceptance criteria:\n${(task.acceptance_criteria ?? []).map(x=>`- ${x}`).join('\n')}\n\nReturn a concise implementation/handoff result.`;
    messages.push({ role:'user', content });
    const payload = await this.#request('/api/chat', { method:'POST', body:JSON.stringify({ model:agent.model, messages, stream:false, options:{ num_predict:4096 } }) });
    const answer = payload.message ?? { role:'assistant', content:'' };
    messages.push(answer);
    this.sessions.set(sessionId, messages);
    return { agent_id:agent.id, task_id:task.id, session_id:sessionId, provider:'ollama', output:answer, completed_at:new Date().toISOString() };
  }
}
