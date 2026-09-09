import { AGENT_STATUSES, TASK_STATUSES } from './constants.mjs';
import { ValidationError } from './errors.mjs';

function isNonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}

export function validateAgentConfig(agent) {
  const errors = [];
  if (!agent || typeof agent !== 'object' || Array.isArray(agent)) errors.push('agent must be an object');
  if (!isNonEmptyString(agent?.id)) errors.push('id is required');
  if (!isNonEmptyString(agent?.name)) errors.push('name is required');
  if (!isNonEmptyString(agent?.role_template)) errors.push('role_template is required');
  if (!isNonEmptyString(agent?.provider)) errors.push('provider is required');
  if (!isNonEmptyString(agent?.model)) errors.push('model is required');
  if (!['none', 'low', 'medium', 'high', 'extra_high'].includes(agent?.reasoning_effort)) {
    errors.push('reasoning_effort is invalid');
  }
  if (!AGENT_STATUSES.includes(agent?.status)) errors.push('status is invalid');
  if (agent?.max_turns != null && (!Number.isInteger(agent.max_turns) || agent.max_turns < 1)) {
    errors.push('max_turns must be a positive integer');
  }
  if (errors.length) throw new ValidationError('Invalid AgentConfig', errors);
  return agent;
}

export function validateTask(task) {
  const errors = [];
  if (!task || typeof task !== 'object' || Array.isArray(task)) errors.push('task must be an object');
  if (!isNonEmptyString(task?.id)) errors.push('id is required');
  if (!isNonEmptyString(task?.created_by)) errors.push('created_by is required');
  if (!isNonEmptyString(task?.objective)) errors.push('objective is required');
  if (!Array.isArray(task?.assigned_to) || task.assigned_to.length === 0 || task.assigned_to.some(v => !isNonEmptyString(v))) {
    errors.push('assigned_to must be a non-empty string array');
  }
  if (!TASK_STATUSES.includes(task?.status)) errors.push('status is invalid');
  for (const key of ['dependencies', 'acceptance_criteria', 'inputs', 'outputs']) {
    if (task?.[key] != null && !Array.isArray(task[key])) errors.push(`${key} must be an array`);
  }
  if (errors.length) throw new ValidationError('Invalid Task', errors);
  return task;
}

export function validateEvent(event) {
  const errors = [];
  if (!isNonEmptyString(event?.id)) errors.push('id is required');
  if (!isNonEmptyString(event?.type)) errors.push('type is required');
  if (!isNonEmptyString(event?.timestamp)) errors.push('timestamp is required');
  if (!isNonEmptyString(event?.source)) errors.push('source is required');
  if (Number.isNaN(Date.parse(event?.timestamp))) errors.push('timestamp must be ISO-8601');
  if (errors.length) throw new ValidationError('Invalid Event', errors);
  return event;
}
