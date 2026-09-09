import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { BataiRuntime } from '../src/core/batai-runtime.mjs';
import { MockProviderAdapter } from '../src/providers/mock-provider.mjs';
import { ValidationError, InvalidTransitionError } from '../src/core/errors.mjs';

function tempProject() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'batai-test-'));
  fs.mkdirSync(path.join(root, '.batai', 'tasks'), { recursive: true });
  fs.mkdirSync(path.join(root, '.batai', 'agents', 'worker'), { recursive: true });
  fs.writeFileSync(path.join(root, '.batai', 'agents', 'worker', 'config.json'), JSON.stringify({
    id:'worker', name:'Worker', role_template:'SoftwareEngineer', parent_agent_id:'director', provider:'mock', model:'mock-medium',
    reasoning_effort:'medium', auth_mode:'local', worktree:null, allowed_paths:['**/*'], tools:['shell'], constraints:[],
    max_turns:10, lifetime:'project', status:'READY'
  }, null, 2));
  return root;
}

function task(id, overrides = {}) {
  return {
    id, created_by:'director', objective:`Objective ${id}`, assigned_to:['worker'], dependencies:[], acceptance_criteria:['done'],
    inputs:[], outputs:[], status:'READY', execution:{parallel:false,requires_director_review:false}, on_success:{}, on_failure:{notify:'director'},
    ...overrides
  };
}

async function waitFor(predicate, timeoutMs = 1500) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const value = predicate();
    if (value) return value;
    await new Promise(r => setTimeout(r, 15));
  }
  throw new Error('waitFor timeout');
}

test('invalid task is rejected', async () => {
  const root = tempProject();
  const runtime = new BataiRuntime({ projectRoot: root });
  await runtime.start();
  assert.throws(() => runtime.taskEngine.ingestTask({ id:'x' }), ValidationError);
  await runtime.close();
});

test('valid task is persisted and completed', async () => {
  const root = tempProject();
  const adapter = new MockProviderAdapter({ latencyMs:2 });
  const runtime = new BataiRuntime({ projectRoot: root, adapters:new Map([['mock',adapter]]) });
  await runtime.start();
  runtime.taskEngine.ingestTask(task('T1'));
  await waitFor(() => runtime.store.getTask('T1')?.status === 'COMPLETED');
  assert.equal(runtime.store.getTask('T1').status, 'COMPLETED');
  assert.equal(adapter.calls.filter(c => c.taskId === 'T1').length, 1);
  await runtime.close();
});

test('dependency blocked task starts automatically after upstream completion', async () => {
  const root = tempProject();
  const adapter = new MockProviderAdapter({ latencyMs:20 });
  const runtime = new BataiRuntime({ projectRoot: root, adapters:new Map([['mock',adapter]]) });
  await runtime.start();
  runtime.taskEngine.ingestTask(task('T2', { dependencies:['T1'] }));
  await waitFor(() => runtime.store.getTask('T2')?.status === 'BLOCKED');
  runtime.taskEngine.ingestTask(task('T1'));
  await waitFor(() => runtime.store.getTask('T1')?.status === 'COMPLETED');
  await waitFor(() => runtime.store.getTask('T2')?.status === 'COMPLETED');
  assert.deepEqual(adapter.calls.map(c => c.taskId), ['T1','T2']);
  await runtime.close();
});

test('duplicate task ingestion does not duplicate execution', async () => {
  const root = tempProject();
  const adapter = new MockProviderAdapter({ latencyMs:30 });
  const runtime = new BataiRuntime({ projectRoot: root, adapters:new Map([['mock',adapter]]) });
  await runtime.start();
  const t = task('T1');
  runtime.taskEngine.ingestTask(t);
  runtime.taskEngine.ingestTask(t);
  runtime.taskEngine.ingestTask(t);
  await waitFor(() => runtime.store.getTask('T1')?.status === 'COMPLETED');
  assert.equal(adapter.calls.filter(c => c.taskId === 'T1').length, 1);
  await runtime.close();
});

test('review-required task pauses at REVIEW until Director approval', async () => {
  const root = tempProject();
  const runtime = new BataiRuntime({ projectRoot: root });
  await runtime.start();
  runtime.taskEngine.ingestTask(task('R1', { execution:{parallel:false,requires_director_review:true} }));
  await waitFor(() => runtime.store.getTask('R1')?.status === 'REVIEW');
  await runtime.taskEngine.approveReview('R1');
  assert.equal(runtime.store.getTask('R1').status, 'COMPLETED');
  await runtime.close();
});

test('invalid agent transition is rejected', async () => {
  const root = tempProject();
  const runtime = new BataiRuntime({ projectRoot: root });
  await runtime.start();
  assert.throws(() => runtime.agentRegistry.transition('worker','CREATED'), InvalidTransitionError);
  await runtime.close();
});

test('rate-limited task parks and resumes without new task definition', async () => {
  const root = tempProject();
  const adapter = new MockProviderAdapter({ latencyMs:2 });
  adapter.setUsage('worker', { status:'RATE_LIMITED', resetAt:null });
  const runtime = new BataiRuntime({ projectRoot: root, adapters:new Map([['mock',adapter]]) });
  await runtime.start();
  runtime.taskEngine.ingestTask(task('Q1'));
  await waitFor(() => runtime.store.getTask('Q1')?.status === 'WAITING_RESOURCE');
  assert.equal(runtime.agentRegistry.get('worker').status, 'WAITING_RESOURCE');
  adapter.setUsage('worker', { status:'AVAILABLE', resetAt:null });
  await runtime.taskEngine.resumeAgentTasks('worker');
  await waitFor(() => runtime.store.getTask('Q1')?.status === 'COMPLETED');
  assert.equal(adapter.calls.filter(c => c.taskId === 'Q1').length, 1);
  await runtime.close();
});

test('filesystem task watcher dispatches newly written task', async () => {
  const root = tempProject();
  const runtime = new BataiRuntime({ projectRoot: root, watcherDebounceMs:20 });
  await runtime.start();
  const file = path.join(root,'.batai','tasks','WATCH.json');
  fs.writeFileSync(file, JSON.stringify(task('WATCH'), null, 2));
  await waitFor(() => runtime.store.getTask('WATCH')?.status === 'COMPLETED', 2000);
  assert.equal(runtime.store.getTask('WATCH').status, 'COMPLETED');
  await runtime.close();
});

test('runtime state survives restart', async () => {
  const root = tempProject();
  const runtime1 = new BataiRuntime({ projectRoot: root });
  await runtime1.start();
  runtime1.taskEngine.ingestTask(task('BLOCKED', { dependencies:['NEVER'] }));
  await waitFor(() => runtime1.store.getTask('BLOCKED')?.status === 'BLOCKED');
  await runtime1.close();
  const runtime2 = new BataiRuntime({ projectRoot: root });
  await runtime2.start();
  assert.equal(runtime2.store.getTask('BLOCKED').status, 'BLOCKED');
  await runtime2.close();
});

test('DIRECTIVES.md change emits DIRECTIVE_UPDATED for the target agent', async () => {
  const root = tempProject();
  const runtime = new BataiRuntime({ projectRoot: root, watcherDebounceMs:20 });
  await runtime.start();
  const seen = [];
  runtime.events.on('DIRECTIVE_UPDATED', event => seen.push(event));
  fs.writeFileSync(path.join(root,'.batai','agents','worker','DIRECTIVES.md'), '# New directives\n- Run tests\n');
  await waitFor(() => seen.length === 1, 1500);
  assert.equal(seen[0].target, 'worker');
  assert.match(seen[0].payload.content, /Run tests/);
  await runtime.close();
});

test('GitWorktreeManager creates an isolated branch/worktree', async () => {
  const root = tempProject();
  const { execFileSync } = await import('node:child_process');
  execFileSync('git',['init','-b','main'],{cwd:root});
  execFileSync('git',['config','user.email','batai-test@example.invalid'],{cwd:root});
  execFileSync('git',['config','user.name','Batai Test'],{cwd:root});
  fs.writeFileSync(path.join(root,'seed.txt'),'seed\n');
  execFileSync('git',['add','.'],{cwd:root});
  execFileSync('git',['commit','-m','seed'],{cwd:root});
  const runtime = new BataiRuntime({ projectRoot: root });
  await runtime.start();
  const wt = await runtime.git.create({ agentId:'worker', taskId:'T-WT' });
  assert.equal(fs.existsSync(wt.directory), true);
  assert.equal(wt.branch, 'batai/worker/T-WT');
  runtime.directorTools.bindWorktree('worker', wt.directory);
  assert.equal(runtime.agentRegistry.get('worker').worktree, wt.directory);
  const persisted = JSON.parse(fs.readFileSync(path.join(root,'.batai','agents','worker','config.json'),'utf8'));
  assert.equal(persisted.worktree, wt.directory);
  await runtime.git.remove(wt.directory, { force:true });
  await runtime.close();
});

test('provider session metadata survives runtime restart', async () => {
  const root = tempProject();
  const adapter1 = new MockProviderAdapter({ latencyMs:2 });
  const runtime1 = new BataiRuntime({ projectRoot:root, adapters:new Map([['mock',adapter1]]) });
  await runtime1.start();
  runtime1.taskEngine.ingestTask(task('S1'));
  await waitFor(() => runtime1.store.getTask('S1')?.status === 'COMPLETED');
  const stored = runtime1.store.getSession('worker');
  assert.ok(stored?.sessionId);
  await runtime1.close();

  const adapter2 = new MockProviderAdapter({ latencyMs:2 });
  const runtime2 = new BataiRuntime({ projectRoot:root, adapters:new Map([['mock',adapter2]]) });
  await runtime2.start();
  assert.equal(runtime2.store.getSession('worker').sessionId, stored.sessionId);
  runtime2.taskEngine.ingestTask(task('S2'));
  await waitFor(() => runtime2.store.getTask('S2')?.status === 'COMPLETED');
  assert.equal(runtime2.store.getSession('worker').sessionId, stored.sessionId);
  await runtime2.close();
});

test('organizational memory writes journals, reports and handoffs for chained tasks', async () => {
  const root = tempProject();
  const adapter = new MockProviderAdapter({ latencyMs:2 });
  const runtime = new BataiRuntime({ projectRoot: root, adapters:new Map([['mock',adapter]]) });
  await runtime.start();

  runtime.taskEngine.ingestTask(task('MEM-2', { dependencies:['MEM-1'] }));
  runtime.taskEngine.ingestTask(task('MEM-1', { on_success:{ start:['MEM-2'] } }));

  await waitFor(() => runtime.store.getTask('MEM-1')?.status === 'COMPLETED');
  await waitFor(() => runtime.store.getTask('MEM-2')?.status === 'COMPLETED');
  await waitFor(() => fs.existsSync(path.join(root,'.batai','handoffs','MEM-1.json')));

  const journalDir = path.join(root,'.batai','journals','worker');
  const journalFiles = fs.readdirSync(journalDir).filter(name => name.endsWith('.json'));
  assert.ok(journalFiles.length >= 2);

  const report = JSON.parse(fs.readFileSync(path.join(root,'.batai','reports','MEM-1.json'),'utf8'));
  const handoff = JSON.parse(fs.readFileSync(path.join(root,'.batai','handoffs','MEM-1.json'),'utf8'));
  assert.equal(report.task_id, 'MEM-1');
  assert.deepEqual(handoff.next_tasks, ['MEM-2']);
  assert.equal(handoff.status, 'COMPLETED');

  await runtime.close();
});

test('GOD messages are persisted with highest authority and delivered to Director inbox', async () => {
  const root = tempProject();
  const runtime = new BataiRuntime({ projectRoot:root });
  await runtime.start();
  const message = runtime.authority.submitGodMessage({ content:'Use PostgreSQL for this project.', scope:'project' });
  assert.equal(message.authority,100);
  assert.equal(message.source,'GOD');
  const inbox = runtime.authority.listInbox('director',{pendingOnly:true});
  assert.equal(inbox.length,1);
  assert.equal(inbox[0].content,'Use PostgreSQL for this project.');
  assert.equal(fs.existsSync(message.file),true);
  runtime.authority.acknowledgeMessage(message.id);
  assert.equal(runtime.store.getAuthorityMessage(message.id).status,'ACKNOWLEDGED');
  await runtime.close();
});

test('Director can request a GOD decision and GOD resolution is locked in decision ledger', async () => {
  const root = tempProject();
  const runtime = new BataiRuntime({ projectRoot:root });
  await runtime.start();
  const decision = runtime.authority.createDecision({
    id:'DEC-DB', topic:'database', question:'Which persistent database should the project use?',
    director_recommendation:'PostgreSQL', alternatives:['PostgreSQL','SQLite'], impact:'high', requires:'GOD'
  });
  assert.equal(decision.status,'OPEN');
  const resolved = runtime.authority.resolveDecision('DEC-DB','PostgreSQL',{decidedBy:'GOD'});
  assert.equal(resolved.status,'LOCKED');
  assert.equal(resolved.effective_decision,'PostgreSQL');
  assert.equal(runtime.store.getDecision('DEC-DB').decided_by,'GOD');
  const events = runtime.store.listEvents(20);
  assert.ok(events.some(e => e.type === 'GOD_DECISION_REQUIRED'));
  assert.ok(events.some(e => e.type === 'GOD_DECISION_RESOLVED'));
  await runtime.close();
});

test('policy engine limits hierarchy depth and requires GOD approval for permanent agents', async () => {
  const root = tempProject();
  fs.mkdirSync(path.join(root,'.batai','agents','director'),{recursive:true});
  fs.writeFileSync(path.join(root,'.batai','agents','director','config.json'), JSON.stringify({
    id:'director',name:'Director',role_template:'Director',parent_agent_id:null,provider:'mock',model:'mock-high',reasoning_effort:'high',auth_mode:'local',worktree:null,
    allowed_paths:['**/*'],tools:['shell'],constraints:[],max_turns:30,lifetime:'project',status:'READY'
  },null,2));
  const runtime = new BataiRuntime({projectRoot:root});
  await runtime.start();
  const config = (id,parent,lifetime='project') => ({
    id,name:id,role_template:'SoftwareEngineer',parent_agent_id:parent,provider:'mock',model:'mock-medium',reasoning_effort:'medium',auth_mode:'local',worktree:null,
    allowed_paths:['**/*'],tools:['shell'],constraints:[],max_turns:10,lifetime,status:'READY'
  });
  runtime.directorTools.createAgent(config('lead','director'));
  runtime.directorTools.createAgent(config('child','lead'));
  assert.throws(() => runtime.directorTools.createAgent(config('too-deep','child')), /Hierarchy depth/);
  assert.throws(() => runtime.directorTools.createAgent(config('permanent','director','permanent')), /GOD approval/);
  const permanent = runtime.directorTools.createAgent(config('permanent','director','permanent'),{godApproved:true});
  assert.equal(permanent.lifetime,'permanent');
  await runtime.close();
});
