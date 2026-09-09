import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { OllamaAdapter } from '../src/providers/ollama-adapter.mjs';

function listen(server) {
  return new Promise(resolve => server.listen(0, '127.0.0.1', () => resolve(server.address())));
}

test('OllamaAdapter lists models and sends a chat task', async () => {
  const server = http.createServer(async (req,res) => {
    if (req.url === '/api/tags') {
      res.setHeader('content-type','application/json'); return res.end(JSON.stringify({models:[{name:'qwen3:14b'}]}));
    }
    if (req.url === '/api/chat') {
      const chunks=[]; for await (const c of req) chunks.push(c);
      const body=JSON.parse(Buffer.concat(chunks));
      assert.equal(body.model,'qwen3:14b');
      res.setHeader('content-type','application/json'); return res.end(JSON.stringify({message:{role:'assistant',content:'done'}}));
    }
    res.statusCode=404; res.end();
  });
  const address = await listen(server);
  const adapter = new OllamaAdapter({ baseUrl:`http://127.0.0.1:${address.port}` });
  assert.deepEqual(await adapter.listModels(), ['qwen3:14b']);
  const {sessionId}=await adapter.createSession({id:'local'});
  const result=await adapter.sendTask({sessionId,agent:{id:'local',model:'qwen3:14b'},task:{id:'T',objective:'test',acceptance_criteria:['done']}});
  assert.equal(result.output.content,'done');
  await new Promise(resolve => server.close(resolve));
});
