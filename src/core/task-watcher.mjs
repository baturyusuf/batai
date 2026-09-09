import fs from 'node:fs';
import path from 'node:path';

export class TaskWatcher {
  constructor({ tasksDir, taskEngine, debounceMs = 80, onError = console.error }) {
    this.tasksDir = tasksDir;
    this.taskEngine = taskEngine;
    this.debounceMs = debounceMs;
    this.onError = onError;
    this.watcher = null;
    this.debounce = new Map();
  }

  ensureDir() { fs.mkdirSync(this.tasksDir, { recursive: true }); }

  async initialScan() {
    this.ensureDir();
    const files = fs.readdirSync(this.tasksDir).filter(name => name.endsWith('.json')).sort();
    for (const name of files) {
      try { this.taskEngine.ingestTaskFile(path.join(this.tasksDir, name)); }
      catch (error) { this.onError(error, path.join(this.tasksDir, name)); }
    }
  }

  start() {
    this.ensureDir();
    if (this.watcher) return;
    this.watcher = fs.watch(this.tasksDir, (_eventType, filename) => {
      if (!filename?.endsWith('.json')) return;
      const filePath = path.join(this.tasksDir, filename);
      const existing = this.debounce.get(filePath);
      if (existing) clearTimeout(existing);
      const timer = setTimeout(() => {
        this.debounce.delete(filePath);
        if (!fs.existsSync(filePath)) return;
        try { this.taskEngine.ingestTaskFile(filePath); }
        catch (error) { this.onError(error, filePath); }
      }, this.debounceMs);
      this.debounce.set(filePath, timer);
    });
  }

  close() {
    this.watcher?.close();
    this.watcher = null;
    for (const timer of this.debounce.values()) clearTimeout(timer);
    this.debounce.clear();
  }
}
