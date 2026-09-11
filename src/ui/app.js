import {buildGraph, preservedSelection, searchableText, statusGroup} from './organization-graph.js';

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const invoke = window.__TAURI__?.core?.invoke;
const esc = value => String(value ?? '').replace(/[&<>'"]/g, character => ({'&':'&amp;','<':'&lt;','>':'&gt;',"'":'&#39;','"':'&quot;'}[character]));
const label = value => String(value ?? 'Unknown').toLowerCase().replaceAll('_', ' ').replace(/\b\w/g, character => character.toUpperCase());
const valueOrDash = value => value === null || value === undefined || value === '' ? '—' : value;
const number = value => value === null || value === undefined ? '—' : Number(value).toLocaleString();
const percent = value => value === null || value === undefined ? '—' : `${Math.round(value * 10) / 10}%`;
const departmentLabel = value => value === 'DATA_AI' || value === 'DataAi' ? 'Data & AI' : label(value);

let snapshot = {god:{id:'god',label:'User'},project:{name:'Batai',progress:0,usage:{}},agents:[],tasks:[],relationships:[],resources:[],activity:[],hierarchyWarnings:[]};
let providers = [];
let refreshTimer;
let inspectorTab = 'overview';
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
  $('#inspector-content').innerHTML = `<div class="project-progress-ring"><strong>${snapshot.project.progress}%</strong><span>Weighted progress</span></div><section class="inspector-section"><h3>Tasks</h3><dl>${inspectorMetric('Completed',snapshot.project.completedTasks)}${inspectorMetric('Running',snapshot.project.runningTasks)}${inspectorMetric('Blocked',snapshot.project.blockedTasks)}${inspectorMetric('Remaining',snapshot.project.remainingTasks)}</dl></section><section class="inspector-section"><h3>Agents</h3><dl>${inspectorMetric('Active',snapshot.project.activeAgents)}${inspectorMetric('Waiting',snapshot.project.waitingAgents)}${inspectorMetric('Organization size',snapshot.agents.length)}</dl></section><section class="inspector-section"><h3>Inference</h3><dl>${inspectorMetric('Local',number(bySource.local?.totalTokens))}${inspectorMetric('Subscription',number(bySource.subscription?.totalTokens))}${inspectorMetric('PAYG / API',number(bySource.api?.totalTokens))}${inspectorMetric('Known tokens',number(usage.totalTokens))}${inspectorMetric('Known API spend',knownCost)}</dl></section><p class="metric-note">Unknown provider values are not estimated.</p>`;
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
    const events = snapshot.activity.filter(event => event.source === agent.id || event.target === agent.id);
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
    $('#inspector-content').innerHTML = `<section class="inspector-section"><h3>Permissions / Context</h3><dl>${inspectorMetric('Worktree',agent.worktree)}${inspectorMetric('Session',agent.sessionState)}${inspectorMetric('Auth source',label(agent.authMode))}${inspectorMetric('Organizational parent',agent.reportsTo)}${inspectorMetric('Direct reports',agent.directReports?.join(', ') || '—')}</dl></section><p class="metric-note">Execution trace includes observable runtime events only. Hidden model reasoning is never stored or shown.</p>`;
    return;
  }
  $('#inspector-content').innerHTML = `<div class="agent-inspector-hero"><div class="avatar ${agent.function === 'DIRECTOR' ? 'director' : ''}">${esc(initials(agent))}</div><div><strong>${esc(agent.name)}</strong><span>${esc(agent.title)}</span><small>${esc(activityIcon(agent.activity))} ${esc(label(agent.activity))}</small></div></div><section class="inspector-section"><h3>Organizational identity</h3><dl>${inspectorMetric('Seniority',agent.seniority ? `${label(agent.seniority)} · L${agent.level}` : 'Unspecified')}${inspectorMetric('Function',label(agent.function))}${inspectorMetric('Department',agent.department)}${inspectorMetric('Reports to',agent.reportsTo)}${inspectorMetric('Direct reports',agent.directReports?.join(', ') || '—')}</dl></section><section class="inspector-section"><h3>Assigned intelligence</h3><dl>${inspectorMetric('Model',agent.model)}${inspectorMetric('Provider',label(agent.provider))}${inspectorMetric('Reasoning effort',label(agent.reasoningEffort))}${inspectorMetric('Auth / usage',label(agent.authMode))}</dl></section><section class="inspector-section"><h3>Current runtime</h3><dl>${inspectorMetric('Status',label(agent.status))}${inspectorMetric('Activity',label(agent.activity))}${inspectorMetric('Task',agent.currentTaskId)}${inspectorMetric('Worktree',agent.worktree)}${inspectorMetric('Session',agent.sessionState)}</dl></section>`;
}

function renderTaskInspector(task) {
  $('#inspector-heading').textContent = task.id;
  $('#inspector-tabs').hidden = true;
  $('#inspector-content').innerHTML = `<section class="inspector-section"><h3>Task</h3><p class="task-objective">${esc(task.objective)}</p><dl>${inspectorMetric('Status',label(task.status))}${inspectorMetric('Owner',(task.assignedTo ?? []).join(', ') || 'Unassigned')}${inspectorMetric('Dependencies',(task.dependencies ?? []).join(', ') || '—')}${inspectorMetric('Weight',task.weight)}</dl></section><button class="primary-button" id="open-task-board">Open Task Board</button>`;
  $('#open-task-board').addEventListener('click', () => switchView('tasks'));
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
    renderOverview(); renderTasks(); renderProviders(); renderResources(); renderFilters(); renderGraph(true);
    graphState.fitted = false;
    $('#blocker-count').textContent = snapshot.project.blockedTasks ?? 0;
    const listen = window.__TAURI__?.event?.listen;
    if (listen) await listen('batai://runtime-event', () => {
      clearTimeout(refreshTimer);
      refreshTimer = setTimeout(async () => {
        try {
          snapshot = await loadSnapshot();
          renderOverview(); renderTasks(); renderResources(); renderFilters(); renderGraph(true);
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
boot();
