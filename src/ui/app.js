import {buildGraph, preservedSelection, searchableText, statusGroup} from './organization-graph.js';
import {
  FUNCTIONS, SENIORITIES, conflictMessage, createMutationRequest, openCount,
  relationshipIsEditable, validFunctionsForSeniority
} from './governance-ui.js';

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const invoke = window.__TAURI__?.core?.invoke;
const esc = value => String(value ?? '').replace(/[&<>'"]/g, character => ({'&':'&amp;','<':'&lt;','>':'&gt;',"'":'&#39;','"':'&quot;'}[character]));
const label = value => String(value ?? 'Unknown').toLowerCase().replaceAll('_', ' ').replace(/\b\w/g, character => character.toUpperCase());
const valueOrDash = value => value === null || value === undefined || value === '' ? '—' : value;
const number = value => value === null || value === undefined ? '—' : Number(value).toLocaleString();
const percent = value => value === null || value === undefined ? '—' : `${Math.round(value * 10) / 10}%`;
const departmentLabel = value => value === 'DATA_AI' || value === 'DataAi' ? 'Data & AI' : label(value);

let snapshot = {god:{id:'god',label:'User'},project:{name:'Batai',progress:0,usage:{}},agents:[],tasks:[],relationships:[],resources:[],activity:[],hierarchyWarnings:[],governance:{revision:0,policy:{limits:{maxActiveAgents:8,maxHierarchyDepth:3}},decisions:[],providerApprovals:[],audit:[],recoveryOperations:[],recoveryRequiresReview:0}};
let providers = [];
let refreshTimer;
let inspectorTab = 'overview';
let organizationEditMode = false;
let confirmOperation = null;
const graphState = {
  mode:'hierarchy', scale:1, tx:0, ty:20, selectedId:null, query:'', fitted:false,
  filters:{departments:[],seniorities:[],statuses:[],providers:[]}
};

async function loadSnapshot() {
  if (invoke) return invoke('get_app_snapshot');
  const response = await fetch('/api/state');
  if (!response.ok) throw new Error('Control plane is offline');
  const state = await response.json();
  const tasks = (state.tasks ?? []).map(task => ({
    id:task.id, objective:task.objective, status:task.status,
    assignedTo:task.assigned_to ?? [], dependencies:task.dependencies ?? [], weight:task.weight ?? 1,
    progress:task.progress ?? ({COMPLETED:1,REVIEW:.9,RUNNING:.55,WAITING_RESOURCE:.4,BLOCKED:.25}[task.status] ?? 0)
  }));
  const agents = (state.agents ?? []).map(agent => ({
    id:agent.id, name:agent.name, title:agent.title ?? agent.role_template,
    department:agent.department ? departmentLabel(agent.department) : 'Unspecified', seniority:agent.seniority ?? null,
    level:agent.level ?? null, function:agent.function ?? 'GENERIC_SOFTWARE_AGENT',
    reportsTo:agent.reports_to ?? agent.parent_agent_id, directReports:agent.direct_reports ?? [],
    provider:agent.provider, model:agent.model, reasoningEffort:agent.reasoning_effort,
    authMode:agent.auth_mode, status:agent.status, activity:agent.activity ?? 'IDLE',
    currentTaskId:agent.current_task_id, currentTaskObjective:agent.current_task_objective,
    worktree:agent.worktree, performance:agent.performance ?? {}, usage:agent.usage ?? {}
    ,lifecycle:agent.lifecycle ?? 'TASK_SCOPED', authority:agent.authority ?? 'WORKER',
    permissions:agent.permissions ?? [], intelligencePolicy:agent.intelligence_policy ?? {}, history:agent.history ?? []
  }));
  const total = tasks.reduce((sum, task) => sum + task.weight, 0);
  const progress = total ? Math.round(tasks.reduce((sum, task) => sum + task.weight * task.progress, 0) / total * 1000) / 10 : 0;
  return {...snapshot, agents, tasks, project:{...snapshot.project,name:'batai',progress,
    activeAgents:agents.filter(agent => agent.status === 'RUNNING').length,
    waitingAgents:agents.filter(agent => ['WAITING_RESOURCE','BLOCKED'].includes(agent.status)).length,
    completedTasks:tasks.filter(task => task.status === 'COMPLETED').length,
    runningTasks:tasks.filter(task => task.status === 'RUNNING').length,
    blockedTasks:tasks.filter(task => task.status === 'BLOCKED').length,
    remainingTasks:tasks.filter(task => !['COMPLETED','CANCELLED'].includes(task.status)).length}};
}

async function loadProviders() {
  if (invoke) return invoke('get_provider_connections');
  const response = await fetch('/api/providers');
  if (!response.ok) return [];
  return (await response.json()).map(item => ({
    id:item.name === 'codex-cli' ? 'codex' : item.name === 'claude-cli' ? 'claude' : item.name,
    name:{'codex-cli':'Codex','claude-cli':'Claude Code',ollama:'Ollama',mock:'Mock Runtime'}[item.name] ?? item.name,
    kind:item.name === 'ollama' ? 'Local runtime' : item.name === 'mock' ? 'Development' : 'Subscription',
    status:item.usage?.status === 'AVAILABLE' ? 'connected' : 'unavailable', statusLabel:label(item.usage?.status ?? 'Unknown'),
    accountLabel:item.usage?.status === 'AVAILABLE' ? 'Available on this computer' : 'Setup required',
    detail:(item.models ?? []).join(', ') || 'Models detected dynamically', actionLabel:'Connection guide'
  }));
}

function initials(agent) {
  return (agent.name || agent.title || '?').split(/\s+/).slice(0, 2).map(word => word[0]).join('').toUpperCase();
}

function activityIcon(activity) {
  return ({CODING:'</>',READING:'R',TESTING:'✓',REVIEWING:'◇',BLOCKED:'!',WAITING:'…',RATE_LIMITED:'⏱',PROCESSING:'⋯',WORKING:'⋯',IDLE:'○'}[activity] ?? '⋯');
}

function agentNode(agent) {
  const kind = agent.function === 'DIRECTOR' || agent.id === 'director' ? ' director' : agent.department === 'Quality' ? ' quality' : '';
  return `<article class="agent-node${kind}" data-agent-id="${esc(agent.id)}" tabindex="0"><div class="node-head"><div class="avatar${kind}">${esc(initials(agent))}</div><div><strong>${esc(agent.name)}</strong><small>${esc(agent.title)}</small></div></div><footer><span class="status status-${esc(agent.status.toLowerCase())}"><i></i>${esc(label(agent.activity))}</span><small class="model-badge">${esc(agent.model || 'Auto')}</small></footer></article>`;
}

function renderOverview() {
  $('#project-name').textContent = snapshot.project.name;
  $('#crumb-project').textContent = snapshot.project.name;
  $('#metric-progress').textContent = `${snapshot.project.progress}%`;
  $('#progress-bar').style.width = `${snapshot.project.progress}%`;
  $('#metric-agents').textContent = snapshot.project.activeAgents ?? 0;
  $('#agent-summary').textContent = `${snapshot.agents.length} AI employees · ${snapshot.project.waitingAgents ?? 0} waiting`;
  const moving = snapshot.tasks.filter(task => ['RUNNING','REVIEW','WAITING_RESOURCE','BLOCKED'].includes(task.status)).length;
  $('#metric-tasks').textContent = moving;
  $('#task-summary').textContent = snapshot.project.blockedTasks ? `${snapshot.project.blockedTasks} blocked` : 'No blockers';
  $('#task-nav-count').textContent = snapshot.tasks.length;
  const connected = providers.filter(provider => ['connected','local'].includes(provider.status)).length;
  $('#metric-providers').textContent = `${connected}/${providers.length || '—'}`;
  $('#provider-summary').textContent = connected ? 'Official sessions detected' : 'Connect an intelligence source';
  const director = snapshot.agents.find(agent => agent.function === 'DIRECTOR' || agent.id === 'director');
  const workers = snapshot.agents.filter(agent => agent !== director).slice(0, 4);
  $('#organization-preview').innerHTML = snapshot.agents.length ? `${director ? agentNode(director) : ''}${workers.map(agentNode).join('')}` : '<p class="empty">Create your first project team.</p>';
  $('#task-list').innerHTML = snapshot.tasks.slice(0, 5).map(task => `<div class="task-row"><i></i><div><strong>${esc(task.objective)}</strong><small>${esc(task.assignedTo.join(', ') || 'Unassigned')}</small></div><b>${esc(label(task.status))}</b></div>`).join('') || '<p class="empty">No tasks yet. Ask Director to plan the first milestone.</p>';
  $('#provider-mini-list').innerHTML = providers.slice(0, 4).map(provider => `<div class="provider-mini"><div><strong>${esc(provider.name)}</strong><small>${esc(provider.kind)}</small></div><span class="connection-dot ${esc(provider.status)}">${esc(provider.statusLabel)}</span></div>`).join('');
  bindAgentCards();
}

function renderTasks() {
  const columns = [['Backlog',['PENDING','READY']],['In progress',['RUNNING','WAITING_RESOURCE','BLOCKED']],['Review',['REVIEW']],['Completed',['COMPLETED']]];
  $('#task-board').innerHTML = columns.map(([name, statuses]) => {
    const tasks = snapshot.tasks.filter(task => statuses.includes(task.status));
    return `<section class="task-column"><header><span>${name}</span><b>${tasks.length}</b></header>${tasks.map(task => `<article class="task-card" data-task-id="${esc(task.id)}"><strong>${esc(task.id)}</strong><p>${esc(task.objective)}</p><footer><span>${esc(label(task.status))}</span><span>${esc(task.assignedTo[0] ?? 'Unassigned')}</span></footer></article>`).join('')}</section>`;
  }).join('');
}

function renderProviders() {
  const logos = {codex:'◎',claude:'A',gh:'⌘',ollama:'◉',mock:'M'};
  $('#provider-grid').innerHTML = providers.map(provider => `<article class="provider-card"><div class="provider-head"><div class="provider-logo">${esc(logos[provider.id] ?? provider.name[0])}</div><div><strong>${esc(provider.name)}</strong><span>${esc(provider.kind)}</span></div><span class="provider-status ${esc(provider.status)}">${esc(provider.statusLabel)}</span></div><div class="provider-detail"><span>Account</span><strong>${esc(provider.accountLabel)}</strong></div><footer><small>${esc(provider.detail)}</small><button data-provider="${esc(provider.id)}">${esc(provider.actionLabel)}</button></footer></article>`).join('') || '<p class="empty">No providers configured.</p>';
  $$('[data-provider]').forEach(button => button.addEventListener('click', () => showGuide(button.dataset.provider)));
}

function renderResources() {
  const connectionById = new Map(providers.map(provider => [provider.id, provider]));
  const resources = snapshot.resources?.length ? snapshot.resources : providers.map(provider => ({
    provider:provider.id, status:provider.status === 'connected' ? 'AVAILABLE' : 'UNKNOWN',
    usageSources:[], activeAgents:snapshot.agents.filter(agent => agent.provider === provider.id && agent.status === 'RUNNING').length,
    usedPercent:null, resetAt:null, usage:{}
  }));
  $('#resource-summary').innerHTML = resources.map(resource => {
    const connection = connectionById.get(resource.provider);
    const cost = resource.usage?.cost == null ? 'Unknown' : `${resource.usage.currency ?? ''} ${resource.usage.cost.toFixed(4)}`.trim();
    return `<article class="resource-card"><header><div class="provider-logo">${esc((connection?.name ?? resource.provider)[0].toUpperCase())}</div><div><strong>${esc(connection?.name ?? label(resource.provider))}</strong><span>${esc(connection?.kind ?? 'Configured provider')}</span></div><b class="resource-health health-${esc(resource.status.toLowerCase())}">${esc(label(resource.status))}</b></header><dl><div><dt>Usage source</dt><dd>${esc((resource.usageSources ?? []).map(label).join(', ') || 'Unknown')}</dd></div><div><dt>Active agents</dt><dd>${number(resource.activeAgents)}</dd></div><div><dt>Known tokens</dt><dd>${number(resource.usage?.totalTokens)}</dd></div><div><dt>Known cost</dt><dd>${esc(cost)}</dd></div><div><dt>Quota used</dt><dd>${percent(resource.usedPercent)}</dd></div><div><dt>Reset</dt><dd>${esc(valueOrDash(resource.resetAt))}</dd></div></dl></article>`;
  }).join('') || '<p class="empty">No runtime resource state has been reported.</p>';
}

function renderGovernance() {
  const governance = snapshot.governance ?? {};
  const decisions = governance.decisions ?? [];
  const approvals = governance.providerApprovals ?? [];
  const recovery = governance.recoveryOperations ?? [];
  const open = openCount(decisions);
  $('#organization-revision').textContent = `rev ${governance.revision ?? 0}`;
  $('#decision-count').textContent = `${open} open`;
  $('#decision-count').classList.toggle('has-open', open > 0);
  const navDecision = $('.nav-item[data-view="decisions"]');
  navDecision?.classList.toggle('attention', open + approvals.filter(item => item.status === 'PENDING_GOD').length + (governance.recoveryRequiresReview ?? 0) > 0);
  $('#decision-list').innerHTML = decisions.map(decision => `<article class="governance-record"><header><strong>${esc(decision.question)}</strong><span class="record-status ${esc(decision.status.toLowerCase())}">${esc(label(decision.status))}</span></header><p>${esc(decision.impact)}</p><small>${esc(decision.requestedBy)} · ${esc(new Date(decision.createdAt).toLocaleString())}</small>${decision.status === 'OPEN' ? `<div class="record-actions"><button class="approve" data-decision="${esc(decision.id)}" data-resolution="approve">Approve exact change</button><button class="reject" data-decision="${esc(decision.id)}" data-resolution="reject">Reject</button></div>` : ''}</article>`).join('') || '<p class="empty">No organizational decisions.</p>';
  $('#approval-list').innerHTML = approvals.map(approval => { const remaining = approval.expiresAt ? Math.max(0, Math.ceil((new Date(approval.expiresAt) - Date.now()) / 1000)) : null; return `<article class="governance-record"><header><strong>${esc(approval.operation)}</strong><span class="record-status ${esc(approval.status.toLowerCase())}">${esc(label(approval.status))}</span></header><p>${esc(label(approval.provider))} · ${esc(approval.agentId ?? 'Unknown agent')} · ${esc(approval.taskId ?? 'No task')}</p><dl class="record-details"><div><dt>Target</dt><dd>${esc(valueOrDash(approval.requestedTarget))}</dd></div><div><dt>Risk</dt><dd>${esc(label(approval.risk ?? 'Unknown'))}</dd></div><div><dt>Thread / turn</dt><dd>${esc(`${approval.threadId ?? '—'} / ${approval.turnId ?? '—'}`)}</dd></div></dl><small>${esc(new Date(approval.createdAt).toLocaleString())}${remaining == null ? '' : ` · expires in ${remaining}s`}</small>${approval.status === 'PENDING_GOD' ? `<div class="record-actions"><button class="approve" data-approval="${esc(approval.id)}" data-resolution="approve">Allow once</button><button class="reject" data-approval="${esc(approval.id)}" data-resolution="reject">Deny</button></div>` : ''}</article>`; }).join('') || '<p class="empty">No provider approval requests.</p>';
  $('#recovery-count').textContent = `${governance.recoveryRequiresReview ?? 0} review`;
  $('#recovery-count').classList.toggle('has-open', (governance.recoveryRequiresReview ?? 0) > 0);
  $('#recovery-list').innerHTML = recovery.map(operation => `<article class="governance-record"><header><strong>${esc(label(operation.operationType))}</strong><span class="record-status ${esc(operation.currentPhase.toLowerCase())}">${esc(label(operation.currentPhase))}</span></header><p>${esc(operation.actor)} · ${esc(valueOrDash(operation.target))}</p><dl class="record-details"><div><dt>Files</dt><dd>${operation.affectedFiles?.length ?? 0}</dd></div><div><dt>Detected state</dt><dd>${esc(label(operation.currentPhase))}</dd></div><div><dt>Recommendation</dt><dd>${esc(label(operation.recoveryDisposition))}</dd></div></dl>${operation.failureDetails ? `<small>${esc(operation.failureDetails)}</small>` : ''}${operation.currentPhase === 'NEEDS_REVIEW' ? `<div class="record-actions"><button data-recovery="${esc(operation.operationId)}" data-recovery-action="RETRY_COMPLETE">Retry / complete</button><button data-recovery="${esc(operation.operationId)}" data-recovery-action="ACCEPT_CURRENT_STATE">Accept current state</button><button class="reject" data-recovery="${esc(operation.operationId)}" data-recovery-action="ROLLBACK">Rollback if safe</button></div>` : ''}</article>`).join('') || '<p class="empty">No recovery operations.</p>';
  $$('[data-decision]').forEach(button => button.addEventListener('click', () => resolveDecision(button.dataset.decision, button.dataset.resolution === 'approve')));
  $$('[data-approval]').forEach(button => button.addEventListener('click', () => resolveApproval(button.dataset.approval, button.dataset.resolution === 'approve')));
  $$('[data-recovery]').forEach(button => button.addEventListener('click', () => resolveRecovery(button.dataset.recovery, button.dataset.recoveryAction)));
  renderPolicy();
}

function renderPolicy() {
  const policy = snapshot.governance?.policy ?? {};
  const form = $('#policy-form');
  form.elements.maxActiveAgents.value = policy.limits?.maxActiveAgents ?? 8;
  form.elements.maxHierarchyDepth.value = policy.limits?.maxHierarchyDepth ?? 3;
  form.elements.providerApprovalTimeoutSeconds.value = policy.providerApprovalTimeoutSeconds ?? 300;
  form.elements.allowPayg.checked = Boolean(policy.allowPayg);
  form.elements.autoAgentCreation.checked = Boolean(policy.autoAgentCreation);
  form.elements.allowedProviders.value = (policy.allowedProviders ?? []).join(', ');
  form.elements.deniedProviders.value = (policy.deniedProviders ?? []).join(', ');
}

async function refreshSnapshot() {
  snapshot = await loadSnapshot();
  renderOverview(); renderTasks(); renderResources(); renderFilters(); renderGovernance(); renderGraph(true);
  if (graphState.selectedId) {
    const agent = snapshot.agents.find(item => item.id === graphState.selectedId);
    if (agent) renderInspector(agent);
  }
}

async function sendMutation(mutation, reason = null) {
  if (!invoke) throw new Error('Organization editing requires the desktop runtime');
  try {
    const result = await invoke('mutate_organization', {request:createMutationRequest(snapshot.governance?.revision ?? 0, mutation, reason)});
    await refreshSnapshot();
    return result;
  } catch (error) {
    await refreshSnapshot().catch(() => {});
    throw new Error(conflictMessage(error));
  }
}

async function resolveDecision(decisionId, approve) {
  if (!invoke) return;
  await invoke('resolve_god_decision', {decisionId, approve, note:null});
  await refreshSnapshot();
}

async function resolveApproval(approvalId, approve) {
  if (!invoke) return;
  await invoke('resolve_provider_approval', {approvalId, approve});
  await refreshSnapshot();
}

async function resolveRecovery(operationId, action) {
  if (!invoke) return;
  await invoke('resolve_recovery_operation', {operationId, action});
  await refreshSnapshot();
}

function renderFilters() {
  const specs = [
    ['department-filters','departments',['Product','Engineering','Architecture','Data & AI','Quality','Operations','Security','Research']],
    ['seniority-filters','seniorities',['DIRECTOR','PRINCIPAL','STAFF','SENIOR','MID','ASSOCIATE','JUNIOR','INTERN']],
    ['status-filters','statuses',['Working','Ready','Waiting','Blocked','Offline']],
    ['provider-filters','providers',[...new Set(snapshot.agents.map(agent => agent.provider).filter(Boolean))]]
  ];
  for (const [elementId, key, values] of specs) {
    const element = $(`#${elementId}`);
    element.innerHTML = values.map(value => `<label><input type="checkbox" data-filter="${key}" value="${esc(value)}" ${graphState.filters[key].includes(value) ? 'checked' : ''}><span>${esc(label(value))}</span><small>${snapshot.agents.filter(agent => key === 'departments' ? agent.department === value : key === 'seniorities' ? agent.seniority === value : key === 'providers' ? agent.provider === value : statusGroup(agent.status) === value).length}</small></label>`).join('');
  }
  $$('[data-filter]').forEach(input => input.addEventListener('change', () => {
    graphState.filters[input.dataset.filter] = $$(`[data-filter="${input.dataset.filter}"]:checked`).map(item => item.value);
    renderGraph(true);
  }));
}

function graphNodeHtml(node) {
  if (node.type === 'god') return `<div class="graph-card god-card" role="button" tabindex="0"><div class="god-symbol">G</div><div><strong>GOD</strong><span>${esc(node.data.label ?? 'User')}</span><small>Highest human authority</small></div></div>`;
  if (node.type === 'task') return `<div class="graph-card task-graph-card status-${esc(node.data.status.toLowerCase())}" role="button" tabindex="0"><div class="task-node-head"><b>${esc(node.data.id)}</b><span>${esc(label(node.data.status))}</span></div><strong>${esc(node.data.objective)}</strong><small>${esc((node.data.assignedTo ?? []).join(', ') || 'Unassigned')}</small></div>`;
  const agent = node.data;
  return `<div class="graph-card agent-graph-card ${agent.function === 'DIRECTOR' ? 'director-card' : ''} status-${esc(agent.status.toLowerCase())}" role="button" tabindex="0"><div class="graph-agent-head"><div class="avatar ${agent.function === 'DIRECTOR' ? 'director' : ''}">${esc(initials(agent))}</div><div><strong>${esc(agent.name)}</strong><span>${esc(agent.title)}</span></div><b>${esc(agent.seniority ? `L${agent.level}` : '—')}</b></div><div class="graph-medium"><span class="activity-chip">${esc(activityIcon(agent.activity))} ${esc(label(agent.activity))}</span><small>${esc(agent.currentTaskId ?? 'No active task')}</small><em>${esc(agent.model || 'Auto')}</em></div><div class="graph-near"><span>${esc(agent.provider)} · ${esc(agent.authMode)}</span><span>${number(agent.usage?.totalTokens)} tokens</span></div></div>`;
}

function renderGraph(preserveViewport = true) {
  const graph = buildGraph(snapshot, graphState.mode, graphState.filters);
  graphState.selectedId = preservedSelection(graph, graphState.selectedId);
  const nodesElement = $('#graph-nodes');
  const edgesElement = $('#graph-edges');
  nodesElement.replaceChildren();
  edgesElement.replaceChildren();
  const sizes = new Map(graph.nodes.map(node => [node.id, node.type === 'god' ? [190,78] : node.type === 'task' ? [240,98] : [230,112]]));
  const byId = new Map(graph.nodes.map(node => [node.id, node]));
  const svgNs = 'http://www.w3.org/2000/svg';
  for (const edge of graph.edges) {
    const source = byId.get(edge.source);
    const target = byId.get(edge.target);
    if (!source || !target) continue;
    const [sourceWidth, sourceHeight] = sizes.get(source.id);
    const [targetWidth] = sizes.get(target.id);
    const x1 = source.x + sourceWidth / 2;
    const y1 = source.y + sourceHeight;
    const x2 = target.x + targetWidth / 2;
    const y2 = target.y;
    const middle = (y1 + y2) / 2;
    const path = document.createElementNS(svgNs, 'path');
    path.setAttribute('d', `M${x1},${y1} C${x1},${middle} ${x2},${middle} ${x2},${y2}`);
    path.setAttribute('class', `graph-edge edge-${edge.type.toLowerCase()}`);
    path.setAttribute('marker-end', edge.type === 'REPORTING' ? 'url(#arrow-reporting)' : 'url(#arrow-workflow)');
    path.dataset.edgeId = edge.id;
    path.addEventListener('click', event => { event.stopPropagation(); renderRelationshipInspector(edge); });
    edgesElement.append(path);
    const text = document.createElementNS(svgNs, 'text');
    text.setAttribute('x', String((x1 + x2) / 2 + 6));
    text.setAttribute('y', String(middle - 5));
    text.setAttribute('class', `edge-label edge-${edge.type.toLowerCase()}`);
    text.textContent = edge.label ?? label(edge.type);
    edgesElement.append(text);
  }
  for (const node of graph.nodes) {
    const [width, height] = sizes.get(node.id);
    const foreign = document.createElementNS(svgNs, 'foreignObject');
    foreign.setAttribute('x', node.x);
    foreign.setAttribute('y', node.y);
    foreign.setAttribute('width', width);
    foreign.setAttribute('height', height);
    foreign.dataset.nodeId = node.id;
    foreign.classList.toggle('selected', graphState.selectedId === node.id);
    foreign.classList.toggle('search-hit', graphState.query && searchableText(node).includes(graphState.query));
    foreign.innerHTML = graphNodeHtml(node);
    const activate = () => selectGraphNode(node);
    foreign.addEventListener('click', activate);
    foreign.addEventListener('keydown', event => { if (event.key === 'Enter' || event.key === ' ') activate(); });
    nodesElement.append(foreign);
  }
  $('#graph-empty').hidden = graph.nodes.length > 0;
  const warnings = graph.warnings ?? [];
  $('#graph-warnings').hidden = !warnings.length;
  $('#graph-warnings').textContent = warnings.length ? `${warnings.length} hierarchy warning${warnings.length === 1 ? '' : 's'}` : '';
  applyTransform();
  if (!preserveViewport || !graphState.fitted) fitGraph(graph);
}

function applyTransform() {
  $('#graph-viewport').setAttribute('transform', `translate(${graphState.tx} ${graphState.ty}) scale(${graphState.scale})`);
  $('#organization-svg').dataset.zoom = graphState.scale < .68 ? 'far' : graphState.scale < 1.12 ? 'medium' : 'near';
  $('#map-zoom-label').textContent = `${Math.round(graphState.scale * 100)}%`;
}

function fitGraph(graph = buildGraph(snapshot, graphState.mode, graphState.filters)) {
  if (!graph.nodes.length) return;
  const area = $('#organization-map').getBoundingClientRect();
  if (area.width < 100 || area.height < 100) {
    graphState.fitted = false;
    return;
  }
  const minX = Math.min(...graph.nodes.map(node => node.x));
  const maxX = Math.max(...graph.nodes.map(node => node.x + (node.type === 'task' ? 240 : 230)));
  const minY = Math.min(...graph.nodes.map(node => node.y));
  const maxY = Math.max(...graph.nodes.map(node => node.y + 112));
  graphState.scale = Math.max(.32, Math.min(1.15, Math.min((area.width - 90) / Math.max(1, maxX - minX), (area.height - 90) / Math.max(1, maxY - minY))));
  graphState.tx = (area.width - (minX + maxX) * graphState.scale) / 2;
  graphState.ty = 42 - minY * graphState.scale;
  graphState.fitted = true;
  applyTransform();
}

function selectGraphNode(node) {
  graphState.selectedId = node.id;
  inspectorTab = 'overview';
  if (node.type === 'agent') renderInspector(node.data);
  else if (node.type === 'task') renderTaskInspector(node.data);
  else renderProjectInspector();
  renderGraph(true);
}

function inspectorMetric(term, description) {
  return `<div><dt>${esc(term)}</dt><dd>${esc(valueOrDash(description))}</dd></div>`;
}

function renderProjectInspector() {
  graphState.selectedId = null;
  $('#inspector-heading').textContent = 'Project Status';
  $('#inspector-tabs').hidden = true;
  const usage = snapshot.project.usage ?? {};
  const bySource = snapshot.project.usageBySource ?? {};
  const knownCost = usage.cost == null ? 'Unknown' : `${usage.currency ?? ''} ${usage.cost.toFixed(4)}`.trim();
  const recoveryReview = snapshot.governance?.recoveryRequiresReview ?? 0;
  $('#inspector-content').innerHTML = `<div class="project-progress-ring"><strong>${snapshot.project.progress}%</strong><span>Weighted progress</span></div>${recoveryReview ? `<button class="recovery-health" data-open-recovery>⚠ ${recoveryReview} operation requires review</button>` : ''}<section class="inspector-section"><h3>Tasks</h3><dl>${inspectorMetric('Completed',snapshot.project.completedTasks)}${inspectorMetric('Running',snapshot.project.runningTasks)}${inspectorMetric('Blocked',snapshot.project.blockedTasks)}${inspectorMetric('Remaining',snapshot.project.remainingTasks)}</dl></section><section class="inspector-section"><h3>Agents</h3><dl>${inspectorMetric('Active',snapshot.project.activeAgents)}${inspectorMetric('Waiting',snapshot.project.waitingAgents)}${inspectorMetric('Organization size',snapshot.agents.length)}</dl></section><section class="inspector-section"><h3>Inference</h3><dl>${inspectorMetric('Local',number(bySource.local?.totalTokens))}${inspectorMetric('Subscription',number(bySource.subscription?.totalTokens))}${inspectorMetric('PAYG / API',number(bySource.api?.totalTokens))}${inspectorMetric('Known tokens',number(usage.totalTokens))}${inspectorMetric('Known API spend',knownCost)}</dl></section><p class="metric-note">Unknown provider values are not estimated.</p>`;
  $('[data-open-recovery]')?.addEventListener('click', () => switchView('decisions'));
}

function renderInspector(agent) {
  $('#inspector-heading').textContent = agent.name;
  $('#inspector-tabs').hidden = false;
  $$('#inspector-tabs button').forEach(button => button.classList.toggle('active', button.dataset.tab === inspectorTab));
  const tasks = snapshot.tasks.filter(task => task.assignedTo.includes(agent.id));
  if (inspectorTab === 'tasks') {
    const groups = [['Current',['RUNNING','WAITING_RESOURCE','BLOCKED','REVIEW']],['Queued',['PENDING','READY']],['Completed',['COMPLETED']],['Failed',['FAILED','CANCELLED']]];
    $('#inspector-content').innerHTML = groups.map(([name,statuses]) => `<section class="inspector-section"><h3>${name}</h3>${tasks.filter(task => statuses.includes(task.status)).map(task => `<button class="inspector-task" data-go-task="${esc(task.id)}"><strong>${esc(task.id)}</strong><span>${esc(task.objective)}</span><small>${esc(label(task.status))}</small></button>`).join('') || '<p class="metric-note">No tasks</p>'}</section>`).join('');
    $$('[data-go-task]').forEach(button => button.addEventListener('click', () => { switchView('tasks'); }));
    return;
  }
  if (inspectorTab === 'activity') {
    const events = agent.history?.length ? agent.history : snapshot.activity.filter(event => event.source === agent.id || event.target === agent.id);
    $('#inspector-content').innerHTML = `<div class="activity-trace">${events.map(event => `<article><i></i><time>${esc(new Date(event.timestamp).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'}))}</time><div><strong>${esc(label(event.eventType))}</strong><span>${esc(event.summary)}</span></div></article>`).join('') || '<p class="metric-note">No runtime events yet.</p>'}</div>`;
    return;
  }
  if (inspectorTab === 'usage') {
    const usage = agent.usage ?? {};
    $('#inspector-content').innerHTML = `<section class="inspector-section"><h3>Token usage</h3><dl>${inspectorMetric('Input',number(usage.inputTokens))}${inspectorMetric('Cached input',number(usage.cachedInputTokens))}${inspectorMetric('Output',number(usage.outputTokens))}${inspectorMetric('Total',number(usage.totalTokens))}</dl></section><section class="inspector-section"><h3>Provider resource</h3><dl>${inspectorMetric('Usage source',(usage.sources ?? []).map(label).join(', ') || 'Unknown')}${inspectorMetric('Reported cost',usage.cost == null ? 'Unknown' : `${usage.currency ?? ''} ${usage.cost}`)}${inspectorMetric('Quota',agent.quotaStatus ? label(agent.quotaStatus) : 'Unknown')}${inspectorMetric('Reset',agent.quotaResetAt)}</dl></section>`;
    return;
  }
  if (inspectorTab === 'performance') {
    const performance = agent.performance ?? {};
    $('#inspector-content').innerHTML = `<section class="inspector-section"><h3>Observed outcomes</h3><dl>${inspectorMetric('Completed',performance.completedTasks)}${inspectorMetric('Active',performance.activeTasks)}${inspectorMetric('Failures',performance.failedTasks)}${inspectorMetric('First-attempt success',percent(performance.firstAttemptSuccessRate))}${inspectorMetric('Final success',percent(performance.finalSuccessRate))}${inspectorMetric('Average attempts',performance.averageAttempts?.toFixed(1))}${inspectorMetric('Review acceptance',percent(performance.reviewAcceptanceRate))}${inspectorMetric('Average duration',performance.averageTaskDurationSeconds == null ? '—' : `${Math.round(performance.averageTaskDurationSeconds)}s`)}</dl></section><p class="metric-note">Only durable task-run history is counted.</p>`;
    return;
  }
  if (inspectorTab === 'context') {
    $('#inspector-content').innerHTML = `<section class="inspector-section"><h3>Permissions / Context</h3><dl>${inspectorMetric('Lifecycle',label(agent.lifecycle))}${inspectorMetric('Authority',label(agent.authority))}${inspectorMetric('Permissions',(agent.permissions ?? []).map(label).join(', ') || 'Least privilege')}${inspectorMetric('Worktree',agent.worktree)}${inspectorMetric('Session',agent.sessionState)}${inspectorMetric('Auth source',label(agent.authMode))}${inspectorMetric('Organizational parent',agent.reportsTo)}${inspectorMetric('Direct reports',agent.directReports?.join(', ') || '—')}</dl></section><section class="inspector-section"><h3>Intelligence policy</h3><dl>${inspectorMetric('Assignment',label(agent.intelligencePolicy?.assignment ?? 'AUTO'))}${inspectorMetric('Preferred providers',(agent.intelligencePolicy?.preferredProviders ?? []).join(', ') || 'Auto')}${inspectorMetric('PAYG',agent.intelligencePolicy?.allowPayg ? 'Allowed' : 'Not allowed')}</dl></section><p class="metric-note">Execution trace includes observable runtime events only. Hidden model reasoning is never stored or shown.</p>`;
    return;
  }
  const actions = organizationEditMode ? `<div class="agent-actions"><button data-agent-edit="identity">Edit identity</button><button data-agent-edit="reporting">Change manager</button><button data-agent-edit="${agent.status === 'PAUSED' ? 'resume' : 'pause'}">${agent.status === 'PAUSED' ? 'Resume' : 'Pause'}</button>${agent.authority !== 'DIRECTOR' && agent.function !== 'DIRECTOR' ? '<button class="danger" data-agent-edit="terminate">Terminate</button>' : ''}</div>` : '';
  $('#inspector-content').innerHTML = `<div class="agent-inspector-hero"><div class="avatar ${agent.function === 'DIRECTOR' ? 'director' : ''}">${esc(initials(agent))}</div><div><strong>${esc(agent.name)}</strong><span>${esc(agent.title)}</span><small>${esc(activityIcon(agent.activity))} ${esc(label(agent.activity))}</small><i class="authority-chip">${esc(label(agent.authority))} · ${esc(label(agent.lifecycle))}</i></div></div>${actions}<section class="inspector-section"><h3>Organizational identity</h3><dl>${inspectorMetric('Seniority',agent.seniority ? `${label(agent.seniority)} · L${agent.level}` : 'Unspecified')}${inspectorMetric('Function',label(agent.function))}${inspectorMetric('Department',agent.department)}${inspectorMetric('Reports to',agent.reportsTo)}${inspectorMetric('Direct reports',agent.directReports?.join(', ') || '—')}</dl></section><section class="inspector-section"><h3>Assigned intelligence</h3><dl>${inspectorMetric('Model',agent.model)}${inspectorMetric('Provider',label(agent.provider))}${inspectorMetric('Reasoning effort',label(agent.reasoningEffort))}${inspectorMetric('Auth / usage',label(agent.authMode))}</dl></section><section class="inspector-section"><h3>Current runtime</h3><dl>${inspectorMetric('Status',label(agent.status))}${inspectorMetric('Activity',label(agent.activity))}${inspectorMetric('Task',agent.currentTaskId)}${inspectorMetric('Worktree',agent.worktree)}${inspectorMetric('Session',agent.sessionState)}</dl></section>`;
  $$('[data-agent-edit]').forEach(button => button.addEventListener('click', () => beginAgentAction(agent, button.dataset.agentEdit)));
}

function renderTaskInspector(task) {
  $('#inspector-heading').textContent = task.id;
  $('#inspector-tabs').hidden = true;
  $('#inspector-content').innerHTML = `<section class="inspector-section"><h3>Task</h3><p class="task-objective">${esc(task.objective)}</p><dl>${inspectorMetric('Status',label(task.status))}${inspectorMetric('Owner',(task.assignedTo ?? []).join(', ') || 'Unassigned')}${inspectorMetric('Dependencies',(task.dependencies ?? []).join(', ') || '—')}${inspectorMetric('Weight',task.weight)}</dl></section><button class="primary-button" id="open-task-board">Open Task Board</button>`;
  $('#open-task-board').addEventListener('click', () => switchView('tasks'));
}

function renderRelationshipInspector(relationship) {
  $('#inspector-heading').textContent = label(relationship.type);
  $('#inspector-tabs').hidden = true;
  const editable = organizationEditMode && relationshipIsEditable(relationship);
  $('#inspector-content').innerHTML = `<section class="inspector-section"><h3>Relationship</h3><dl>${inspectorMetric('Type',label(relationship.type))}${inspectorMetric('From',relationship.source)}${inspectorMetric('To',relationship.target)}${inspectorMetric('Label',relationship.label)}${inspectorMetric('Source',relationship.persistent ? 'Organization configuration' : 'Runtime derived')}</dl></section>${editable ? '<button id="remove-relationship" class="quiet-button">Remove relationship</button>' : '<p class="metric-note">Reporting comes from the agent manager field. Workflow edges are derived and cannot be edited here.</p>'}`;
  if (editable) $('#remove-relationship').addEventListener('click', () => openConfirm('Remove relationship', '<p>This persistent organization link will be removed. Runtime-derived links are unaffected.</p>', () => ({type:'REMOVE_RELATIONSHIP',data:{relationshipId:relationship.id}})));
}

function bindAgentCards() {
  $$('[data-agent-id]').forEach(node => {
    const open = () => {
      const agent = snapshot.agents.find(item => item.id === node.dataset.agentId);
      if (agent) { switchView('organization'); graphState.selectedId = agent.id; renderInspector(agent); renderGraph(true); }
    };
    node.addEventListener('click', open);
    node.addEventListener('keydown', event => { if (event.key === 'Enter') open(); });
  });
}

async function showGuide(providerId) {
  let guide;
  if (invoke) guide = await invoke('get_connection_guide', {providerId});
  else {
    const provider = providers.find(item => item.id === providerId);
    const commands = {codex:'codex login',claude:'claude auth login',gh:'gh auth login',ollama:'ollama serve'};
    guide = {title:`Connect ${provider?.name ?? providerId}`,description:'Use the official provider flow. Batai never stores your raw password or token.',command:commands[providerId],steps:['Open a terminal','Run the official connection command','Complete authentication','Return to Batai and check again']};
  }
  $('#dialog-title').textContent = guide.title;
  $('#dialog-description').textContent = guide.description;
  $('#dialog-steps').innerHTML = guide.steps.map(step => `<li>${esc(step)}</li>`).join('');
  $('#dialog-command').textContent = guide.command ?? 'No command required';
  $('#copy-command').dataset.command = guide.command ?? '';
  $('#connection-dialog').showModal();
}

function openConfirm(title, descriptionHtml, operation) {
  $('#confirm-title').textContent = title;
  $('#confirm-description').innerHTML = descriptionHtml;
  $('#confirm-reason').value = '';
  confirmOperation = operation;
  $('#confirm-dialog').showModal();
}

function beginAgentAction(agent, action) {
  if (action === 'identity') {
    openConfirm('Edit agent identity', `<label>Name<input id="edit-agent-name" maxlength="64" value="${esc(agent.name)}"></label><label>Display title override<input id="edit-agent-title" maxlength="80" value=""></label>`, () => ({type:'UPDATE_IDENTITY',data:{agentId:agent.id,name:$('#edit-agent-name').value,titleOverride:$('#edit-agent-title').value || null}}));
    return;
  }
  if (action === 'reporting') {
    const options = ['god', ...snapshot.agents.filter(item => item.id !== agent.id && item.status !== 'TERMINATED').map(item => item.id)];
    openConfirm('Change reporting line', `<p>${esc(agent.name)} will report to:</p><select id="edit-agent-parent">${options.map(id => `<option value="${esc(id)}" ${agent.reportsTo === id ? 'selected' : ''}>${esc(id === 'god' ? 'GOD / User' : snapshot.agents.find(item => item.id === id)?.name ?? id)}</option>`).join('')}</select>`, () => ({type:'CHANGE_REPORTING_LINE',data:{agentId:agent.id,reportsTo:$('#edit-agent-parent').value}}));
    return;
  }
  if (action === 'terminate') {
    openConfirm('Terminate agent', `<p>This is a soft termination. ${esc(agent.name)} remains in history and audit records.</p>`, reason => ({type:'TERMINATE_AGENT',data:{agentId:agent.id,reason:reason || 'Terminated by GOD'}}));
    return;
  }
  const resume = action === 'resume';
  openConfirm(`${resume ? 'Resume' : 'Pause'} agent`, `<p>${esc(agent.name)} will be ${resume ? 'returned to ready state' : 'paused after current safe boundary'}.</p>`, () => ({type:resume ? 'RESUME_AGENT' : 'PAUSE_AGENT',data:{agentId:agent.id}}));
}

function populateAgentForm() {
  const form = $('#agent-form');
  form.reset();
  form.elements.seniority.innerHTML = SENIORITIES.map(([name, level]) => `<option value="${name}" ${name === 'SENIOR' ? 'selected' : ''}>L${level} · ${label(name)}</option>`).join('');
  updateFunctionOptions();
  form.elements.reportsTo.innerHTML = `<option value="">No manager</option><option value="god">GOD / User</option>${snapshot.agents.filter(agent => agent.status !== 'TERMINATED').map(agent => `<option value="${esc(agent.id)}">${esc(agent.name)} · ${esc(agent.title)}</option>`).join('')}`;
  const director = snapshot.agents.find(agent => agent.function === 'DIRECTOR' || agent.id.toLowerCase() === 'director');
  if (director) form.elements.reportsTo.value = director.id;
  $('#agent-form-error').textContent = '';
}

function updateFunctionOptions() {
  const form = $('#agent-form');
  const previous = form.elements.function.value;
  const options = validFunctionsForSeniority(form.elements.seniority.value);
  form.elements.function.innerHTML = options.map(([id, value]) => `<option value="${id}">${esc(value.title)}</option>`).join('');
  if (options.some(([id]) => id === previous)) form.elements.function.value = previous;
  else if (options.some(([id]) => id === 'BACKEND_ENGINEERING')) form.elements.function.value = 'BACKEND_ENGINEERING';
}

function populateRelationshipForm() {
  const options = snapshot.agents.filter(agent => agent.status !== 'TERMINATED').map(agent => `<option value="${esc(agent.id)}">${esc(agent.name)} · ${esc(agent.title)}</option>`).join('');
  $('#relationship-form').elements.source.innerHTML = options;
  $('#relationship-form').elements.target.innerHTML = options;
  if (snapshot.agents.length > 1) $('#relationship-form').elements.target.selectedIndex = 1;
  $('#relationship-form-error').textContent = '';
}

function switchView(view) {
  $$('.view').forEach(section => section.classList.toggle('active', section.id === `view-${view}`));
  $$('.nav-item').forEach(item => item.classList.toggle('active', item.dataset.view === view));
  $$('.primary-modes button').forEach(button => button.classList.toggle('active', button.dataset.viewLink === view));
  $('#crumb-view').textContent = label(view);
  $('#director-panel').classList.toggle('active', view !== 'organization');
  $('#organization-inspector').classList.toggle('active', view === 'organization');
  if (view === 'organization') {
    renderProjectInspector();
    requestAnimationFrame(() => { renderGraph(true); if (!graphState.fitted) fitGraph(); });
  }
  if (view === 'resources') renderResources();
  if (view === 'decisions' || view === 'settings') renderGovernance();
}

async function sendMessage(content) {
  if (invoke) return invoke('send_director_message', {content});
  const response = await fetch('/api/god/messages', {method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({content,scope:'task',target:'director'})});
  if (!response.ok) throw new Error('Control plane rejected the message');
  return response.json();
}

function initializeGraphControls() {
  $$('#graph-modes [data-mode]').forEach(button => button.addEventListener('click', () => {
    graphState.mode = button.dataset.mode;
    graphState.fitted = false;
    $$('#graph-modes [data-mode]').forEach(item => item.classList.toggle('active', item === button));
    renderGraph(false);
  }));
  $('#map-fit').addEventListener('click', () => fitGraph());
  $('#map-reset').addEventListener('click', () => { graphState.scale = 1; graphState.tx = 40; graphState.ty = 30; graphState.fitted = true; applyTransform(); });
  const zoom = factor => { graphState.scale = Math.max(.28, Math.min(1.8, graphState.scale * factor)); applyTransform(); };
  $('#map-zoom-in').addEventListener('click', () => zoom(1.15));
  $('#map-zoom-out').addEventListener('click', () => zoom(.87));
  $('#clear-filters').addEventListener('click', () => { Object.keys(graphState.filters).forEach(key => graphState.filters[key] = []); renderFilters(); renderGraph(true); });
  $('#map-search').addEventListener('input', event => {
    graphState.query = event.target.value.trim().toLowerCase();
    renderGraph(true);
    if (graphState.query) {
      const graph = buildGraph(snapshot, graphState.mode, graphState.filters);
      const hit = graph.nodes.find(node => searchableText(node).includes(graphState.query));
      if (hit) {
        const area = $('#organization-map').getBoundingClientRect();
        graphState.tx = area.width / 2 - (hit.x + 115) * graphState.scale;
        graphState.ty = area.height / 2 - (hit.y + 55) * graphState.scale;
        applyTransform();
      }
    }
  });
  const map = $('#organization-map');
  let drag = null;
  map.addEventListener('pointerdown', event => {
    if (event.target.closest('.graph-card')) return;
    drag = {x:event.clientX,y:event.clientY,tx:graphState.tx,ty:graphState.ty};
    map.setPointerCapture(event.pointerId);
  });
  map.addEventListener('pointermove', event => {
    if (!drag) return;
    graphState.tx = drag.tx + event.clientX - drag.x;
    graphState.ty = drag.ty + event.clientY - drag.y;
    applyTransform();
  });
  map.addEventListener('pointerup', () => drag = null);
  map.addEventListener('wheel', event => {
    event.preventDefault();
    if (event.ctrlKey || event.metaKey) {
      graphState.scale = Math.max(.28, Math.min(1.8, graphState.scale * Math.exp(-event.deltaY * .008)));
    } else {
      graphState.tx -= event.deltaX;
      graphState.ty -= event.deltaY;
    }
    applyTransform();
  }, {passive:false});
  map.addEventListener('keydown', event => {
    if (event.key === '+' || event.key === '=') zoom(1.15);
    if (event.key === '-') zoom(.87);
    if (event.key.toLowerCase() === 'f') fitGraph();
    if (event.key === '0') { graphState.scale = 1; applyTransform(); }
  });
}

async function boot() {
  try {
    [snapshot, providers] = await Promise.all([loadSnapshot(), loadProviders()]);
    renderOverview(); renderTasks(); renderProviders(); renderResources(); renderFilters(); renderGovernance(); renderGraph(true);
    graphState.fitted = false;
    $('#blocker-count').textContent = snapshot.project.blockedTasks ?? 0;
    const listen = window.__TAURI__?.event?.listen;
    if (listen) await listen('batai://runtime-event', () => {
      clearTimeout(refreshTimer);
      refreshTimer = setTimeout(async () => {
        try {
          snapshot = await loadSnapshot();
          renderOverview(); renderTasks(); renderResources(); renderFilters(); renderGovernance(); renderGraph(true);
          if (graphState.selectedId) {
            const agent = snapshot.agents.find(item => item.id === graphState.selectedId);
            if (agent) renderInspector(agent);
          }
        } catch (_) { /* the next durable event retries */ }
      }, 140);
    });
  } catch (error) {
    $('.runtime-state strong').textContent = 'Runtime offline';
    $('.runtime-state>i').style.background = 'var(--red)';
    $('#organization-preview').innerHTML = `<p class="empty">${esc(error.message)}</p>`;
  }
}

$$('[data-view]').forEach(button => button.addEventListener('click', () => switchView(button.dataset.view)));
$$('[data-view-link]').forEach(button => button.addEventListener('click', () => switchView(button.dataset.viewLink)));
$('#open-accounts').addEventListener('click', () => switchView('accounts'));
$('#refresh-providers').addEventListener('click', async () => { providers = await loadProviders(); renderProviders(); renderOverview(); renderResources(); });
$('#copy-command').addEventListener('click', () => navigator.clipboard.writeText($('#copy-command').dataset.command || ''));
$('#inspector-back').addEventListener('click', renderProjectInspector);
$('#toggle-org-edit').addEventListener('click', event => {
  organizationEditMode = !organizationEditMode;
  event.currentTarget.textContent = organizationEditMode ? 'Done editing' : 'Edit organization';
  $('#create-agent').hidden = !organizationEditMode;
  $('#add-relationship').hidden = !organizationEditMode;
  const agent = snapshot.agents.find(item => item.id === graphState.selectedId);
  if (agent) renderInspector(agent);
});
$('#create-agent').addEventListener('click', () => { populateAgentForm(); $('#agent-dialog').showModal(); });
$('#add-relationship').addEventListener('click', () => { populateRelationshipForm(); $('#relationship-dialog').showModal(); });
$$('[data-close-dialog]').forEach(button => button.addEventListener('click', () => button.closest('dialog').close()));
$('#agent-form').elements.seniority.addEventListener('change', updateFunctionOptions);
$('#agent-form').addEventListener('submit', async event => {
  event.preventDefault();
  const form = event.currentTarget;
  const functionId = form.elements.function.value;
  const functionPolicy = FUNCTIONS[functionId];
  try {
    await sendMutation({type:'CREATE_AGENT',data:{
      id:null,name:form.elements.name.value || null,seniority:form.elements.seniority.value,
      function:functionId,department:functionPolicy.department,reportsTo:form.elements.reportsTo.value || null,
      lifecycle:form.elements.lifecycle.value,authority:'WORKER',provider:form.elements.provider.value,model:form.elements.model.value,
      reasoningEffort:'medium',permissions:['READ_WORKSPACE','WRITE_WORKSPACE','RUN_COMMANDS'],
      intelligencePolicy:{assignment:'AUTO',preferredProviders:[],allowedModels:[],allowPayg:false,minimumCapability:null}
    }}, 'Created from Organization Editor');
    $('#agent-dialog').close();
  } catch (error) { $('#agent-form-error').textContent = error.message; }
});
$('#relationship-form').addEventListener('submit', async event => {
  event.preventDefault();
  const form = event.currentTarget;
  if (form.elements.source.value === form.elements.target.value) {
    $('#relationship-form-error').textContent = 'Choose two different agents.';
    return;
  }
  const type = form.elements.type.value;
  try {
    await sendMutation({type:'ADD_RELATIONSHIP',data:{relationship:{
      id:`${type.toLowerCase()}:${form.elements.source.value}:${form.elements.target.value}`,
      type,source:form.elements.source.value,target:form.elements.target.value,persistent:true,
      taskId:null,label:form.elements.label.value || label(type)
    }}}, 'Added from Organization Editor');
    $('#relationship-dialog').close();
  } catch (error) { $('#relationship-form-error').textContent = error.message; }
});
$('#confirm-form').addEventListener('submit', async event => {
  event.preventDefault();
  if (!confirmOperation) return;
  const reason = $('#confirm-reason').value.trim() || null;
  try {
    await sendMutation(confirmOperation(reason), reason);
    $('#confirm-dialog').close();
    confirmOperation = null;
  } catch (error) { $('#confirm-description').insertAdjacentHTML('beforeend', `<p class="form-error">${esc(error.message)}</p>`); }
});
$('#policy-form').addEventListener('submit', async event => {
  event.preventDefault();
  const form = event.currentTarget;
  const current = snapshot.governance?.policy ?? {};
  const split = value => value.split(',').map(item => item.trim().toLowerCase()).filter(Boolean);
  const policy = {...current,
    schemaVersion:1,revision:snapshot.governance?.revision ?? 0,
    limits:{maxActiveAgents:Number(form.elements.maxActiveAgents.value),maxHierarchyDepth:Number(form.elements.maxHierarchyDepth.value)},
    providerApprovalTimeoutSeconds:Number(form.elements.providerApprovalTimeoutSeconds.value),
    allowPayg:form.elements.allowPayg.checked,autoAgentCreation:form.elements.autoAgentCreation.checked,
    permanentAgentsRequireGod:true,allowedProviders:split(form.elements.allowedProviders.value),deniedProviders:split(form.elements.deniedProviders.value),
    productionDeployRequiresGod:true
  };
  try {
    await sendMutation({type:'UPDATE_PROJECT_POLICY',data:{policy}}, 'Updated project governance policy');
    $('#policy-status').textContent = 'Saved';
  } catch (error) { $('#policy-status').textContent = error.message; }
});
$$('#inspector-tabs [data-tab]').forEach(button => button.addEventListener('click', () => {
  inspectorTab = button.dataset.tab;
  const agent = snapshot.agents.find(item => item.id === graphState.selectedId);
  if (agent) renderInspector(agent);
}));
$('#director-form').addEventListener('submit', async event => {
  event.preventDefault();
  const input = $('#director-input');
  const content = input.value.trim();
  if (!content) return;
  const message = document.createElement('div');
  message.className = 'message user';
  message.innerHTML = `<div><p>${esc(content)}</p><small>Sending…</small></div>`;
  $('#chat-log').append(message);
  input.value = '';
  try {
    const receipt = await sendMessage(content);
    $('small', message).textContent = `Sent to Director · ${receipt.id}`;
  } catch (error) { $('small', message).textContent = error.message; }
});
$$('.quick-prompts button').forEach(button => button.addEventListener('click', () => { $('#director-input').value = button.textContent; $('#director-input').focus(); }));
initializeGraphControls();
setInterval(() => {
  if ($('#view-decisions').classList.contains('active')) renderGovernance();
}, 1000);
boot();
