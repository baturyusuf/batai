export class ProviderAdapter {
  constructor(name) {
    this.name = name;
  }
  async authenticate() { return { ok: true }; }
  async listModels() { return []; }
  async createSession() { throw new Error('createSession not implemented'); }
  async resumeSession() { throw new Error('resumeSession not implemented'); }
  async sendTask() { throw new Error('sendTask not implemented'); }
  async cancelTurn() { return false; }
  async getUsageState() { return { status: 'UNKNOWN' }; }
  async closeSession() { return true; }
}
