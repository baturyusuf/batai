import path from 'node:path';
import { ProviderAdapter } from './provider-adapter.mjs';
import { runCommand, commandAvailable } from './command-runner.mjs';
import { ResourceUnavailableError } from '../core/errors.mjs';
import { parseResetAtFromText } from './quota-parser.mjs';

function workspaceFor(agent, projectRoot) {
  return agent.worktree ? path.resolve(projectRoot, agent.worktree) : projectRoot;
}

function promptFor(task) {
  return [
    `BATAI TASK ${task.id}`,
    `Objective: ${task.objective}`,
    task.acceptance_criteria?.length ? `Acceptance criteria:\n- ${task.acceptance_criteria.join('\n- ')}` : '',
    task.inputs?.length ? `Inputs:\n- ${task.inputs.join('\n- ')}` : '',
    'Work only within your assigned scope. When finished, summarize changed files, tests, blockers, and handoff information concisely.'
  ].filter(Boolean).join('\n\n');
}

export class ClaudeCliAdapter extends ProviderAdapter {
  constructor({ projectRoot, command = 'claude', timeoutMs = 45 * 60 * 1000, permissionMode = 'auto' } = {}) {
    super('claude-cli');
    this.projectRoot = projectRoot;
    this.command = command;
    this.timeoutMs = timeoutMs;
    this.permissionMode = permissionMode;
    this.started = new Set();
  }

  async authenticate() {
    if (!(await commandAvailable(this.command))) return { ok:false, reason:'COMMAND_NOT_FOUND' };
    try {
      const result = await runCommand(this.command, ['auth','status'], { cwd:this.projectRoot, timeoutMs:8000 });
      return { ok:true, details:result.stdout.trim() };
    } catch (error) { return { ok:false, reason:'AUTH_REQUIRED', details:error.message }; }
  }

  async listModels() { return ['haiku','sonnet','opus']; }

  async createSession(agent) {
    return { sessionId:`batai-${agent.id}` };
  }

  async resumeSession(sessionId, agent) { if (agent?.id) this.started.add(agent.id); return { sessionId }; }

  async getUsageState(agent) {
    const auth = await this.authenticate();
    return auth.ok ? { status:'AVAILABLE' } : { status:auth.reason === 'COMMAND_NOT_FOUND' ? 'OFFLINE' : 'AUTH_REQUIRED', details:auth };
  }

  async sendTask({ sessionId, agent, task }) {
    const cwd = workspaceFor(agent, this.projectRoot);
    const common = ['-p','--output-format','json','--model',agent.model,'--effort',agent.reasoning_effort === 'extra_high' ? 'xhigh' : agent.reasoning_effort,
      '--permission-mode',this.permissionMode,'--name',sessionId];
    if (agent.max_turns) common.push('--max-turns', String(agent.max_turns));
    let args;
    if (this.started.has(agent.id)) args = ['-p','--resume',sessionId,'--output-format','json','--model',agent.model,'--effort',agent.reasoning_effort === 'extra_high' ? 'xhigh' : agent.reasoning_effort,
      '--permission-mode',this.permissionMode, promptFor(task)];
    else args = [...common, promptFor(task)];
    try {
      const { stdout, stderr } = await runCommand(this.command, args, { cwd, timeoutMs:this.timeoutMs });
      this.started.add(agent.id);
      let parsed = null;
      try { parsed = JSON.parse(stdout); } catch { parsed = { text:stdout.trim() }; }
      return { agent_id:agent.id, task_id:task.id, session_id:sessionId, provider:'claude-cli', output:parsed, stderr:stderr.trim(), completed_at:new Date().toISOString() };
    } catch (error) {
      const text = `${error.stderr ?? ''}\n${error.stdout ?? ''}\n${error.message}`.toLowerCase();
      if (/rate.?limit|usage.?limit|limit reached|quota/.test(text)) throw new ResourceUnavailableError(error.message, { status:'RATE_LIMITED', resetAt:parseResetAtFromText(text) });
      if (/auth|login|sign in/.test(text)) throw new ResourceUnavailableError(error.message, { status:'AUTH_REQUIRED' });
      throw error;
    }
  }
}
