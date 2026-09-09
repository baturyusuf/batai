import { runCommand, commandAvailable } from '../providers/command-runner.mjs';

export class GitHubCliManager {
  constructor({ projectRoot, command = 'gh' }) { this.projectRoot = projectRoot; this.command = command; }
  async isAvailable() { return commandAvailable(this.command); }
  async authStatus() {
    if (!(await this.isAvailable())) return { ok:false, reason:'COMMAND_NOT_FOUND' };
    try { const r = await runCommand(this.command,['auth','status'],{cwd:this.projectRoot,timeoutMs:8000}); return {ok:true, details:r.stderr || r.stdout}; }
    catch(error){ return {ok:false, reason:'AUTH_REQUIRED', details:error.message}; }
  }
  async createIssue({ title, body = '', labels = [] }) {
    const args = ['issue','create','--title',title,'--body',body];
    for (const label of labels) args.push('--label',label);
    const r = await runCommand(this.command,args,{cwd:this.projectRoot});
    return { url:r.stdout.trim() };
  }
  async viewIssue(number) {
    const r = await runCommand(this.command,['issue','view',String(number),'--json','number,title,body,state,url,labels,assignees'],{cwd:this.projectRoot});
    return JSON.parse(r.stdout);
  }
  async createPullRequest({ title, body = '', base = 'main', head = null }) {
    const args = ['pr','create','--title',title,'--body',body,'--base',base];
    if (head) args.push('--head',head);
    const r = await runCommand(this.command,args,{cwd:this.projectRoot});
    return { url:r.stdout.trim() };
  }
}
