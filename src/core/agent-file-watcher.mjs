import fs from 'node:fs';
import path from 'node:path';

export class AgentFileWatcher {
  constructor({ projectRoot, agentRegistry, events, debounceMs = 80, onError = console.error }) {
    this.root = path.join(projectRoot, '.batai', 'agents');
    this.agentRegistry = agentRegistry;
    this.events = events;
    this.debounceMs = debounceMs;
    this.onError = onError;
    this.rootWatcher = null;
    this.agentWatchers = new Map();
    this.debounce = new Map();
  }

  start() {
    fs.mkdirSync(this.root, { recursive: true });
    for (const entry of fs.readdirSync(this.root, { withFileTypes:true })) if (entry.isDirectory()) this.#watchAgent(entry.name);
    if (!this.rootWatcher) {
      this.rootWatcher = fs.watch(this.root, (_event, filename) => {
        if (!filename) return;
        const id = filename.toString();
        this.#schedule(`dir:${id}`, () => this.#watchAgent(id));
      });
    }
  }

  #watchAgent(agentId) {
    const dir = path.join(this.root, agentId);
    if (!fs.existsSync(dir) || !fs.statSync(dir).isDirectory()) return;
    if (!this.agentWatchers.has(agentId)) {
      const watcher = fs.watch(dir, (_event, filename) => {
        const name = filename?.toString();
        if (!name) return;
        if (name === 'config.json') this.#schedule(`config:${agentId}`, () => this.#loadConfig(agentId));
        if (name === 'DIRECTIVES.md') this.#schedule(`directives:${agentId}`, () => {
          const file = path.join(dir, 'DIRECTIVES.md');
          if (!fs.existsSync(file)) return;
          this.events.publish('DIRECTIVE_UPDATED', {
            source:'director', target:agentId,
            payload:{ path:file, content:fs.readFileSync(file,'utf8') }
          });
        });
      });
      this.agentWatchers.set(agentId, watcher);
    }
    this.#loadConfig(agentId);
  }

  #loadConfig(agentId) {
    const file = path.join(this.root, agentId, 'config.json');
    if (!fs.existsSync(file)) return;
    try {
      const config = JSON.parse(fs.readFileSync(file,'utf8'));
      this.agentRegistry.register(config);
    } catch (error) { this.onError(error, file); }
  }

  #schedule(key, fn) {
    const old = this.debounce.get(key);
    if (old) clearTimeout(old);
    this.debounce.set(key, setTimeout(() => {
      this.debounce.delete(key);
      try { fn(); } catch (error) { this.onError(error, key); }
    }, this.debounceMs));
  }

  close() {
    this.rootWatcher?.close();
    this.rootWatcher = null;
    for (const watcher of this.agentWatchers.values()) watcher.close();
    this.agentWatchers.clear();
    for (const timer of this.debounce.values()) clearTimeout(timer);
    this.debounce.clear();
  }
}
