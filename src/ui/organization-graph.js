const ACTIVE = new Set(['RUNNING', 'INITIALIZING']);

export function agentMatches(agent, filters = {}) {
  if (filters.departments?.length && !filters.departments.includes(agent.department)) return false;
  if (filters.seniorities?.length && !filters.seniorities.includes(agent.seniority)) return false;
  if (filters.providers?.length && !filters.providers.includes(agent.provider)) return false;
  if (filters.statuses?.length) {
    const status = statusGroup(agent.status);
    if (!filters.statuses.includes(status)) return false;
  }
  return true;
}

export function statusGroup(status) {
  if (ACTIVE.has(status)) return 'Working';
  if (status === 'READY' || status === 'CREATED') return 'Ready';
  if (status === 'WAITING_RESOURCE' || status === 'PAUSED') return 'Waiting';
  if (status === 'BLOCKED' || status === 'FAILED') return 'Blocked';
  return 'Offline';
}

function hierarchyDepth(id, parentById, available, trail = new Set()) {
  if (id === 'god') return 0;
  if (trail.has(id)) return 1;
  trail.add(id);
  const parent = parentById.get(id);
  if (!parent || !available.has(parent)) return 1;
  return hierarchyDepth(parent, parentById, available, trail) + 1;
}

function layoutLayers(nodes, depthFor, horizontal = 280, vertical = 165) {
  const layers = new Map();
  nodes.forEach(node => {
    const depth = depthFor(node);
    const layer = layers.get(depth) ?? [];
    layer.push(node);
    layers.set(depth, layer);
  });
  for (const [depth, layer] of layers) {
    layer.sort((a, b) => a.id.localeCompare(b.id));
    const width = (layer.length - 1) * horizontal;
    layer.forEach((node, index) => {
      node.x = index * horizontal - width / 2;
      node.y = depth * vertical;
    });
  }
  return nodes;
}

export function buildHierarchyGraph(snapshot, filters = {}) {
  const agents = (snapshot.agents ?? []).filter(agent => agentMatches(agent, filters));
  const god = {id: 'god', type: 'god', data: snapshot.god ?? {label: 'User'}};
  const nodes = [god, ...agents.map(agent => ({id: agent.id, type: 'agent', data: agent}))];
  const available = new Set(nodes.map(node => node.id));
  const parentById = new Map();
  const edges = [];
  for (const agent of agents) {
    let parent = agent.reportsTo;
    if (!parent && (agent.function === 'DIRECTOR' || agent.id.toLowerCase() === 'director')) parent = 'god';
    parentById.set(agent.id, parent);
    if (parent && available.has(parent)) {
      edges.push({id: `reporting:${parent}:${agent.id}`, source: parent, target: agent.id, type: 'REPORTING', label: 'reports'});
    }
  }
  layoutLayers(nodes, node => hierarchyDepth(node.id, parentById, available));
  return {nodes, edges, warnings: snapshot.hierarchyWarnings ?? []};
}

function taskDepth(task, byId, trail = new Set()) {
  if (trail.has(task.id)) return 0;
  trail.add(task.id);
  const parents = (task.dependencies ?? []).map(id => byId.get(id)).filter(Boolean);
  return parents.length ? Math.max(...parents.map(parent => taskDepth(parent, byId, new Set(trail)))) + 1 : 0;
}

export function buildWorkflowGraph(snapshot) {
  const tasks = snapshot.tasks ?? [];
  const byId = new Map(tasks.map(task => [task.id, task]));
  const nodes = tasks.map(task => ({id: `task:${task.id}`, type: 'task', data: task}));
  layoutLayers(nodes, node => taskDepth(node.data, byId), 310, 175);
  const edges = tasks.flatMap(task => (task.dependencies ?? [])
    .filter(dependency => byId.has(dependency))
    .map(dependency => ({
      id: `dependency:${dependency}:${task.id}`,
      source: `task:${dependency}`,
      target: `task:${task.id}`,
      type: 'DEPENDENCY',
      label: 'depends'
    })));
  return {nodes, edges, warnings: []};
}

export function buildCombinedGraph(snapshot, filters = {}) {
  const hierarchy = buildHierarchyGraph(snapshot, filters);
  const agents = hierarchy.nodes.filter(node => node.type === 'agent');
  const availableAgents = new Set(agents.map(node => node.id));
  const tasks = (snapshot.tasks ?? []).filter(task =>
    !(task.assignedTo ?? []).length || task.assignedTo.some(id => availableAgents.has(id))
  );
  const taskNodes = tasks.map((task, index) => ({
    id: `task:${task.id}`,
    type: 'task',
    data: task,
    x: (index % 4) * 270 - Math.min(3, tasks.length - 1) * 135,
    y: (Math.floor(index / 4) + 1) * 170 + Math.max(0, ...hierarchy.nodes.map(node => node.y))
  }));
  const edges = [...hierarchy.edges];
  for (const task of tasks) {
    for (const owner of task.assignedTo ?? []) {
      if (availableAgents.has(owner)) {
        edges.push({id: `handoff:${owner}:${task.id}`, source: owner, target: `task:${task.id}`, type: 'HANDOFF', label: 'owns'});
      }
    }
    for (const dependency of task.dependencies ?? []) {
      if (tasks.some(candidate => candidate.id === dependency)) {
        edges.push({id: `dependency:${dependency}:${task.id}`, source: `task:${dependency}`, target: `task:${task.id}`, type: 'DEPENDENCY', label: 'depends'});
      }
    }
  }
  for (const relationship of snapshot.relationships ?? []) {
    if (relationship.type === 'COLLABORATION' || relationship.type === 'REVIEW') {
      if (availableAgents.has(relationship.source) && availableAgents.has(relationship.target)) {
        edges.push({...relationship, type: relationship.type});
      }
    }
  }
  return {nodes: [...hierarchy.nodes, ...taskNodes], edges, warnings: hierarchy.warnings};
}

export function buildGraph(snapshot, mode = 'hierarchy', filters = {}) {
  if (mode === 'workflow') return buildWorkflowGraph(snapshot);
  if (mode === 'combined') return buildCombinedGraph(snapshot, filters);
  return buildHierarchyGraph(snapshot, filters);
}

export function searchableText(node) {
  const data = node.data ?? {};
  return [data.name, data.title, data.id, data.objective, data.currentTaskId, data.currentTaskObjective]
    .filter(Boolean).join(' ').toLowerCase();
}

export function preservedSelection(graph, selectedId) {
  return graph.nodes.some(node => node.id === selectedId) ? selectedId : null;
}
