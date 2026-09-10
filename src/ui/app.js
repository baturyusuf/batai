const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const invoke = window.__TAURI__?.core?.invoke;
const esc = value => String(value ?? '').replace(/[&<>'"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;',"'":'&#39;','"':'&quot;'}[c]));
const title = value => String(value ?? '').toLowerCase().replaceAll('_',' ').replace(/\b\w/g, c => c.toUpperCase());
let snapshot = {project:{name:'Batai',progress:0,activeAgents:0,blockedTasks:0},agents:[],tasks:[]};
let providers = [];

function department(role = '') {
  const value = role.toLowerCase();
  if (value.includes('director') || value.includes('product')) return 'Leadership';
  if (value.includes('review') || value.includes('qa') || value.includes('test')) return 'Quality';
  if (value.includes('design') || value.includes('frontend')) return 'Product Design';
  return 'Engineering';
}

async function loadSnapshot() {
  if (invoke) return invoke('get_app_snapshot');
  const response = await fetch('/api/state');
  if (!response.ok) throw new Error('Control plane is offline');
  const state = await response.json();
  const tasks = (state.tasks ?? []).map(task => ({
    id:task.id, objective:task.objective, status:task.status,
    assignedTo:task.assigned_to ?? [], dependencies:task.dependencies ?? [],
    weight:task.weight ?? 1,
    progress:task.progress ?? ({COMPLETED:1,REVIEW:.9,RUNNING:.55,WAITING_RESOURCE:.4,BLOCKED:.25}[task.status] ?? 0)
  }));
  const agents = (state.agents ?? []).map(agent => ({
    id:agent.id, name:agent.name, title:agent.role_template,
    department:department(agent.role_template), reportsTo:agent.parent_agent_id,
    provider:agent.provider, model:agent.model, authMode:agent.auth_mode,
    status:agent.status, currentTaskId:agent.current_task_id, worktree:agent.worktree
  }));
  const total = tasks.reduce((sum, task) => sum + task.weight, 0);
  const progress = total ? Math.round(tasks.reduce((sum, task) => sum + task.weight * task.progress, 0) / total * 1000) / 10 : 0;
  return {project:{name:'batai',progress,activeAgents:agents.filter(a => ['READY','RUNNING'].includes(a.status)).length,blockedTasks:tasks.filter(t => t.status === 'BLOCKED').length},agents,tasks};
}

async function loadProviders() {
  if (invoke) return invoke('get_provider_connections');
  const response = await fetch('/api/providers');
  if (!response.ok) return [];
  return (await response.json()).map(item => ({
    id:item.name === 'codex-cli' ? 'codex' : item.name === 'claude-cli' ? 'claude' : item.name,
    name:{'codex-cli':'Codex','claude-cli':'Claude Code',ollama:'Ollama',mock:'Mock Runtime'}[item.name] ?? item.name,
    kind:item.name === 'ollama' ? 'Local runtime' : item.name === 'mock' ? 'Development' : 'Subscription',
    status:item.usage?.status === 'AVAILABLE' ? 'connected' : 'unavailable',
    statusLabel:title(item.usage?.status ?? 'Unknown'),
    accountLabel:item.usage?.status === 'AVAILABLE' ? 'Available on this computer' : 'Setup required',
    detail:(item.models ?? []).join(', ') || 'Models detected dynamically',
    actionLabel:'Connection guide'
  }));
}

function initials(agent) {
  return (agent.name || agent.title || '?').split(/\s+/).slice(0,2).map(w => w[0]).join('').toUpperCase();
}

function agentNode(agent) {
  const kind = agent.id === 'director' ? ' director' : agent.department === 'Quality' ? ' quality' : '';
  return '<article class="agent-node" data-agent-id="' + esc(agent.id) + '"><div class="node-head">' +
    '<div class="avatar' + kind + '">' + esc(initials(agent)) + '</div><div><strong>' + esc(agent.name) +
    '</strong><small>' + esc(agent.title) + '</small></div></div><footer><span class="status"><i></i>' +
    esc(title(agent.status)) + '</span><small class="model-badge">' + esc(agent.model || 'Auto') + '</small></footer></article>';
}

function renderOverview() {
  $('#project-name').textContent = snapshot.project.name;
  $('#crumb-project').textContent = snapshot.project.name;
  $('#metric-progress').textContent = snapshot.project.progress + '%';
  $('#progress-bar').style.width = snapshot.project.progress + '%';
  $('#metric-agents').textContent = snapshot.project.activeAgents;
  $('#agent-summary').textContent = snapshot.agents.length + ' people in the organization';
  const moving = snapshot.tasks.filter(t => ['RUNNING','REVIEW','WAITING_RESOURCE','BLOCKED'].includes(t.status)).length;
  $('#metric-tasks').textContent = moving;
  $('#task-summary').textContent = snapshot.project.blockedTasks ? snapshot.project.blockedTasks + ' blocked' : 'No blockers';
  $('#task-nav-count').textContent = snapshot.tasks.length;
  const connected = providers.filter(p => ['connected','local'].includes(p.status)).length;
  $('#metric-providers').textContent = connected + '/' + (providers.length || '—');
  $('#provider-summary').textContent = connected ? 'Official sessions detected' : 'Connect an intelligence source';
  const director = snapshot.agents.find(a => a.id === 'director');
  const workers = snapshot.agents.filter(a => a.id !== 'director').slice(0,4);
  $('#organization-preview').innerHTML = snapshot.agents.length ? (director ? agentNode(director) : '') + workers.map(agentNode).join('') : '<p class="empty">Create your first project team.</p>';
  $('#task-list').innerHTML = snapshot.tasks.slice(0,5).map(t => '<div class="task-row"><i></i><div><strong>' + esc(t.objective) + '</strong><small>' + esc(t.assignedTo.join(', ') || 'Unassigned') + '</small></div><b>' + esc(title(t.status)) + '</b></div>').join('') || '<p class="empty">No tasks yet. Ask Director to plan the first milestone.</p>';
  $('#provider-mini-list').innerHTML = providers.slice(0,4).map(p => '<div class="provider-mini"><div><strong>' + esc(p.name) + '</strong><small>' + esc(p.kind) + '</small></div><span class="connection-dot ' + esc(p.status) + '">' + esc(p.statusLabel) + '</span></div>').join('');
  bindAgents();
}

function renderOrganization() {
  const groups = snapshot.agents.reduce((all, agent) => {
    (all[agent.department] ??= []).push(agent);
    return all;
  }, {});
  $('#organization-map').innerHTML = Object.entries(groups).map(([name, agents]) =>
    '<section class="department-column"><span class="department-label">' + esc(name) + '</span>' +
    agents.map(agentNode).join('') + '</section>'
  ).join('') || '<p class="empty">The organization is empty.</p>';
  bindAgents();
}

function renderTasks() {
  const columns = [['Backlog',['PENDING','READY']],['In progress',['RUNNING','WAITING_RESOURCE','BLOCKED']],['Review',['REVIEW']],['Completed',['COMPLETED']]];
  $('#task-board').innerHTML = columns.map(([name, statuses]) => {
    const tasks = snapshot.tasks.filter(task => statuses.includes(task.status));
    return '<section class="task-column"><header><span>' + name + '</span><b>' + tasks.length + '</b></header>' +
      tasks.map(task => '<article class="task-card"><strong>' + esc(task.id) + '</strong><p>' + esc(task.objective) +
      '</p><footer><span>' + esc(title(task.status)) + '</span><span>' + esc(task.assignedTo[0] ?? 'Unassigned') + '</span></footer></article>').join('') + '</section>';
  }).join('');
}

function renderProviders() {
  const logos = {codex:'◎',claude:'A',gh:'⌘',ollama:'◉',mock:'M'};
  $('#provider-grid').innerHTML = providers.map(p =>
    '<article class="provider-card"><div class="provider-head"><div class="provider-logo">' + esc(logos[p.id] ?? p.name[0]) +
    '</div><div><strong>' + esc(p.name) + '</strong><span>' + esc(p.kind) + '</span></div><span class="provider-status ' +
    esc(p.status) + '">' + esc(p.statusLabel) + '</span></div><div class="provider-detail"><span>Account</span><strong>' +
    esc(p.accountLabel) + '</strong></div><footer><small>' + esc(p.detail) + '</small><button data-provider="' + esc(p.id) +
    '">' + esc(p.actionLabel) + '</button></footer></article>'
  ).join('') || '<p class="empty">No providers configured.</p>';
  $$('[data-provider]').forEach(button => button.addEventListener('click', () => showGuide(button.dataset.provider)));
}

function bindAgents() {
  $$('[data-agent-id]').forEach(node => node.addEventListener('click', () => inspectAgent(node.dataset.agentId)));
}

function inspectAgent(id) {
  const agent = snapshot.agents.find(item => item.id === id);
  if (!agent) return;
  $$('.agent-node').forEach(node => node.classList.toggle('selected', node.dataset.agentId === id));
  $('.director-profile .avatar').textContent = initials(agent);
  $('.director-profile strong').textContent = agent.name;
  $('.director-profile span').innerHTML = '<i></i>' + esc(title(agent.status)) + ' · ' + esc(agent.title);
}

async function showGuide(providerId) {
  let guide;
  if (invoke) {
    guide = await invoke('get_connection_guide', {providerId});
  } else {
    const provider = providers.find(item => item.id === providerId);
    const commands = {codex:'codex login',claude:'claude auth login',gh:'gh auth login',ollama:'ollama serve'};
    guide = {title:'Connect ' + (provider?.name ?? providerId),description:'Use the official provider flow. Batai never stores your raw password or token.',command:commands[providerId],steps:['Open a terminal','Run the official connection command','Complete authentication','Return to Batai and check again']};
  }
  $('#dialog-title').textContent = guide.title;
  $('#dialog-description').textContent = guide.description;
  $('#dialog-steps').innerHTML = guide.steps.map(step => '<li>' + esc(step) + '</li>').join('');
  $('#dialog-command').textContent = guide.command ?? 'No command required';
  $('#copy-command').dataset.command = guide.command ?? '';
  $('#connection-dialog').showModal();
}

function switchView(view) {
  $$('.view').forEach(section => section.classList.toggle('active', section.id === 'view-' + view));
  $$('.nav-item').forEach(item => item.classList.toggle('active', item.dataset.view === view));
  $('#crumb-view').textContent = title(view);
}

async function sendMessage(content) {
  if (invoke) return invoke('send_director_message', {content});
  const response = await fetch('/api/god/messages', {method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({content,scope:'task',target:'director'})});
  if (!response.ok) throw new Error('Control plane rejected the message');
  return response.json();
}

async function boot() {
  try {
    [snapshot, providers] = await Promise.all([loadSnapshot(), loadProviders()]);
    renderOverview();
    renderOrganization();
    renderTasks();
    renderProviders();
  } catch (error) {
    $('.runtime-state strong').textContent = 'Runtime offline';
    $('.runtime-state>i').style.background = 'var(--red)';
    $('#organization-preview').innerHTML = '<p class="empty">' + esc(error.message) + '</p>';
  }
}

$$('[data-view]').forEach(button => button.addEventListener('click', () => switchView(button.dataset.view)));
$$('[data-view-link]').forEach(button => button.addEventListener('click', () => switchView(button.dataset.viewLink)));
$('#open-accounts').addEventListener('click', () => switchView('accounts'));
$('#refresh-providers').addEventListener('click', async () => { providers = await loadProviders(); renderProviders(); renderOverview(); });
$('#copy-command').addEventListener('click', () => navigator.clipboard.writeText($('#copy-command').dataset.command || ''));
$('#director-form').addEventListener('submit', async event => {
  event.preventDefault();
  const input = $('#director-input');
  const content = input.value.trim();
  if (!content) return;
  const message = document.createElement('div');
  message.className = 'message user';
  message.innerHTML = '<div><p>' + esc(content) + '</p><small>Sending…</small></div>';
  $('#chat-log').append(message);
  input.value = '';
  try {
    const receipt = await sendMessage(content);
    $('small', message).textContent = 'Sent to Director · ' + receipt.id;
  } catch (error) {
    $('small', message).textContent = error.message;
  }
});
$$('.quick-prompts button').forEach(button => button.addEventListener('click', () => {
  $('#director-input').value = button.textContent;
  $('#director-input').focus();
}));
boot();
