import fs from 'node:fs';
import path from 'node:path';
import { RuntimeStore } from './runtime-store.mjs';
import { EventEngine } from './event-engine.mjs';
import { AgentRegistry } from './agent-registry.mjs';
import { SessionManager } from './session-manager.mjs';
import { ResourceManager } from './resource-manager.mjs';
import { TaskEngine } from './task-engine.mjs';
import { TaskWatcher } from './task-watcher.mjs';
import { DirectorTools } from './director-tools.mjs';
import { GitWorktreeManager } from './git-worktree-manager.mjs';
import { AgentFileWatcher } from './agent-file-watcher.mjs';
import { MockProviderAdapter } from '../providers/mock-provider.mjs';
import { GitHubCliManager } from './github-cli-manager.mjs';
import { OrganizationalMemory } from './organizational-memory.mjs';
import { AuthorityController } from './authority-controller.mjs';
import { PolicyEngine } from './policy-engine.mjs';

export class BataiRuntime {
  constructor({ projectRoot, runtimeDir = null, adapters = null, watcherDebounceMs = 80 } = {}) {
    if (!projectRoot) throw new Error('projectRoot is required');
    this.projectRoot = path.resolve(projectRoot);
    fs.mkdirSync(path.join(this.projectRoot, '.batai', 'tasks'), { recursive: true });
    const stateDir = runtimeDir ?? path.join(this.projectRoot, '.runtime');
    fs.mkdirSync(stateDir, { recursive: true });
    this.store = new RuntimeStore(path.join(stateDir, 'runtime.sqlite'));
    this.events = new EventEngine(this.store);
    this.authority = new AuthorityController({ projectRoot:this.projectRoot, store:this.store, events:this.events });
    this.policyEngine = new PolicyEngine({ projectRoot:this.projectRoot, store:this.store });
    this.adapters = adapters ?? new Map([['mock', new MockProviderAdapter()]]);
    this.memory = new OrganizationalMemory({ projectRoot:this.projectRoot, store:this.store, events:this.events });
    this.agentRegistry = new AgentRegistry({ store: this.store, events: this.events, projectRoot: this.projectRoot });
    this.sessionManager = new SessionManager({ adapters: this.adapters, store: this.store });
    this.resourceManager = new ResourceManager({
      store: this.store, events: this.events, agentRegistry: this.agentRegistry, sessionManager: this.sessionManager
    });
    this.taskEngine = new TaskEngine({
      store: this.store, events: this.events, agentRegistry: this.agentRegistry,
      sessionManager: this.sessionManager, resourceManager: this.resourceManager
    });
    this.taskWatcher = new TaskWatcher({
      tasksDir: path.join(this.projectRoot, '.batai', 'tasks'), taskEngine: this.taskEngine,
      debounceMs: watcherDebounceMs,
      onError: (error, file) => this.events.publish('TASK_FAILED', { source: 'watcher', payload: { file, error: error.message } })
    });
    this.agentFileWatcher = new AgentFileWatcher({ projectRoot: this.projectRoot, agentRegistry: this.agentRegistry, events: this.events, debounceMs: watcherDebounceMs, onError: (error, file) => this.events.publish('TASK_FAILED', { source:'agent-watcher', payload:{ file, error:error.message } }) });
    this.directorTools = new DirectorTools({ projectRoot: this.projectRoot, agentRegistry: this.agentRegistry, taskEngine: this.taskEngine, store: this.store, authority:this.authority, policyEngine:this.policyEngine });
    this.git = new GitWorktreeManager({ projectRoot: this.projectRoot });
    this.github = new GitHubCliManager({ projectRoot: this.projectRoot });
  }

  async start() {
    this.agentRegistry.loadFromProject();
    await this.taskWatcher.initialScan();
    this.taskWatcher.start();
    this.agentFileWatcher.start();
    return this;
  }

  state() { return this.directorTools.readProjectState(); }

  async close() {
    this.taskWatcher.close();
    this.agentFileWatcher.close();
    this.memory.dispose();
    this.resourceManager.dispose();
    this.store.close();
  }
}
