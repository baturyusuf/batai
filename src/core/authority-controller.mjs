import fs from 'node:fs';
import path from 'node:path';
import { newId } from './ids.mjs';

function safeTimestamp(iso) { return iso.replace(/[:.]/g, '-'); }
function atomicJsonWrite(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive:true });
  const temp = `${filePath}.${process.pid}.${Date.now()}.tmp`;
  fs.writeFileSync(temp, `${JSON.stringify(value,null,2)}\n`);
  fs.renameSync(temp,filePath);
}

export class AuthorityController {
  constructor({ projectRoot, store, events }) {
    this.projectRoot = projectRoot;
    this.store = store;
    this.events = events;
  }

  submitGodMessage({ content, scope='project', target='director', metadata={} }) {
    if (!content?.trim()) throw new Error('GOD message content is required');
    const message = {
      id:newId('MSG'), source:'GOD', target, authority:100, scope,
      content:content.trim(), status:'PENDING', timestamp:new Date().toISOString(), metadata
    };
    this.store.appendAuthorityMessage(message);
    const file = path.join(this.projectRoot,'.batai','inbox',target,`${safeTimestamp(message.timestamp)}-${message.id}.json`);
    atomicJsonWrite(file,message);
    this.events.publish('GOD_MESSAGE_RECEIVED',{source:'GOD',target,payload:{message_id:message.id,scope,content:message.content}});
    return {...message,file};
  }

  listInbox(target='director', { pendingOnly=false } = {}) {
    return this.store.listAuthorityMessages({target,status:pendingOnly?'PENDING':null});
  }

  acknowledgeMessage(id) {
    const message = this.store.getAuthorityMessage(id);
    if (!message) throw new Error(`Message not found: ${id}`);
    return this.store.setAuthorityMessageStatus(id,'ACKNOWLEDGED');
  }

  createDecision(input) {
    if (!input?.topic || !input?.question) throw new Error('Decision topic and question are required');
    const decision = this.store.upsertDecision({
      id:input.id ?? newId('DEC'), topic:input.topic, question:input.question,
      director_recommendation:input.director_recommendation ?? null,
      alternatives:input.alternatives ?? [], impact:input.impact ?? 'medium', requires:input.requires ?? 'GOD',
      effective_decision:null, decided_by:null, status:'OPEN'
    });
    atomicJsonWrite(path.join(this.projectRoot,'.batai','decisions',`${decision.id}.json`),decision);
    this.events.publish('DECISION_CREATED',{source:'director',target:decision.requires,payload:{decision_id:decision.id,topic:decision.topic,impact:decision.impact}});
    if (decision.requires === 'GOD') this.events.publish('GOD_DECISION_REQUIRED',{source:'director',target:'GOD',payload:{decision_id:decision.id,question:decision.question,recommendation:decision.director_recommendation}});
    return decision;
  }

  resolveDecision(id, effectiveDecision, { decidedBy='GOD', lock=true } = {}) {
    const current = this.store.getDecision(id);
    if (!current) throw new Error(`Decision not found: ${id}`);
    if (current.status === 'LOCKED' && current.decided_by === 'GOD' && decidedBy !== 'GOD') throw new Error(`Decision ${id} is locked by GOD`);
    const decision = this.store.upsertDecision({
      ...current, effective_decision:effectiveDecision, decided_by:decidedBy,
      status:lock && decidedBy==='GOD' ? 'LOCKED' : 'APPROVED'
    });
    atomicJsonWrite(path.join(this.projectRoot,'.batai','decisions',`${decision.id}.json`),decision);
    this.events.publish('GOD_DECISION_RESOLVED',{source:decidedBy,target:'director',payload:{decision_id:id,effective_decision:effectiveDecision,status:decision.status}});
    return decision;
  }

  listDecisions(options={}) { return this.store.listDecisions(options); }
}
