import { EventEmitter } from 'node:events';
import { newId } from './ids.mjs';
import { validateEvent } from './validation.mjs';

export class EventEngine extends EventEmitter {
  constructor(store) {
    super();
    this.store = store;
  }

  publish(type, { source = 'batai', target = null, taskId = null, payload = {} } = {}) {
    const event = validateEvent({
      id: newId('EVT'),
      type,
      timestamp: new Date().toISOString(),
      source,
      target,
      task_id: taskId,
      payload
    });
    this.store.appendEvent(event);
    this.emit(type, event);
    this.emit('*', event);
    return event;
  }
}
