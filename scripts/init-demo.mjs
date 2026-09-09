import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const tasks = path.join(root, '.batai', 'tasks');
fs.mkdirSync(tasks, { recursive: true });
const demo = {
  id:'TASK-DEMO-1', created_by:'director', objective:'Demonstrate event-driven task dispatch using the mock provider.',
  assigned_to:['agent8'], dependencies:[], acceptance_criteria:['Task reaches COMPLETED'], inputs:[], outputs:[], status:'READY',
  execution:{parallel:false,requires_director_review:false}, on_success:{notify:'director',reason:'demo_completed'}, on_failure:{notify:'director'}
};
fs.writeFileSync(path.join(tasks, `${demo.id}.json`), JSON.stringify(demo, null, 2)+'\n');
console.log(`Demo task written: ${path.join(tasks, `${demo.id}.json`)}`);
