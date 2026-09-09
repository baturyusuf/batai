import fs from 'node:fs';
import os from 'node:os';
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
    'Complete the task autonomously within the repository scope. Run relevant tests. End with a concise implementation and handoff summary.'
  ].filter(Boolean).join('\n\n');
}

export class CodexCliAdapter extends ProviderAdapter {
  constructor({ projectRoot, command = 'codex', timeoutMs = 45 * 60 * 1000, sandbox = 'workspace-write' } = {}) {
    super('codex-cli');
    this.projectRoot = projectRoot;
    this.command = command;
    this.timeoutMs = timeoutMs;
    this.sandbox = sandbox;
    this.started = new Set();
  }

  async authenticate() {
    if (!(await commandAvailable(this.command))) return { ok:false, reason:'COMMAND_NOT_FOUND' };
    try {
      const result = await runCommand(this.command, ['login','status'], { cwd:this.projectRoot, timeoutMs:8000 });
      return { ok:true, details:result.stdout.trim() || result.stderr.trim() };
    } catch (error) { return { ok:false, reason:'AUTH_REQUIRED', details:error.message }; }
  }

  async createSession(agent) { return { sessionId:`codex-last:${agent.id}` }; }
  async resumeSession(sessionId, agent) { if (agent?.id) this.started.add(agent.id); return { sessionId }; }

  async getUsageState() {
    const auth = await this.authenticate();
    return auth.ok ? { status:'AVAILABLE' } : { status:auth.reason === 'COMMAND_NOT_FOUND' ? 'OFFLINE' : 'AUTH_REQUIRED', details:auth };
  }

  async sendTask({ sessionId, agent, task }) {
    const cwd = workspaceFor(agent, this.projectRoot);
    const outputFile = path.join(os.tmpdir(), `batai-codex-${agent.id}-${Date.now()}.txt`);
    const prompt = promptFor(task);
    const effort = agent.reasoning_effort === 'extra_high' ? 'xhigh' : agent.reasoning_effort;
    let args;
    if (this.started.has(agent.id)) {
      args = ['exec','resume','--last', prompt];
    } else {
      args = ['exec','--model',agent.model,'--sandbox',this.sandbox,'--output-last-message',outputFile,
        '-c',`model_reasoning_effort=${JSON.stringify(effort)}`, prompt];
    }
    try {
      const { stdout, stderr } = await runCommand(this.command, args, { cwd, timeoutMs:this.timeoutMs });
      this.started.add(agent.id);
      const finalText = fs.existsSync(outputFile) ? fs.readFileSync(outputFile,'utf8').trim() : stdout.trim();
      try { fs.unlinkSync(outputFile); } catch {}
      return { agent_id:agent.id, task_id:task.id, session_id:sessionId, provider:'codex-cli', output:{ text:finalText }, stdout:stdout.trim(), stderr:stderr.trim(), completed_at:new Date().toISOString() };
    } catch (error) {
      try { fs.unlinkSync(outputFile); } catch {}
      const text = `${error.stderr ?? ''}\n${error.stdout ?? ''}\n${error.message}`.toLowerCase();
      if (/rate.?limit|usage.?limit|limit reached|quota/.test(text)) throw new ResourceUnavailableError(error.message, { status:'RATE_LIMITED', resetAt:parseResetAtFromText(text) });
      if (/auth|login|sign in/.test(text)) throw new ResourceUnavailableError(error.message, { status:'AUTH_REQUIRED' });
      throw error;
    }
  }
}
