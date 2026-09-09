import fs from 'node:fs';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

function ensureParent(filePath) {
  if (filePath !== ':memory:') fs.mkdirSync(path.dirname(filePath), { recursive: true });
}

function parseJson(value, fallback = null) {
  if (value == null) return fallback;
  try { return JSON.parse(value); } catch { return fallback; }
}

export class RuntimeStore {
  constructor(dbPath = ':memory:') {
    ensureParent(dbPath);
    this.dbPath = dbPath;
    this.db = new DatabaseSync(dbPath);
    this.db.exec('PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;');
    this.#migrate();
  }

  #migrate() {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS agents (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT NOT NULL,
        reasoning_effort TEXT NOT NULL,
        status TEXT NOT NULL,
        parent_agent_id TEXT,
        current_task_id TEXT,
        config_json TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS tasks (
        id TEXT PRIMARY KEY,
        status TEXT NOT NULL,
        created_by TEXT NOT NULL,
        objective TEXT NOT NULL,
        assigned_to_json TEXT NOT NULL,
        source_file TEXT,
        task_json TEXT NOT NULL,
        result_json TEXT,
        updated_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS events (
        id TEXT PRIMARY KEY,
        type TEXT NOT NULL,
        timestamp TEXT NOT NULL,
        source TEXT NOT NULL,
        target TEXT,
        task_id TEXT,
        payload_json TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS resources (
        agent_id TEXT PRIMARY KEY,
        status TEXT NOT NULL,
        reset_at TEXT,
        details_json TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS sessions (
        agent_id TEXT PRIMARY KEY,
        provider TEXT NOT NULL,
        provider_session_id TEXT NOT NULL,
        metadata_json TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS scheduler_jobs (
        id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        run_at TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        status TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS authority_messages (
        id TEXT PRIMARY KEY,
        source TEXT NOT NULL,
        target TEXT NOT NULL,
        authority INTEGER NOT NULL,
        scope TEXT NOT NULL,
        content TEXT NOT NULL,
        status TEXT NOT NULL,
        timestamp TEXT NOT NULL,
        metadata_json TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS decisions (
        id TEXT PRIMARY KEY,
        topic TEXT NOT NULL,
        question TEXT NOT NULL,
        recommendation_json TEXT,
        alternatives_json TEXT NOT NULL,
        impact TEXT NOT NULL,
        requires TEXT NOT NULL,
        effective_json TEXT,
        decided_by TEXT,
        status TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
    `);
  }

  close() { this.db.close(); }

  upsertAgent(agent) {
    const now = new Date().toISOString();
    this.db.prepare(`
      INSERT INTO agents(id,name,provider,model,reasoning_effort,status,parent_agent_id,current_task_id,config_json,updated_at)
      VALUES(?,?,?,?,?,?,?,?,?,?)
      ON CONFLICT(id) DO UPDATE SET
        name=excluded.name, provider=excluded.provider, model=excluded.model,
        reasoning_effort=excluded.reasoning_effort, status=excluded.status,
        parent_agent_id=excluded.parent_agent_id, config_json=excluded.config_json, updated_at=excluded.updated_at
    `).run(agent.id, agent.name, agent.provider, agent.model, agent.reasoning_effort, agent.status,
      agent.parent_agent_id ?? null, agent.current_task_id ?? null, JSON.stringify(agent), now);
    return this.getAgent(agent.id);
  }

  getAgent(id) {
    const row = this.db.prepare('SELECT * FROM agents WHERE id=?').get(id);
    if (!row) return null;
    const config = parseJson(row.config_json, {});
    return { ...config, status: row.status, current_task_id: row.current_task_id ?? null };
  }

  listAgents() {
    return this.db.prepare('SELECT id FROM agents ORDER BY id').all().map(r => this.getAgent(r.id));
  }

  setAgentStatus(id, status, currentTaskId = undefined) {
    const now = new Date().toISOString();
    const row = this.db.prepare('SELECT config_json,current_task_id FROM agents WHERE id=?').get(id);
    if (!row) return null;
    const config = parseJson(row.config_json, {});
    config.status = status;
    if (currentTaskId !== undefined) config.current_task_id = currentTaskId;
    this.db.prepare('UPDATE agents SET status=?, current_task_id=?, config_json=?, updated_at=? WHERE id=?')
      .run(status, currentTaskId === undefined ? row.current_task_id : currentTaskId, JSON.stringify(config), now, id);
    return this.getAgent(id);
  }

  upsertTask(task, sourceFile = null) {
    const now = new Date().toISOString();
    this.db.prepare(`
      INSERT INTO tasks(id,status,created_by,objective,assigned_to_json,source_file,task_json,result_json,updated_at)
      VALUES(?,?,?,?,?,?,?,?,?)
      ON CONFLICT(id) DO UPDATE SET
        status=excluded.status, created_by=excluded.created_by, objective=excluded.objective,
        assigned_to_json=excluded.assigned_to_json, source_file=COALESCE(excluded.source_file,tasks.source_file),
        task_json=excluded.task_json, updated_at=excluded.updated_at
    `).run(task.id, task.status, task.created_by, task.objective, JSON.stringify(task.assigned_to), sourceFile,
      JSON.stringify(task), null, now);
    return this.getTask(task.id);
  }

  getTask(id) {
    const row = this.db.prepare('SELECT * FROM tasks WHERE id=?').get(id);
    if (!row) return null;
    const task = parseJson(row.task_json, {});
    return { ...task, status: row.status, result: parseJson(row.result_json), source_file: row.source_file ?? null };
  }

  listTasks() {
    return this.db.prepare('SELECT id FROM tasks ORDER BY updated_at DESC').all().map(r => this.getTask(r.id));
  }

  setTaskStatus(id, status) {
    const row = this.db.prepare('SELECT task_json FROM tasks WHERE id=?').get(id);
    if (!row) return null;
    const task = parseJson(row.task_json, {});
    task.status = status;
    this.db.prepare('UPDATE tasks SET status=?, task_json=?, updated_at=? WHERE id=?')
      .run(status, JSON.stringify(task), new Date().toISOString(), id);
    return this.getTask(id);
  }

  setTaskResult(id, result) {
    this.db.prepare('UPDATE tasks SET result_json=?, updated_at=? WHERE id=?')
      .run(JSON.stringify(result ?? null), new Date().toISOString(), id);
    return this.getTask(id);
  }

  appendEvent(event) {
    this.db.prepare(`
      INSERT OR IGNORE INTO events(id,type,timestamp,source,target,task_id,payload_json)
      VALUES(?,?,?,?,?,?,?)
    `).run(event.id, event.type, event.timestamp, event.source, event.target ?? null,
      event.task_id ?? null, JSON.stringify(event.payload ?? {}));
    return event;
  }

  listEvents(limit = 200) {
    return this.db.prepare('SELECT * FROM events ORDER BY timestamp DESC LIMIT ?').all(limit).map(row => ({
      id: row.id,
      type: row.type,
      timestamp: row.timestamp,
      source: row.source,
      target: row.target ?? null,
      task_id: row.task_id ?? null,
      payload: parseJson(row.payload_json, {})
    }));
  }


  upsertSession(agentId, provider, providerSessionId, metadata = {}) {
    const now = new Date().toISOString();
    this.db.prepare(`
      INSERT INTO sessions(agent_id,provider,provider_session_id,metadata_json,updated_at)
      VALUES(?,?,?,?,?)
      ON CONFLICT(agent_id) DO UPDATE SET provider=excluded.provider, provider_session_id=excluded.provider_session_id,
      metadata_json=excluded.metadata_json, updated_at=excluded.updated_at
    `).run(agentId, provider, providerSessionId, JSON.stringify(metadata), now);
    return this.getSession(agentId);
  }

  getSession(agentId) {
    const row = this.db.prepare('SELECT * FROM sessions WHERE agent_id=?').get(agentId);
    if (!row) return null;
    return { agentId, provider:row.provider, sessionId:row.provider_session_id, metadata:parseJson(row.metadata_json,{}) };
  }

  listSessions() {
    return this.db.prepare('SELECT agent_id FROM sessions ORDER BY agent_id').all().map(r => this.getSession(r.agent_id));
  }


  appendAuthorityMessage(message) {
    this.db.prepare(`
      INSERT OR REPLACE INTO authority_messages(id,source,target,authority,scope,content,status,timestamp,metadata_json)
      VALUES(?,?,?,?,?,?,?,?,?)
    `).run(message.id, message.source, message.target, message.authority, message.scope, message.content,
      message.status ?? 'PENDING', message.timestamp, JSON.stringify(message.metadata ?? {}));
    return this.getAuthorityMessage(message.id);
  }

  getAuthorityMessage(id) {
    const row = this.db.prepare('SELECT * FROM authority_messages WHERE id=?').get(id);
    if (!row) return null;
    return { id:row.id, source:row.source, target:row.target, authority:row.authority, scope:row.scope,
      content:row.content, status:row.status, timestamp:row.timestamp, metadata:parseJson(row.metadata_json,{}) };
  }

  listAuthorityMessages({ target = null, status = null, limit = 200 } = {}) {
    let sql = 'SELECT id FROM authority_messages'; const where=[]; const params=[];
    if (target) { where.push('target=?'); params.push(target); }
    if (status) { where.push('status=?'); params.push(status); }
    if (where.length) sql += ` WHERE ${where.join(' AND ')}`;
    sql += ' ORDER BY timestamp DESC LIMIT ?'; params.push(limit);
    return this.db.prepare(sql).all(...params).map(r => this.getAuthorityMessage(r.id));
  }

  setAuthorityMessageStatus(id, status) {
    this.db.prepare('UPDATE authority_messages SET status=? WHERE id=?').run(status,id);
    return this.getAuthorityMessage(id);
  }

  upsertDecision(decision) {
    const now = new Date().toISOString();
    this.db.prepare(`
      INSERT INTO decisions(id,topic,question,recommendation_json,alternatives_json,impact,requires,effective_json,decided_by,status,updated_at)
      VALUES(?,?,?,?,?,?,?,?,?,?,?)
      ON CONFLICT(id) DO UPDATE SET topic=excluded.topic, question=excluded.question,
      recommendation_json=excluded.recommendation_json, alternatives_json=excluded.alternatives_json,
      impact=excluded.impact, requires=excluded.requires, effective_json=excluded.effective_json,
      decided_by=excluded.decided_by, status=excluded.status, updated_at=excluded.updated_at
    `).run(decision.id, decision.topic, decision.question, JSON.stringify(decision.director_recommendation ?? null),
      JSON.stringify(decision.alternatives ?? []), decision.impact ?? 'medium', decision.requires ?? 'GOD',
      JSON.stringify(decision.effective_decision ?? null), decision.decided_by ?? null, decision.status ?? 'OPEN', now);
    return this.getDecision(decision.id);
  }

  getDecision(id) {
    const row = this.db.prepare('SELECT * FROM decisions WHERE id=?').get(id);
    if (!row) return null;
    return { id:row.id, topic:row.topic, question:row.question, director_recommendation:parseJson(row.recommendation_json),
      alternatives:parseJson(row.alternatives_json,[]), impact:row.impact, requires:row.requires,
      effective_decision:parseJson(row.effective_json), decided_by:row.decided_by ?? null, status:row.status, updated_at:row.updated_at };
  }

  listDecisions({ status = null, limit = 200 } = {}) {
    const rows = status
      ? this.db.prepare('SELECT id FROM decisions WHERE status=? ORDER BY updated_at DESC LIMIT ?').all(status,limit)
      : this.db.prepare('SELECT id FROM decisions ORDER BY updated_at DESC LIMIT ?').all(limit);
    return rows.map(r => this.getDecision(r.id));
  }

  setResourceState(agentId, state) {
    const now = new Date().toISOString();
    this.db.prepare(`
      INSERT INTO resources(agent_id,status,reset_at,details_json,updated_at)
      VALUES(?,?,?,?,?)
      ON CONFLICT(agent_id) DO UPDATE SET status=excluded.status, reset_at=excluded.reset_at,
      details_json=excluded.details_json, updated_at=excluded.updated_at
    `).run(agentId, state.status, state.resetAt ?? null, JSON.stringify(state.details ?? {}), now);
    return this.getResourceState(agentId);
  }

  getResourceState(agentId) {
    const row = this.db.prepare('SELECT * FROM resources WHERE agent_id=?').get(agentId);
    if (!row) return null;
    return { agentId, status: row.status, resetAt: row.reset_at ?? null, details: parseJson(row.details_json, {}) };
  }

  listResources() {
    return this.db.prepare('SELECT agent_id FROM resources ORDER BY agent_id').all().map(r => this.getResourceState(r.agent_id));
  }
}
