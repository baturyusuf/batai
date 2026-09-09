#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import { BataiRuntime } from '../core/batai-runtime.mjs';
import { MockProviderAdapter } from '../providers/mock-provider.mjs';
import { CodexCliAdapter } from '../providers/codex-cli-adapter.mjs';
import { ClaudeCliAdapter } from '../providers/claude-cli-adapter.mjs';
import { OllamaAdapter } from '../providers/ollama-adapter.mjs';

const projectRoot = path.resolve(process.env.BATAI_PROJECT_ROOT ?? process.cwd());
const adapters = new Map([
  ['mock', new MockProviderAdapter()],
  ['codex-cli', new CodexCliAdapter({ projectRoot })],
  ['claude-cli', new ClaudeCliAdapter({ projectRoot })],
  ['ollama', new OllamaAdapter()]
]);
const runtime = new BataiRuntime({ projectRoot, adapters });
await runtime.start();

const tools = [
  {
    name:'batai_create_agent',
    description:'Create or configure an agent in the Batai organization. Use the minimum sufficient provider/model/reasoning for the task.',
    inputSchema:{type:'object',required:['id','name','role_template','provider','model','reasoning_effort'],properties:{
      id:{type:'string'},name:{type:'string'},role_template:{type:'string'},parent_agent_id:{type:['string','null'],default:'director'},
      provider:{type:'string',enum:['mock','codex-cli','claude-cli','ollama']},model:{type:'string'},
      reasoning_effort:{type:'string',enum:['none','low','medium','high','extra_high']},auth_mode:{type:'string'},
      allowed_paths:{type:'array',items:{type:'string'}},tools:{type:'array',items:{type:'string'}},constraints:{type:'array',items:{type:'string'}},
      max_turns:{type:'integer',minimum:1},lifetime:{type:'string',enum:['task_scoped','project','permanent']}
    }}
  },
  {
    name:'batai_assign_task',
    description:'Create a declarative Batai task JSON. The file watcher will dispatch it automatically when dependencies are satisfied.',
    inputSchema:{type:'object',required:['id','objective','assigned_to'],properties:{
      id:{type:'string'},objective:{type:'string'},assigned_to:{type:'array',items:{type:'string'},minItems:1},
      dependencies:{type:'array',items:{type:'string'}},acceptance_criteria:{type:'array',items:{type:'string'}},
      inputs:{type:'array',items:{type:'string'}},outputs:{type:'array',items:{type:'string'}},
      requires_director_review:{type:'boolean'},notify_on_success:{type:['string','null']},start_on_success:{type:'array',items:{type:'string'}}
    }}
  },
  { name:'batai_read_project_state', description:'Read agents, tasks, resource states and recent structured events.', inputSchema:{type:'object',properties:{}} },
  { name:'batai_approve_task', description:'Approve a task currently waiting at the Director REVIEW gate.', inputSchema:{type:'object',required:['task_id'],properties:{task_id:{type:'string'}}} },
  { name:'batai_create_worktree', description:'Create an isolated Git worktree and branch for an agent/task.', inputSchema:{type:'object',required:['agent_id','task_id'],properties:{agent_id:{type:'string'},task_id:{type:'string'},base_ref:{type:'string',default:'HEAD'}}} },
  { name:'batai_update_directives', description:'Replace an agent DIRECTIVES.md. Batai emits DIRECTIVE_UPDATED and notifies the target session layer.', inputSchema:{type:'object',required:['agent_id','content'],properties:{agent_id:{type:'string'},content:{type:'string'}}} },
  { name:'batai_resume_agent', description:'Resume tasks parked for a resource-limited agent after its provider becomes available.', inputSchema:{type:'object',required:['agent_id'],properties:{agent_id:{type:'string'}}} },
  { name:'batai_create_github_issue', description:'Create a GitHub issue through authenticated gh CLI when available.', inputSchema:{type:'object',required:['title'],properties:{title:{type:'string'},body:{type:'string'},labels:{type:'array',items:{type:'string'}}}} },
  { name:'batai_read_director_inbox', description:'Read pending GOD messages sent to the Director.', inputSchema:{type:'object',properties:{pending_only:{type:'boolean',default:true}}} },
  { name:'batai_acknowledge_god_message', description:'Acknowledge a GOD message after incorporating it into project planning.', inputSchema:{type:'object',required:['message_id'],properties:{message_id:{type:'string'}}} },
  { name:'batai_request_god_decision', description:'Create a structured decision request for GOD with recommendation, alternatives and impact.', inputSchema:{type:'object',required:['topic','question'],properties:{id:{type:'string'},topic:{type:'string'},question:{type:'string'},director_recommendation:{},alternatives:{type:'array'},impact:{type:'string',enum:['low','medium','high','critical']}}} }
];

function rpcResult(id, result) { process.stdout.write(`${JSON.stringify({jsonrpc:'2.0',id,result})}\n`); }
function rpcError(id, code, message, data = undefined) { process.stdout.write(`${JSON.stringify({jsonrpc:'2.0',id,error:{code,message,...(data===undefined?{}:{data})}})}\n`); }
function textResult(value) { return { content:[{type:'text',text:JSON.stringify(value,null,2)}], isError:false }; }

async function callTool(name, args = {}) {
  if (name === 'batai_create_agent') {
    return runtime.directorTools.createAgent({
      id:args.id,name:args.name,role_template:args.role_template,parent_agent_id:args.parent_agent_id ?? 'director',provider:args.provider,model:args.model,
      reasoning_effort:args.reasoning_effort,auth_mode:args.auth_mode ?? (args.provider==='ollama'?'local':'subscription'),worktree:null,
      allowed_paths:args.allowed_paths ?? ['**/*'],tools:args.tools ?? ['git','shell','test_runner'],constraints:args.constraints ?? ['Do not contact customer'],
      max_turns:args.max_turns ?? 18,lifetime:args.lifetime ?? 'project',status:'READY'
    });
  }
  if (name === 'batai_assign_task') {
    const task = {
      id:args.id,created_by:'director',objective:args.objective,assigned_to:args.assigned_to,dependencies:args.dependencies ?? [],
      acceptance_criteria:args.acceptance_criteria ?? ['Complete the objective and run relevant tests'],inputs:args.inputs ?? [],outputs:args.outputs ?? [],status:'READY',
      execution:{parallel:(args.assigned_to?.length ?? 0)>1,requires_director_review:args.requires_director_review ?? false},
      on_success:{...(args.notify_on_success?{notify:args.notify_on_success}:{}),...(args.start_on_success?.length?{start:args.start_on_success}:{})},
      on_failure:{notify:'director'}
    };
    return runtime.directorTools.assignTask(task);
  }
  if (name === 'batai_read_project_state') return runtime.state();
  if (name === 'batai_approve_task') return runtime.directorTools.approveTask(args.task_id);
  if (name === 'batai_create_worktree') { const wt = await runtime.git.create({agentId:args.agent_id,taskId:args.task_id,baseRef:args.base_ref ?? 'HEAD'}); runtime.directorTools.bindWorktree(args.agent_id,wt.directory); return {...wt,agent_id:args.agent_id,bound:true}; }
  if (name === 'batai_update_directives') {
    const file = path.join(projectRoot,'.batai','agents',args.agent_id,'DIRECTIVES.md');
    fs.mkdirSync(path.dirname(file),{recursive:true}); fs.writeFileSync(file,args.content.endsWith('\n')?args.content:`${args.content}\n`); return {agent_id:args.agent_id,path:file};
  }
  if (name === 'batai_resume_agent') { await runtime.taskEngine.resumeAgentTasks(args.agent_id); return {agent_id:args.agent_id,status:'resume_requested'}; }
  if (name === 'batai_create_github_issue') return runtime.github.createIssue({title:args.title,body:args.body ?? '',labels:args.labels ?? []});
  if (name === 'batai_read_director_inbox') return runtime.authority.listInbox('director',{pendingOnly:args.pending_only ?? true});
  if (name === 'batai_acknowledge_god_message') return runtime.authority.acknowledgeMessage(args.message_id);
  if (name === 'batai_request_god_decision') return runtime.authority.createDecision({id:args.id,topic:args.topic,question:args.question,director_recommendation:args.director_recommendation ?? null,alternatives:args.alternatives ?? [],impact:args.impact ?? 'medium',requires:'GOD'});
  throw new Error(`Unknown tool: ${name}`);
}

async function handle(message) {
  const { id, method, params } = message;
  if (method === 'notifications/initialized') return;
  if (method === 'initialize') return rpcResult(id, {
    protocolVersion: params?.protocolVersion ?? '2025-11-25',
    capabilities:{tools:{listChanged:false}},
    serverInfo:{name:'batai-control',version:'0.1.0'},
    instructions:'Use Batai tools to manage agent organization, tasks, worktrees and deterministic orchestration. Prefer the minimum sufficient model/reasoning.'
  });
  if (method === 'ping') return rpcResult(id, {});
  if (method === 'tools/list') return rpcResult(id, {tools});
  if (method === 'tools/call') {
    try { return rpcResult(id, textResult(await callTool(params?.name, params?.arguments ?? {}))); }
    catch (error) { return rpcResult(id, {content:[{type:'text',text:`${error.name}: ${error.message}`}],isError:true}); }
  }
  if (id !== undefined) rpcError(id,-32601,`Method not found: ${method}`);
}

const rl = readline.createInterface({ input:process.stdin, crlfDelay:Infinity });
rl.on('line', async line => {
  if (!line.trim()) return;
  try { await handle(JSON.parse(line)); }
  catch (error) { rpcError(null,-32700,'Parse error',error.message); }
});
rl.on('close', async () => { await runtime.close(); process.exit(0); });
