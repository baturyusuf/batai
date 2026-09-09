import fs from 'node:fs';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
const execFileAsync = promisify(execFile);

export class GitWorktreeManager {
  constructor({ projectRoot }) {
    this.projectRoot = projectRoot;
    this.worktreesRoot = path.join(projectRoot, '.batai-worktrees');
  }

  async isRepository() {
    try {
      const { stdout } = await execFileAsync('git', ['rev-parse', '--is-inside-work-tree'], { cwd: this.projectRoot });
      return stdout.trim() === 'true';
    } catch { return false; }
  }

  async create({ agentId, taskId, baseRef = 'HEAD' }) {
    if (!(await this.isRepository())) throw new Error('Project is not a Git repository');
    fs.mkdirSync(this.worktreesRoot, { recursive: true });
    const safeTask = taskId.replace(/[^a-zA-Z0-9._-]/g, '-');
    const branch = `batai/${agentId}/${safeTask}`;
    const directory = path.join(this.worktreesRoot, `${agentId}-${safeTask}`);
    await execFileAsync('git', ['worktree', 'add', '-b', branch, directory, baseRef], { cwd: this.projectRoot });
    return { branch, directory };
  }

  async remove(directory, { force = false } = {}) {
    const args = ['worktree', 'remove'];
    if (force) args.push('--force');
    args.push(directory);
    await execFileAsync('git', args, { cwd: this.projectRoot });
  }
}
