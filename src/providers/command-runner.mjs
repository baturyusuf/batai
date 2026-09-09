import { spawn } from 'node:child_process';

export function runCommand(command, args = [], { cwd = process.cwd(), input = null, env = process.env, timeoutMs = 30 * 60 * 1000 } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, env, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
    let stdout = '';
    let stderr = '';
    let timedOut = false;
    const timeout = setTimeout(() => {
      timedOut = true;
      child.kill('SIGTERM');
    }, timeoutMs);
    timeout.unref?.();
    child.stdout.on('data', d => { stdout += d.toString(); });
    child.stderr.on('data', d => { stderr += d.toString(); });
    child.on('error', error => { clearTimeout(timeout); reject(error); });
    child.on('close', code => {
      clearTimeout(timeout);
      if (code === 0 && !timedOut) return resolve({ code, stdout, stderr });
      const error = new Error(timedOut ? `${command} timed out` : `${command} exited with code ${code}: ${stderr.trim() || stdout.trim()}`);
      error.code = code;
      error.stdout = stdout;
      error.stderr = stderr;
      error.timedOut = timedOut;
      reject(error);
    });
    if (input != null) child.stdin.end(input);
    else child.stdin.end();
  });
}

export async function commandAvailable(command) {
  try {
    const lookup = process.platform === 'win32' ? ['where', command] : ['sh', '-lc', `command -v ${JSON.stringify(command)}`];
    await runCommand(lookup[0], lookup.slice(1), { timeoutMs: 3000 });
    return true;
  } catch { return false; }
}
