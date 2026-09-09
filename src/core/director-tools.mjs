import fs from 'node:fs';
import path from 'node:path';
import { validateAgentConfig, validateTask } from './validation.mjs';

function atomicJsonWrite(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  const tmp = `${filePath}.${process.pid}.${Date.now()}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`);
  fs.renameSync(tmp, filePath);
}

export class DirectorTools {
  constructor({ projectRoot, agentRegistry, taskEngine, store, authority = null, policyEngine = null }) {
    this.projectRoot = projectRoot;
    this.agentRegistry = agentRegistry;
    this.taskEngine = taskEngine;
    this.store = store;
    this.authority = authority;
    this.policyEngine = policyEngine;
  }

  createAgent(config, options = {}) {
    validateAgentConfig(config);
    this.policyEngine?.validateAgentCreation(config, options);
    const saved = this.agentRegistry.register(config, { persistFile: true });
    return saved;
  }

  assignTask(task) {
    validateTask(task);
    const filePath = path.join(this.projectRoot, '.batai', 'tasks', `${task.id}.json`);
    atomicJsonWrite(filePath, task);
    return { task, filePath };
  }

  async approveTask(taskId) {
    return this.taskEngine.approveReview(taskId, 'director');
  }

  bindWorktree(agentId, worktree) {
    const current = this.agentRegistry.get(agentId);
    if (!current) throw new Error(`Agent not found: ${agentId}`);
    const updated = { ...current, worktree };
    return this.agentRegistry.register(updated, { persistFile:true });
  }

  readProjectState() {
    return {
      agents: this.store.listAgents(),
      tasks: this.store.listTasks(),
      resources: this.store.listResources(),
      events: this.store.listEvents(200),
      decisions: this.store.listDecisions?.({limit:200}) ?? [],
      director_inbox: this.store.listAuthorityMessages?.({target:'director',limit:200}) ?? []
    };
  }
}
