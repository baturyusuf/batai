import fs from 'node:fs';
import path from 'node:path';

function safeTimestamp(iso) { return iso.replace(/[:.]/g, '-'); }
function atomicWrite(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive:true });
  const temp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(temp, `${JSON.stringify(value, null, 2)}\n`);
  fs.renameSync(temp, file);
}

export class OrganizationalMemory {
  constructor({ projectRoot, store, events }) {
    this.projectRoot = projectRoot;
    this.store = store;
    this.events = events;
    this.listener = event => this.#onEvent(event);
    events.on('*', this.listener);
  }

  #journal(agentId, event) {
    if (!agentId) return;
    const dir = path.join(this.projectRoot,'.batai','journals',agentId);
    const file = path.join(dir, `${safeTimestamp(event.timestamp)}-${event.type}-${event.id}.json`);
    atomicWrite(file, {
      event:event.type, agent:agentId, task_id:event.task_id ?? null, timestamp:event.timestamp,
      payload:event.payload ?? {}
    });
  }

  #onEvent(event) {
    if (event.type === 'TASK_DISPATCHED') this.#journal(event.target, event);
    if (event.type === 'TASK_RESULT_RECORDED') this.#journal(event.source, event);
    if (['AGENT_RATE_LIMITED','AGENT_RESUMED','DIRECTIVE_UPDATED'].includes(event.type)) this.#journal(event.target ?? event.source, event);
    if (['TASK_COMPLETED','REVIEW_REQUIRED'].includes(event.type) && event.task_id) {
      const task = this.store.getTask(event.task_id);
      if (!task) return;
      const targets = task.on_success?.start ?? [];
      const report = {
        task_id:task.id,
        status:task.status,
        completed_at:event.timestamp,
        assigned_to:task.assigned_to,
        objective:task.objective,
        result:task.result ?? event.payload ?? {},
        next_tasks:targets
      };
      atomicWrite(path.join(this.projectRoot,'.batai','reports',`${task.id}.json`), report);
      if (targets.length) atomicWrite(path.join(this.projectRoot,'.batai','handoffs',`${task.id}.json`), report);
    }
  }

  dispose() { this.events.off('*', this.listener); }
}
