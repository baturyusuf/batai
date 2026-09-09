const el = id => document.getElementById(id);

async function api(url, options = {}) {
  const response = await fetch(url, { headers: { 'content-type': 'application/json' }, ...options });
  const body = await response.json();
  if (!response.ok) throw new Error(body.details?.join?.(', ') || body.message || 'Request failed');
  return body;
}

function esc(value) {
  return String(value ?? '').replace(/[&<>'"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;',"'":'&#39;','"':'&quot;'}[c]));
}

function renderAgents(agents, resources) {
  const resourceMap = new Map(resources.map(r => [r.agentId, r]));
  el('agent-count').textContent = `${agents.length} agents`;
  el('agents').innerHTML = agents.map(a => {
    const resource = resourceMap.get(a.id);
    return `<article class="agent-card">
      <div class="head"><strong>${esc(a.name)}</strong><span><i class="dot ${esc(a.status)}"></i> ${esc(a.status)}</span></div>
      <dl><dt>ID</dt><dd>${esc(a.id)}</dd><dt>Role</dt><dd>${esc(a.role_template)}</dd><dt>Provider</dt><dd>${esc(a.provider)}</dd><dt>Model</dt><dd>${esc(a.model)}</dd><dt>Reasoning</dt><dd>${esc(a.reasoning_effort)}</dd><dt>Resource</dt><dd>${esc(resource?.status ?? 'UNKNOWN')}</dd><dt>Task</dt><dd>${esc(a.current_task_id ?? '—')}</dd></dl>
    </article>`;
  }).join('') || '<p class="muted">No agents yet.</p>';
}


function renderProviders(providers) {
  el('providers').innerHTML = providers.map(p => `<article class="agent-card">
    <div class="head"><strong>${esc(p.name)}</strong><span><i class="dot ${esc(p.usage?.status === 'AVAILABLE' ? 'READY' : p.usage?.status)}"></i> ${esc(p.usage?.status ?? 'UNKNOWN')}</span></div>
    <dl><dt>Models</dt><dd>${esc((p.models ?? []).join(', ') || 'dynamic / unavailable')}</dd><dt>Reset</dt><dd>${esc(p.usage?.resetAt ?? '—')}</dd></dl>
  </article>`).join('');
}

async function refreshProviders() {
  try { renderProviders(await api('/api/providers')); } catch { renderProviders([]); }
}


function renderInbox(messages = []) {
  const pending = messages.filter(m => m.status === 'PENDING');
  el('director-inbox').innerHTML = pending.slice(0,8).map(m => `<div class="list-row"><strong>${esc(m.scope)}</strong><span>${esc(m.content)}</span><small>${esc(new Date(m.timestamp).toLocaleString())}</small></div>`).join('') || '<p class="muted">No pending GOD instructions.</p>';
}

function renderDecisions(decisions = []) {
  el('decision-count').textContent = `${decisions.length} decisions`;
  el('decisions').innerHTML = decisions.slice(0,20).map(d => `<article class="decision-row">
    <div><strong>${esc(d.topic)}</strong><span class="badge">${esc(d.status)}</span></div>
    <p>${esc(d.question)}</p>
    <small>Recommended: ${esc(typeof d.director_recommendation === 'string' ? d.director_recommendation : JSON.stringify(d.director_recommendation))}</small>
    ${d.status === 'OPEN' && d.requires === 'GOD' ? `<div class="decision-actions"><input data-decision-value="${esc(d.id)}" placeholder="GOD decision"/><button data-resolve-decision="${esc(d.id)}">Resolve</button></div>` : `<small>Effective: ${esc(typeof d.effective_decision === 'string' ? d.effective_decision : JSON.stringify(d.effective_decision))}</small>`}
  </article>`).join('') || '<p class="muted">No decisions yet.</p>';
  document.querySelectorAll('[data-resolve-decision]').forEach(button => button.addEventListener('click', async () => {
    const id = button.dataset.resolveDecision;
    const input = document.querySelector(`[data-decision-value="${CSS.escape(id)}"]`);
    if (!input.value.trim()) return;
    await api(`/api/decisions/${encodeURIComponent(id)}/resolve`, {method:'POST',body:JSON.stringify({value:input.value.trim()})});
    await refresh();
  }));
}

function renderTasks(tasks) {
  el('task-count').textContent = `${tasks.length} tasks`;
  el('tasks').innerHTML = tasks.map(t => `<article class="task ${esc(t.status)}">
    <div class="panel-title"><strong>${esc(t.id)}</strong><span class="badge">${esc(t.status)}</span></div>
    <p>${esc(t.objective)}</p>
    <div class="meta">Agents: ${esc((t.assigned_to ?? []).join(', '))}<br>Dependencies: ${esc((t.dependencies ?? []).join(', ') || 'none')}</div>
    ${t.status === 'REVIEW' ? `<button data-approve="${esc(t.id)}">Director approve</button>` : ''}
  </article>`).join('') || '<p class="muted">No tasks yet.</p>';
  document.querySelectorAll('[data-approve]').forEach(button => button.addEventListener('click', async () => {
    await api(`/api/tasks/${encodeURIComponent(button.dataset.approve)}/approve`, { method:'POST', body:'{}' });
    await refresh();
  }));
}

function renderEvents(events) {
  el('events').innerHTML = events.slice(0, 100).map(e => `<div class="event">
    <span>${esc(new Date(e.timestamp).toLocaleString())}</span><span class="type">${esc(e.type)}</span><span>${esc(e.source)} → ${esc(e.target ?? '—')}</span><span>${esc(e.task_id ?? '')}</span>
  </div>`).join('') || '<p class="muted">No events yet.</p>';
}

async function refresh() {
  try {
    const state = await api('/api/state');
    renderAgents(state.agents, state.resources);
    renderTasks(state.tasks);
    renderEvents(state.events);
    renderInbox(state.director_inbox ?? []);
    renderDecisions(state.decisions ?? []);
    el('health').textContent = 'runtime online'; el('health').className = 'status-pill ok';
  } catch (error) {
    el('health').textContent = 'runtime offline'; el('health').className = 'status-pill';
  }
}

el('create-agent').addEventListener('click', async () => {
  const id = el('agent-id').value.trim();
  try {
    await api('/api/agents', { method:'POST', body:JSON.stringify({
      id, name:el('agent-name').value.trim() || id, role_template:el('agent-role').value.trim() || 'SoftwareEngineer',
      parent_agent_id:'director', provider:el('agent-provider').value, model:el('agent-model').value.trim(), reasoning_effort:el('agent-reasoning').value,
      auth_mode:'local', worktree:null, allowed_paths:['**/*'], tools:['git','shell'], constraints:['Do not contact customer'],
      max_turns:18, lifetime:'project', status:'READY'
    }) });
    el('agent-form-message').textContent = `Created ${id}`;
    await refresh();
  } catch (error) { el('agent-form-message').textContent = error.message; }
});

el('create-task').addEventListener('click', async () => {
  const id = el('task-id').value.trim();
  try {
    await api('/api/tasks', { method:'POST', body:JSON.stringify({
      id, created_by:'director', objective:el('task-objective').value.trim(), assigned_to:[el('task-agent').value.trim()],
      dependencies:el('task-deps').value.split(',').map(x=>x.trim()).filter(Boolean), acceptance_criteria:['Complete the assigned objective'], inputs:[], outputs:[],
      status:'READY', execution:{parallel:false,requires_director_review:el('task-review').checked}, on_success:{notify:'director',reason:'task_completed'}, on_failure:{notify:'director'}
    }) });
    el('task-form-message').textContent = `Task ${id} written to .batai/tasks/`;
    await new Promise(r => setTimeout(r, 150)); await refresh();
  } catch (error) { el('task-form-message').textContent = error.message; }
});


el('send-god-message').addEventListener('click', async () => {
  try {
    const content = el('god-message').value.trim();
    if (!content) return;
    const created = await api('/api/god/messages',{method:'POST',body:JSON.stringify({content,scope:el('god-scope').value,target:'director'})});
    el('god-form-message').textContent = `Sent ${created.id} to Director`;
    el('god-message').value = '';
    await refresh();
  } catch (error) { el('god-form-message').textContent = error.message; }
});

refresh();
refreshProviders();
setInterval(refresh, 1500);
setInterval(refreshProviders, 10000);
