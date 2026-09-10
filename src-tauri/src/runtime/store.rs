use std::{
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex, MutexGuard},
};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::{
    errors::{Result, RuntimeError},
    migrations,
    types::{
        Agent, AgentStatus, IngestDisposition, ResourceState, RuntimeEvent, SchedulerJob,
        SchedulerJobStatus, Session, Task, TaskRun, TaskRunStatus, TaskStatus,
    },
};

#[derive(Clone)]
pub struct RuntimeStore {
    connection: Arc<Mutex<Connection>>,
}

impl RuntimeStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        migrations::migrate(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn open_memory() -> Result<Self> {
        let mut connection = Connection::open_in_memory()?;
        migrations::migrate(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    fn db(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| RuntimeError::Lock("sqlite connection"))
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.db()?.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        let json = serde_json::to_string(agent)?;
        self.db()?.execute(
            r#"INSERT INTO agents(id,name,provider,model,reasoning_effort,status,parent_agent_id,current_task_id,config_json,updated_at)
               VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,
               provider=excluded.provider,model=excluded.model,reasoning_effort=excluded.reasoning_effort,
               status=excluded.status,parent_agent_id=excluded.parent_agent_id,current_task_id=excluded.current_task_id,
               config_json=excluded.config_json,updated_at=excluded.updated_at"#,
            params![agent.id, agent.name, agent.provider, agent.model, agent.reasoning_effort,
                agent.status.to_string(), agent.parent_agent_id, agent.current_task_id, json, now()],
        )?;
        Ok(())
    }

    pub fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        self.db()?
            .query_row(
                "SELECT config_json,status,current_task_id FROM agents WHERE id=?",
                [id],
                |row| {
                    let mut agent: Agent = json_column(row.get::<_, String>(0)?)?;
                    agent.status = parse_column::<AgentStatus>(row.get::<_, String>(1)?)?;
                    agent.current_task_id = row.get(2)?;
                    Ok(agent)
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        let db = self.db()?;
        let mut statement = db.prepare("SELECT id FROM agents ORDER BY id")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        drop(db);
        ids.into_iter()
            .map(|id| self.get_agent(&id)?.ok_or(RuntimeError::AgentNotFound(id)))
            .collect()
    }

    pub fn set_agent_state(
        &self,
        id: &str,
        status: AgentStatus,
        current_task_id: Option<&str>,
    ) -> Result<Agent> {
        let mut agent = self
            .get_agent(id)?
            .ok_or_else(|| RuntimeError::AgentNotFound(id.into()))?;
        agent.status = status;
        agent.current_task_id = current_task_id.map(str::to_owned);
        self.upsert_agent(&agent)?;
        Ok(agent)
    }

    pub fn ingest_task(
        &self,
        task: &Task,
        source_file: Option<&Path>,
        content_hash: &str,
    ) -> Result<IngestDisposition> {
        let existing = self.task_meta(&task.id)?;
        if let Some((status, hash, _)) = &existing {
            if hash.as_deref() == Some(content_hash) {
                return Ok(IngestDisposition::Duplicate);
            }
            if matches!(
                status,
                TaskStatus::Running | TaskStatus::Completed | TaskStatus::Cancelled
            ) {
                return Ok(IngestDisposition::Duplicate);
            }
        }
        let disposition = if existing.is_some() {
            IngestDisposition::Updated
        } else {
            IngestDisposition::Created
        };
        let revision = existing.map_or(1, |(_, _, revision)| revision + 1);
        self.db()?.execute(
            r#"INSERT INTO tasks(id,status,created_by,objective,assigned_to_json,source_file,task_json,result_json,updated_at,content_hash,revision)
               VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET status=excluded.status,
               created_by=excluded.created_by,objective=excluded.objective,assigned_to_json=excluded.assigned_to_json,
               source_file=COALESCE(excluded.source_file,tasks.source_file),task_json=excluded.task_json,
               updated_at=excluded.updated_at,content_hash=excluded.content_hash,revision=excluded.revision"#,
            params![task.id, task.status.to_string(), task.created_by, task.objective,
                serde_json::to_string(&task.assigned_to)?, source_file.map(|p| p.to_string_lossy().to_string()),
                serde_json::to_string(task)?, Option::<String>::None, now(), content_hash, revision],
        )?;
        Ok(disposition)
    }

    fn task_meta(&self, id: &str) -> Result<Option<(TaskStatus, Option<String>, i64)>> {
        self.db()?
            .query_row(
                "SELECT status,content_hash,revision FROM tasks WHERE id=?",
                [id],
                |row| {
                    Ok((
                        parse_column(row.get::<_, String>(0)?)?,
                        row.get(1)?,
                        row.get(2)?,
                    ))
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        self.db()?
            .query_row(
                "SELECT task_json,status FROM tasks WHERE id=?",
                [id],
                |row| {
                    let mut task: Task = json_column(row.get::<_, String>(0)?)?;
                    task.status = parse_column::<TaskStatus>(row.get::<_, String>(1)?)?;
                    Ok(task)
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        let db = self.db()?;
        let mut statement = db.prepare("SELECT id FROM tasks ORDER BY updated_at,id")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        drop(db);
        ids.into_iter()
            .map(|id| self.get_task(&id)?.ok_or(RuntimeError::TaskNotFound(id)))
            .collect()
    }

    pub fn set_task_status(&self, id: &str, status: TaskStatus) -> Result<Task> {
        let mut task = self
            .get_task(id)?
            .ok_or_else(|| RuntimeError::TaskNotFound(id.into()))?;
        task.status = status;
        self.db()?.execute(
            "UPDATE tasks SET status=?,task_json=?,updated_at=? WHERE id=?",
            params![status.to_string(), serde_json::to_string(&task)?, now(), id],
        )?;
        Ok(task)
    }

    pub fn set_task_result(&self, id: &str, result: &Value) -> Result<()> {
        self.db()?.execute(
            "UPDATE tasks SET result_json=?,updated_at=? WHERE id=?",
            params![serde_json::to_string(result)?, now(), id],
        )?;
        Ok(())
    }

    pub fn append_event(&self, event: &RuntimeEvent) -> Result<bool> {
        Ok(self.db()?.execute(
            "INSERT OR IGNORE INTO events(id,type,timestamp,source,target,task_id,payload_json) VALUES(?,?,?,?,?,?,?)",
            params![event.id, event.event_type.to_string(), event.timestamp, event.source, event.target,
                event.task_id, serde_json::to_string(&event.payload)?],
        )? == 1)
    }

    pub fn list_events(&self, limit: usize) -> Result<Vec<RuntimeEvent>> {
        let db = self.db()?;
        let mut statement = db.prepare("SELECT id,type,timestamp,source,target,task_id,payload_json FROM events ORDER BY timestamp DESC LIMIT ?")?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(RuntimeEvent {
                    id: row.get(0)?,
                    event_type: parse_column(row.get::<_, String>(1)?)?,
                    timestamp: row.get(2)?,
                    source: row.get(3)?,
                    target: row.get(4)?,
                    task_id: row.get(5)?,
                    payload: json_column(row.get::<_, String>(6)?)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_session(&self, agent_id: &str) -> Result<Option<Session>> {
        self.db()?.query_row("SELECT provider,provider_session_id,fingerprint,metadata_json,updated_at FROM sessions WHERE agent_id=?", [agent_id], |row| Ok(Session {
            agent_id: agent_id.into(), provider: row.get(0)?, provider_session_id: row.get(1)?, fingerprint: row.get(2)?,
            metadata: json_column(row.get::<_, String>(3)?)?, updated_at: row.get(4)?,
        })).optional().map_err(Into::into)
    }

    pub fn upsert_session(&self, session: &Session) -> Result<()> {
        self.db()?.execute(r#"INSERT INTO sessions(agent_id,provider,provider_session_id,metadata_json,updated_at,fingerprint)
          VALUES(?,?,?,?,?,?) ON CONFLICT(agent_id) DO UPDATE SET provider=excluded.provider,
          provider_session_id=excluded.provider_session_id,metadata_json=excluded.metadata_json,
          updated_at=excluded.updated_at,fingerprint=excluded.fingerprint"#,
          params![session.agent_id,session.provider,session.provider_session_id,serde_json::to_string(&session.metadata)?,now(),session.fingerprint])?;
        Ok(())
    }

    pub fn delete_session(&self, agent_id: &str) -> Result<()> {
        self.db()?
            .execute("DELETE FROM sessions WHERE agent_id=?", [agent_id])?;
        Ok(())
    }

    pub fn upsert_resource(&self, state: &ResourceState) -> Result<()> {
        self.db()?.execute(r#"INSERT INTO resources(agent_id,status,reset_at,details_json,updated_at) VALUES(?,?,?,?,?)
          ON CONFLICT(agent_id) DO UPDATE SET status=excluded.status,reset_at=excluded.reset_at,
          details_json=excluded.details_json,updated_at=excluded.updated_at"#,
          params![state.agent_id,state.status.to_string(),state.reset_at,serde_json::to_string(&state.details)?,now()])?;
        Ok(())
    }

    pub fn get_resource(&self, agent_id: &str) -> Result<Option<ResourceState>> {
        self.db()?
            .query_row(
                "SELECT status,reset_at,details_json,updated_at FROM resources WHERE agent_id=?",
                [agent_id],
                |row| {
                    Ok(ResourceState {
                        agent_id: agent_id.into(),
                        status: parse_column(row.get::<_, String>(0)?)?,
                        reset_at: row.get(1)?,
                        details: json_column(row.get::<_, String>(2)?)?,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_task_run(&self, task_id: &str, agent_id: &str) -> Result<Option<TaskRun>> {
        self.db()?.query_row(r#"SELECT attempt,status,result_json,provider,provider_session_id,started_at,completed_at,updated_at
          FROM task_runs WHERE task_id=? AND agent_id=?"#, params![task_id,agent_id], |row| Ok(TaskRun {
            task_id: task_id.into(), agent_id: agent_id.into(), attempt: row.get(0)?,
            status: parse_column(row.get::<_, String>(1)?)?, result: optional_json(row.get(2)?)?, provider: row.get(3)?,
            provider_session_id: row.get(4)?, started_at: row.get(5)?, completed_at: row.get(6)?, updated_at: row.get(7)?,
        })).optional().map_err(Into::into)
    }

    pub fn list_task_runs(&self, task_id: &str) -> Result<Vec<TaskRun>> {
        let db = self.db()?;
        let mut statement =
            db.prepare("SELECT agent_id FROM task_runs WHERE task_id=? ORDER BY agent_id")?;
        let ids = statement
            .query_map([task_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        drop(db);
        ids.into_iter()
            .map(|agent| {
                self.get_task_run(task_id, &agent)?
                    .ok_or_else(|| RuntimeError::Provider("missing task run".into()))
            })
            .collect()
    }

    pub fn mark_task_run(
        &self,
        task_id: &str,
        agent_id: &str,
        status: TaskRunStatus,
        result: Option<&Value>,
        provider: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<TaskRun> {
        let previous = self.get_task_run(task_id, agent_id)?;
        let attempt = previous.as_ref().map_or(1, |run| {
            if status == TaskRunStatus::Running {
                run.attempt + 1
            } else {
                run.attempt
            }
        });
        let timestamp = now();
        let started = if status == TaskRunStatus::Running {
            Some(timestamp.clone())
        } else {
            previous.as_ref().and_then(|r| r.started_at.clone())
        };
        let completed = if status == TaskRunStatus::Completed {
            Some(timestamp.clone())
        } else {
            None
        };
        self.db()?.execute(r#"INSERT INTO task_runs(task_id,agent_id,attempt,status,result_json,provider,provider_session_id,started_at,completed_at,updated_at)
          VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(task_id,agent_id) DO UPDATE SET attempt=excluded.attempt,
          status=excluded.status,result_json=COALESCE(excluded.result_json,task_runs.result_json),provider=COALESCE(excluded.provider,task_runs.provider),
          provider_session_id=COALESCE(excluded.provider_session_id,task_runs.provider_session_id),started_at=COALESCE(excluded.started_at,task_runs.started_at),
          completed_at=excluded.completed_at,updated_at=excluded.updated_at"#,
          params![task_id,agent_id,attempt,status.to_string(),result.map(serde_json::to_string).transpose()?,provider,session_id,started,completed,timestamp])?;
        self.get_task_run(task_id, agent_id)?
            .ok_or_else(|| RuntimeError::Provider("task run write failed".into()))
    }

    pub fn schedule_resource_recheck(
        &self,
        agent_id: &str,
        run_at: &str,
        payload: &Value,
    ) -> Result<SchedulerJob> {
        if let Some(job) = self.pending_resource_job(agent_id)? {
            self.db()?.execute(
                "UPDATE scheduler_jobs SET run_at=?,payload_json=?,updated_at=? WHERE id=?",
                params![run_at, serde_json::to_string(payload)?, now(), job.id],
            )?;
            return self
                .get_job(&job.id)?
                .ok_or_else(|| RuntimeError::Provider("scheduler update failed".into()));
        }
        let job = SchedulerJob {
            id: format!("JOB-{}", uuid::Uuid::new_v4()),
            kind: "RESOURCE_RECHECK".into(),
            agent_id: Some(agent_id.into()),
            run_at: run_at.into(),
            payload: payload.clone(),
            status: SchedulerJobStatus::Pending,
            attempts: 0,
            updated_at: now(),
        };
        self.db()?.execute("INSERT INTO scheduler_jobs(id,kind,run_at,payload_json,status,updated_at,agent_id,attempts) VALUES(?,?,?,?,?,?,?,?)",
            params![job.id,job.kind,job.run_at,serde_json::to_string(&job.payload)?,job.status.to_string(),job.updated_at,job.agent_id,job.attempts])?;
        Ok(job)
    }

    pub fn pending_resource_job(&self, agent_id: &str) -> Result<Option<SchedulerJob>> {
        let id: Option<String> = self.db()?.query_row("SELECT id FROM scheduler_jobs WHERE kind='RESOURCE_RECHECK' AND agent_id=? AND status IN ('PENDING','RUNNING') LIMIT 1", [agent_id], |row| row.get(0)).optional()?;
        id.map(|id| self.get_job(&id))
            .transpose()
            .map(Option::flatten)
    }

    pub fn get_job(&self, id: &str) -> Result<Option<SchedulerJob>> {
        self.db()?.query_row("SELECT kind,agent_id,run_at,payload_json,status,attempts,updated_at FROM scheduler_jobs WHERE id=?", [id], |row| Ok(SchedulerJob {
            id:id.into(),kind:row.get(0)?,agent_id:row.get(1)?,run_at:row.get(2)?,payload:json_column(row.get::<_,String>(3)?)?,
            status:parse_column(row.get::<_,String>(4)?)?,attempts:row.get(5)?,updated_at:row.get(6)?,
        })).optional().map_err(Into::into)
    }

    pub fn due_jobs(&self, at: &str) -> Result<Vec<SchedulerJob>> {
        let cutoff = chrono::DateTime::parse_from_rfc3339(at).map_err(|error| {
            RuntimeError::Provider(format!("invalid scheduler cutoff: {error}"))
        })?;
        let db = self.db()?;
        let mut statement = db.prepare(
            "SELECT id,run_at FROM scheduler_jobs WHERE status='PENDING' ORDER BY run_at",
        )?;
        let ids = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        drop(db);
        ids.into_iter()
            .filter_map(|(id, run_at)| {
                let due = chrono::DateTime::parse_from_rfc3339(&run_at)
                    .map(|value| value <= cutoff)
                    .unwrap_or(true);
                due.then_some(id)
            })
            .map(|id| {
                self.get_job(&id)?
                    .ok_or_else(|| RuntimeError::Provider("missing scheduler job".into()))
            })
            .collect()
    }

    pub fn set_job_status(
        &self,
        id: &str,
        status: SchedulerJobStatus,
        run_at: Option<&str>,
    ) -> Result<()> {
        self.db()?.execute("UPDATE scheduler_jobs SET status=?,run_at=COALESCE(?,run_at),attempts=attempts+1,updated_at=? WHERE id=?",
            params![status.to_string(),run_at,now(),id])?;
        Ok(())
    }

    pub fn reconcile_interrupted(&self) -> Result<()> {
        let timestamp = now();
        self.db()?.execute(
            "UPDATE scheduler_jobs SET status='PENDING',updated_at=? WHERE status='RUNNING'",
            [&timestamp],
        )?;
        self.db()?.execute(
            "UPDATE tasks SET status='READY',updated_at=? WHERE status='RUNNING'",
            [&timestamp],
        )?;
        self.db()?.execute(
            "UPDATE task_runs SET status='PENDING',updated_at=? WHERE status='RUNNING'",
            [&timestamp],
        )?;
        self.db()?.execute("UPDATE agents SET status='READY',current_task_id=NULL,updated_at=? WHERE status='RUNNING'", [&timestamp])?;
        Ok(())
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_column<T: FromStr>(value: String) -> rusqlite::Result<T>
where
    T::Err: std::fmt::Display,
{
    value.parse::<T>().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            value.len(),
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.to_string(),
            )),
        )
    })
}

fn json_column<T: serde::de::DeserializeOwned>(value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            value.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn optional_json(value: Option<String>) -> rusqlite::Result<Option<Value>> {
    value.map(json_column).transpose()
}
