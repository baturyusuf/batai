import fs from 'node:fs';
import path from 'node:path';

function readJson(file, fallback={}) {
  try { return JSON.parse(fs.readFileSync(file,'utf8')); } catch { return fallback; }
}

export class PolicyEngine {
  constructor({ projectRoot, store }) {
    this.projectRoot = projectRoot;
    this.store = store;
  }

  organizationPolicy() {
    const org = readJson(path.join(this.projectRoot,'.batai','organization.json'),{});
    return {
      maxActiveAgents: org.limits?.max_active_agents ?? 8,
      maxHierarchyDepth: org.limits?.max_hierarchy_depth ?? 3
    };
  }

  hierarchyDepth(parentId) {
    if (!parentId) return 1;
    let depth = 1; let current = this.store.getAgent(parentId); const seen = new Set();
    while (current?.parent_agent_id) {
      if (seen.has(current.id)) throw new Error('Agent hierarchy cycle detected');
      seen.add(current.id); depth += 1; current = this.store.getAgent(current.parent_agent_id);
    }
    return depth + 1;
  }

  validateAgentCreation(config, { godApproved=false } = {}) {
    const existing = this.store.getAgent(config.id);
    const { maxActiveAgents, maxHierarchyDepth } = this.organizationPolicy();
    if (!existing) {
      const active = this.store.listAgents().filter(a => a.status !== 'TERMINATED').length;
      if (active >= maxActiveAgents) throw new Error(`Agent limit reached (${maxActiveAgents}). Director must reuse/terminate an agent or request GOD policy change.`);
    }
    if (config.parent_agent_id && !this.store.getAgent(config.parent_agent_id)) throw new Error(`Parent agent not found: ${config.parent_agent_id}`);
    const depth = this.hierarchyDepth(config.parent_agent_id);
    if (depth > maxHierarchyDepth) throw new Error(`Hierarchy depth ${depth} exceeds project limit ${maxHierarchyDepth}`);
    if (config.lifetime === 'permanent' && !godApproved) throw new Error('Permanent agent creation requires GOD approval');
    return { ok:true, depth, maxActiveAgents, maxHierarchyDepth };
  }
}
