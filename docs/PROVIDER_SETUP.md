# Provider Setup

Batai's adapter layer separates provider authentication from orchestration.

## Codex CLI

Expected provider id: `codex-cli`

1. Install the official Codex CLI.
2. Authenticate with `codex login` (ChatGPT OAuth is supported by Codex CLI).
3. Confirm with `codex login status`.
4. Configure a Batai agent with provider `codex-cli`, a model available in the installed Codex version, and the desired reasoning effort.

Batai uses a repository/worktree as the process cwd so `codex exec resume --last` remains scoped to that agent workspace.

## Claude Code

Expected provider id: `claude-cli`

1. Install Claude Code.
2. Authenticate with `claude auth login`.
3. Confirm with `claude auth status`.
4. Set model to an available Claude alias/id and choose reasoning effort.

Batai uses named Claude sessions (`batai-<agent-id>`) and resumes them by name.

## Ollama

Expected provider id: `ollama`

1. Start Ollama locally.
2. Pull the desired model.
3. Set the agent provider to `ollama` and model to the installed model name.

Default endpoint: `http://127.0.0.1:11434`.

## Mock

Provider id: `mock`. Used for deterministic integration tests and UI demos without consuming tokens.
