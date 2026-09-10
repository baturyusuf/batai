use super::{
    errors::{Result, RuntimeError},
    events::EventEngine,
    store::RuntimeStore,
    types::{Agent, AgentStatus, EventType},
};

#[derive(Clone)]
pub struct AgentRegistry {
    store: RuntimeStore,
    events: EventEngine,
}

impl AgentRegistry {
    pub fn new(store: RuntimeStore, events: EventEngine) -> Self {
        Self { store, events }
    }
    pub fn register(&self, agent: &Agent) -> Result<()> {
        self.store.upsert_agent(agent)
    }
    pub fn get(&self, id: &str) -> Result<Option<Agent>> {
        self.store.get_agent(id)
    }
    pub fn list(&self) -> Result<Vec<Agent>> {
        self.store.list_agents()
    }

    pub fn transition(
        &self,
        id: &str,
        to: AgentStatus,
        current_task_id: Option<&str>,
    ) -> Result<Agent> {
        let agent = self
            .store
            .get_agent(id)?
            .ok_or_else(|| RuntimeError::AgentNotFound(id.into()))?;
        if agent.status == to {
            return Ok(agent);
        }
        if !allowed(agent.status, to) {
            return Err(RuntimeError::InvalidAgentTransition {
                from: agent.status,
                to,
            });
        }
        let updated = self.store.set_agent_state(id, to, current_task_id)?;
        self.events.publish(
            EventType::AgentStatusChanged,
            id,
            None,
            current_task_id.map(str::to_owned),
            serde_json::json!({"from":agent.status,"to":to,"currentTaskId":current_task_id}),
        )?;
        Ok(updated)
    }
}

fn allowed(from: AgentStatus, to: AgentStatus) -> bool {
    use AgentStatus::*;
    match from {
        Created => matches!(to, Initializing | Terminated),
        Initializing => matches!(to, Ready | Failed | Terminated),
        Ready => matches!(to, Running | Paused | Terminated),
        Running => matches!(
            to,
            Blocked | WaitingResource | Paused | Completed | Failed | Ready | Terminated
        ),
        Blocked => matches!(to, Ready | Running | Paused | Failed | Terminated),
        WaitingResource => matches!(to, Ready | Running | Paused | Failed | Terminated),
        Paused => matches!(to, Ready | Running | Terminated),
        Completed => matches!(to, Ready | Terminated),
        Failed => matches!(to, Ready | Running | Terminated),
        Terminated => false,
    }
}
