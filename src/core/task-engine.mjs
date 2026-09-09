import fs from 'node:fs';
import path from 'node:path';
import { ResourceUnavailableError } from './errors.mjs';
import { validateTask } from './validation.mjs';

export class TaskEngine {
  constructor({ store, events, agentRegistry, sessionManager, resourceManager }) {
    this.store = store;
    this.events = events;
    this.agentRegistry = agentRegistry;
    this.sessionManager = sessionManager;
    this.resourceManager = resourceManager;
    this.running = new Set();
    this.waitingByAgent = new Map();
    this.resourceManager.onResumeRequested = async (agentId) => this.resumeAgentTasks(agentId);
  }

  ingestTask(task, { sourceFile = null } = {}) {
    validateTask(task);
    const existing = this.store.getTask(task.id);
    const effective = existing && ['RUNNING', 'COMPLETED', 'CANCELLED'].includes(existing.status)
      ? { ...task, status: existing.status }
      : task;
    this.store.upsertTask(effective, sourceFile);
    this.events.publish(existing ? 'TASK_UPDATED' : 'TASK_CREATED', {
      source: task.created_by,
      taskId: task.id,
      payload: { assigned_to: task.assigned_to, source_file: sourceFile }
    });
    queueMicrotask(() => this.evaluate(task.id).catch(error => {
      this.events.publish('TASK_FAILED', { source: 'batai', taskId: task.id, payload: { error: error.message } });
    }));
    return this.store.getTask(task.id);
  }

  ingestTaskFile(filePath) {
    const raw = fs.readFileSync(filePath, 'utf8');
    const task = JSON.parse(raw);
    return this.ingestTask(task, { sourceFile: filePath });
  }

  dependenciesSatisfied(task) {
    const dependencies = task.dependencies ?? [];
    return dependencies.every(id => this.store.getTask(id)?.status === 'COMPLETED');
  }

  async evaluate(taskId) {
    const task = this.store.getTask(taskId);
    if (!task) return null;
    if (['COMPLETED', 'CANCELLED', 'RUNNING', 'WAITING_RESOURCE', 'REVIEW'].includes(task.status)) return task;
    if (this.running.has(task.id)) return task;

    if (!this.dependenciesSatisfied(task)) {
      if (task.status !== 'BLOCKED') {
        this.store.setTaskStatus(task.id, 'BLOCKED');
        this.events.publish('TASK_BLOCKED', { source: 'batai', taskId: task.id, payload: { dependencies: task.dependencies ?? [] } });
      }
      return this.store.getTask(task.id);
    }

    for (const agentId of task.assigned_to) {
      const agent = this.agentRegistry.get(agentId);
      if (!agent) throw new Error(`Task ${task.id} assigned to unknown agent ${agentId}`);
      if (agent.status === 'TERMINATED') throw new Error(`Task ${task.id} assigned to terminated agent ${agentId}`);
    }

    return this.dispatch(task.id);
  }

  async dispatch(taskId) {
    if (this.running.has(taskId)) return this.store.getTask(taskId);
    const task = this.store.getTask(taskId);
    if (!task || ['COMPLETED', 'CANCELLED', 'RUNNING'].includes(task.status)) return task;

    this.running.add(taskId);
    this.store.setTaskStatus(taskId, 'RUNNING');
    this.events.publish('TASK_STARTED', { source: 'batai', taskId, payload: { assigned_to: task.assigned_to } });

    try {
      const results = await Promise.all(task.assigned_to.map(agentId => this.#runForAgent(agentId, task)));
      const combinedResult = { task_id: taskId, results, completed_at: new Date().toISOString() };
      this.store.setTaskResult(taskId, combinedResult);
      const targetStatus = task.execution?.requires_director_review ? 'REVIEW' : 'COMPLETED';
      this.store.setTaskStatus(taskId, targetStatus);
      this.events.publish(targetStatus === 'REVIEW' ? 'REVIEW_REQUIRED' : 'TASK_COMPLETED', {
        source: 'batai', target: targetStatus === 'REVIEW' ? 'director' : null, taskId, payload: combinedResult
      });

      if (targetStatus === 'COMPLETED') await this.#handleCompletion(taskId);
      return this.store.getTask(taskId);
    } catch (error) {
      if (error instanceof ResourceUnavailableError) {
        this.store.setTaskStatus(taskId, 'WAITING_RESOURCE');
        for (const agentId of task.assigned_to) {
          const set = this.waitingByAgent.get(agentId) ?? new Set();
          set.add(taskId);
          this.waitingByAgent.set(agentId, set);
          const agent = this.agentRegistry.get(agentId);
          if (agent && agent.status === 'RUNNING') this.agentRegistry.transition(agentId, 'WAITING_RESOURCE', { currentTaskId: taskId });
          this.resourceManager.markUnavailable(agentId, { status:error.status ?? 'RATE_LIMITED', resetAt:error.resetAt, details:{ message:error.message } });
        }
        return this.store.getTask(taskId);
      }
      this.store.setTaskStatus(taskId, 'FAILED');
      this.events.publish('TASK_FAILED', { source: 'batai', taskId, payload: { error: error.message } });
      if (task.on_failure?.notify) {
        this.events.publish('TASK_FAILED', { source: 'batai', target: task.on_failure.notify, taskId, payload: { error: error.message } });
      }
      throw error;
    } finally {
      this.running.delete(taskId);
    }
  }

  async #runForAgent(agentId, task) {
    let agent = this.agentRegistry.get(agentId);
    if (['CREATED', 'INITIALIZING'].includes(agent.status)) {
      if (agent.status === 'CREATED') this.agentRegistry.transition(agentId, 'INITIALIZING');
      this.agentRegistry.transition(agentId, 'READY');
      agent = this.agentRegistry.get(agentId);
    }
    if (agent.status === 'COMPLETED') {
      this.agentRegistry.transition(agentId, 'READY');
      agent = this.agentRegistry.get(agentId);
    }
    if (agent.status !== 'READY' && agent.status !== 'RUNNING') {
      if (['BLOCKED', 'PAUSED', 'FAILED', 'WAITING_RESOURCE'].includes(agent.status)) this.agentRegistry.transition(agentId, 'READY');
      agent = this.agentRegistry.get(agentId);
    }
    if (agent.status === 'READY') this.agentRegistry.transition(agentId, 'RUNNING', { currentTaskId: task.id });

    const session = await this.sessionManager.ensureSession(agent);
    const adapter = this.sessionManager.adapterFor(agent);
    this.events.publish('TASK_DISPATCHED', { source: 'batai', target: agentId, taskId: task.id, payload: { session_id: session.sessionId } });
    const result = await adapter.sendTask({ sessionId: session.sessionId, agent, task });
    this.events.publish('TASK_RESULT_RECORDED', { source: agentId, taskId: task.id, payload: result });
    const current = this.agentRegistry.get(agentId);
    if (current?.status === 'RUNNING') this.agentRegistry.transition(agentId, 'READY', { currentTaskId: null });
    return result;
  }

  async approveReview(taskId, reviewer = 'director') {
    const task = this.store.getTask(taskId);
    if (!task || task.status !== 'REVIEW') throw new Error(`Task ${taskId} is not awaiting review`);
    this.store.setTaskStatus(taskId, 'COMPLETED');
    this.events.publish('TASK_COMPLETED', { source: reviewer, taskId, payload: task.result ?? {} });
    await this.#handleCompletion(taskId);
    return this.store.getTask(taskId);
  }

  async #handleCompletion(taskId) {
    const task = this.store.getTask(taskId);
    const starts = task?.on_success?.start ?? [];
    for (const downstream of starts) await this.evaluate(downstream);
    if (task?.on_success?.notify) {
      this.events.publish('AGENT_COMPLETED', {
        source: task.assigned_to?.[0] ?? 'batai', target: task.on_success.notify, taskId,
        payload: { reason: task.on_success.reason ?? 'task_completed', result: task.result }
      });
    }
    for (const candidate of this.store.listTasks()) {
      if ((candidate.dependencies ?? []).includes(taskId) && ['PENDING', 'READY', 'BLOCKED'].includes(candidate.status)) {
        await this.evaluate(candidate.id);
      }
    }
  }

  async resumeAgentTasks(agentId) {
    const waiting = this.waitingByAgent.get(agentId);
    if (!waiting?.size) {
      for (const task of this.store.listTasks()) {
        if (task.status === 'WAITING_RESOURCE' && task.assigned_to.includes(agentId)) {
          const set = this.waitingByAgent.get(agentId) ?? new Set();
          set.add(task.id);
          this.waitingByAgent.set(agentId, set);
        }
      }
    }
    const tasks = [...(this.waitingByAgent.get(agentId) ?? [])];
    const agent = this.agentRegistry.get(agentId);
    if (agent?.status === 'WAITING_RESOURCE') this.agentRegistry.transition(agentId, 'READY');
    await this.sessionManager.resume(this.agentRegistry.get(agentId));
    for (const taskId of tasks) {
      this.store.setTaskStatus(taskId, 'READY');
      await this.evaluate(taskId);
      if (this.store.getTask(taskId)?.status !== 'WAITING_RESOURCE') this.waitingByAgent.get(agentId)?.delete(taskId);
    }
    this.events.publish('AGENT_RESUMED', { source: agentId, payload: { tasks } });
  }
}
