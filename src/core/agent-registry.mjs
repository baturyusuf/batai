import fs from 'node:fs';
import path from 'node:path';
import { AGENT_TRANSITIONS } from './constants.mjs';
import { InvalidTransitionError } from './errors.mjs';
import { validateAgentConfig } from './validation.mjs';

export class AgentRegistry {
  constructor({ store, events, projectRoot }) {
    this.store = store;
    this.events = events;
    this.projectRoot = projectRoot;
  }

  register(config, { persistFile = false } = {}) {
    validateAgentConfig(config);
    const existed = this.store.getAgent(config.id);
    const saved = this.store.upsertAgent(config);
    if (persistFile) this.writeConfigFile(saved);
    if (!existed) this.events.publish('AGENT_CREATED', { source: 'batai', target: config.id, payload: { agent: saved } });
    if (saved.status === 'READY') this.events.publish('AGENT_READY', { source: saved.id });
    return saved;
  }

  writeConfigFile(config) {
    const dir = path.join(this.projectRoot, '.batai', 'agents', config.id);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'config.json'), `${JSON.stringify(config, null, 2)}\n`);
  }

  loadFromProject() {
    const root = path.join(this.projectRoot, '.batai', 'agents');
    if (!fs.existsSync(root)) return [];
    const loaded = [];
    for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      const configPath = path.join(root, entry.name, 'config.json');
      if (!fs.existsSync(configPath)) continue;
      const config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
      loaded.push(this.register(config));
    }
    return loaded;
  }

  transition(id, to, { currentTaskId = undefined } = {}) {
    const agent = this.store.getAgent(id);
    if (!agent) throw new Error(`Agent not found: ${id}`);
    if (agent.status === to) return agent;
    if (!AGENT_TRANSITIONS[agent.status]?.has(to)) throw new InvalidTransitionError('agent', agent.status, to);
    const updated = this.store.setAgentStatus(id, to, currentTaskId);
    this.events.publish('AGENT_STATUS_CHANGED', {
      source: id,
      payload: { from: agent.status, to, currentTaskId: currentTaskId ?? updated.current_task_id }
    });
    return updated;
  }

  get(id) { return this.store.getAgent(id); }
  list() { return this.store.listAgents(); }
}
