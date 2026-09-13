use std::{sync::Arc, time::Duration};

use tokio::{sync::watch, task::JoinHandle};

use super::{
    agents::AgentRegistry,
    delivery::{CiState, DeliveryService},
    errors::Result,
    events::EventEngine,
    sessions::SessionManager,
    store::RuntimeStore,
    tasks::TaskEngine,
    types::{EventType, ResourceState, ResourceStatus, SchedulerJobStatus},
};

pub struct DurableScheduler {
    stop: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}

impl DurableScheduler {
    pub fn start(
        store: RuntimeStore,
        events: EventEngine,
        agents: AgentRegistry,
        sessions: SessionManager,
        tasks: Arc<TaskEngine>,
        interval: Duration,
        unknown_retry: Duration,
    ) -> Self {
        Self::start_with_delivery(
            store,
            events,
            agents,
            sessions,
            tasks,
            None,
            interval,
            unknown_retry,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_with_delivery(
        store: RuntimeStore,
        events: EventEngine,
        agents: AgentRegistry,
        sessions: SessionManager,
        tasks: Arc<TaskEngine>,
        delivery: Option<DeliveryService>,
        interval: Duration,
        unknown_retry: Duration,
    ) -> Self {
        let (stop, mut receiver) = watch::channel(false);
        let worker = tokio::spawn(async move {
            loop {
                if *receiver.borrow() {
                    break;
                }
                let due = store
                    .due_jobs(&chrono::Utc::now().to_rfc3339())
                    .unwrap_or_default();
                for job in due {
                    if job.kind == "GITHUB_CHECK" {
                        let Some(delivery) = &delivery else {
                            let _ = store.set_job_status(&job.id, SchedulerJobStatus::Failed, None);
                            continue;
                        };
                        let Some(task_id) = job
                            .payload
                            .get("taskId")
                            .and_then(serde_json::Value::as_str)
                        else {
                            let _ = store.set_job_status(&job.id, SchedulerJobStatus::Failed, None);
                            continue;
                        };
                        if store
                            .set_job_status(&job.id, SchedulerJobStatus::Running, None)
                            .is_err()
                        {
                            continue;
                        }
                        match delivery.sync(task_id) {
                            Ok(checkpoint)
                                if checkpoint
                                    .pull_request
                                    .as_ref()
                                    .is_some_and(|pr| pr.ci_state == CiState::Pending) =>
                            {
                                let retry = (chrono::Utc::now() + chrono::Duration::minutes(2))
                                    .to_rfc3339();
                                let _ = store.set_job_status(
                                    &job.id,
                                    SchedulerJobStatus::Pending,
                                    Some(&retry),
                                );
                            }
                            Ok(_) => {
                                let _ = store.set_job_status(
                                    &job.id,
                                    SchedulerJobStatus::Completed,
                                    None,
                                );
                            }
                            Err(_) => {
                                let retry = (chrono::Utc::now() + chrono::Duration::minutes(5))
                                    .to_rfc3339();
                                let _ = store.set_job_status(
                                    &job.id,
                                    SchedulerJobStatus::Pending,
                                    Some(&retry),
                                );
                            }
                        }
                        continue;
                    }
                    let Some(agent_id) = job.agent_id.clone() else {
                        let _ = store.set_job_status(&job.id, SchedulerJobStatus::Failed, None);
                        continue;
                    };
                    if store
                        .set_job_status(&job.id, SchedulerJobStatus::Running, None)
                        .is_err()
                    {
                        continue;
                    }
                    let outcome = async {
                        let agent = agents.get(&agent_id)?.ok_or_else(|| {
                            super::errors::RuntimeError::AgentNotFound(agent_id.clone())
                        })?;
                        let usage = sessions
                            .provider_for(&agent)?
                            .get_usage_state(&agent)
                            .await?;
                        store.upsert_resource(&ResourceState {
                            agent_id: agent_id.clone(),
                            status: usage.status,
                            reset_at: usage.reset_at.clone(),
                            details: usage.details.clone(),
                            updated_at: chrono::Utc::now().to_rfc3339(),
                        })?;
                        events.publish(
                            EventType::ResourceStatusChanged,
                            agent_id.clone(),
                            None,
                            None,
                            serde_json::json!({"status":usage.status,"reset_at":usage.reset_at}),
                        )?;
                        if usage.status == ResourceStatus::Available {
                            tasks.resume_agent(&agent_id).await?;
                            store.set_job_status(&job.id, SchedulerJobStatus::Completed, None)?;
                        } else {
                            let retry_at = usage.reset_at.unwrap_or_else(|| {
                                (chrono::Utc::now()
                                    + chrono::Duration::from_std(unknown_retry)
                                        .unwrap_or(chrono::Duration::minutes(15)))
                                .to_rfc3339()
                            });
                            store.set_job_status(
                                &job.id,
                                SchedulerJobStatus::Pending,
                                Some(&retry_at),
                            )?;
                        }
                        Result::<()>::Ok(())
                    }
                    .await;
                    if outcome.is_err() {
                        let retry = (chrono::Utc::now()
                            + chrono::Duration::from_std(unknown_retry)
                                .unwrap_or(chrono::Duration::minutes(15)))
                        .to_rfc3339();
                        let _ = store.set_job_status(
                            &job.id,
                            SchedulerJobStatus::Pending,
                            Some(&retry),
                        );
                    }
                }
                tokio::select! {
                    _=tokio::time::sleep(interval)=>{},
                    changed=receiver.changed()=>{ if changed.is_err()||*receiver.borrow(){break;} }
                }
            }
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }

    pub async fn shutdown(&mut self) {
        let _ = self.stop.send(true);
        if let Some(worker) = self.worker.take() {
            let _ = worker.await;
        }
    }
}

impl Drop for DurableScheduler {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}
