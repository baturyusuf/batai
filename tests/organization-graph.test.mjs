import test from 'node:test';
import assert from 'node:assert/strict';
import {
  agentMatches,
  buildCombinedGraph,
  buildHierarchyGraph,
  buildWorkflowGraph,
  preservedSelection
} from '../src/ui/organization-graph.js';

const snapshot = {
  god: {id:'god', label:'User'},
  agents: [
    {id:'director', name:'Orion', function:'DIRECTOR', department:'Leadership', seniority:'DIRECTOR', status:'READY', provider:'codex'},
    {id:'nova', name:'Nova', title:'Senior Backend Engineer', reportsTo:'director', department:'Engineering', seniority:'SENIOR', status:'RUNNING', provider:'codex'},
    {id:'iris', name:'Iris', title:'Senior QA Engineer', reportsTo:'missing', department:'Quality', seniority:'SENIOR', status:'READY', provider:'claude'}
  ],
  tasks: [
    {id:'T-1', objective:'Build API', status:'RUNNING', assignedTo:['nova'], dependencies:[]},
    {id:'T-2', objective:'Review API', status:'READY', assignedTo:['iris'], dependencies:['T-1']}
  ],
  relationships: [{id:'review', type:'REVIEW', source:'nova', target:'iris'}],
  hierarchyWarnings:['ORPHAN:iris:missing']
};

test('hierarchy graph follows reporting data and keeps orphan safe', () => {
  const graph = buildHierarchyGraph(snapshot);
  assert.equal(graph.nodes[0].id, 'god');
  assert.ok(graph.edges.some(edge => edge.source === 'god' && edge.target === 'director'));
  assert.ok(graph.edges.some(edge => edge.source === 'director' && edge.target === 'nova'));
  assert.ok(graph.warnings[0].startsWith('ORPHAN'));
});

test('workflow graph creates task dependency DAG', () => {
  const graph = buildWorkflowGraph(snapshot);
  assert.deepEqual(graph.nodes.map(node => node.id), ['task:T-1', 'task:T-2']);
  assert.ok(graph.edges.some(edge => edge.source === 'task:T-1' && edge.target === 'task:T-2'));
});

test('combined graph connects organization, ownership, tasks and review', () => {
  const graph = buildCombinedGraph(snapshot);
  assert.ok(graph.nodes.some(node => node.id === 'nova'));
  assert.ok(graph.nodes.some(node => node.id === 'task:T-1'));
  assert.ok(graph.edges.some(edge => edge.type === 'HANDOFF'));
  assert.ok(graph.edges.some(edge => edge.type === 'REVIEW'));
});

test('filters use explicit Rust-provided fields', () => {
  assert.equal(agentMatches(snapshot.agents[1], {departments:['Engineering'], statuses:['Working']}), true);
  assert.equal(agentMatches(snapshot.agents[2], {departments:['Engineering']}), false);
});

test('selection survives mode refresh only when node still exists', () => {
  const hierarchy = buildHierarchyGraph(snapshot);
  const workflow = buildWorkflowGraph(snapshot);
  assert.equal(preservedSelection(hierarchy, 'nova'), 'nova');
  assert.equal(preservedSelection(workflow, 'nova'), null);
});
