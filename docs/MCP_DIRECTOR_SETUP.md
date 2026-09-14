# Director MCP Setup

Batai exposes a thin stdio JSON-RPC/MCP bridge to the per-project Rust daemon:

```bash
BATAI_PROJECT_ROOT=/path/to/customer-project npm run mcp
```

The Director can use these tools:

- `batai_create_agent`
- `batai_assign_task`
- `batai_read_project_state`
- `batai_approve_task`
- `batai_create_worktree`
- `batai_update_directives`
- `batai_resume_agent`
- `batai_create_github_issue`
- `batai_read_director_inbox`
- `batai_acknowledge_god_message`
- `batai_request_god_decision`

A Director model connected to this control server can create/configure agents, choose provider/model/reasoning, write declarative tasks, manage worktrees, read GOD instructions and request GOD decisions without Batai using a second orchestration LLM.

Example generic configuration:

```json
{
  "mcpServers": {
    "batai": {
      "command": "/absolute/path/to/batai-control",
      "args": ["mcp"],
      "env": {
        "BATAI_PROJECT_ROOT": "/absolute/path/to/customer-project"
      }
    }
  }
}
```

Provider-specific MCP registration syntax may differ on the target machine.

The bridge attaches to an existing daemon or starts one safely. It never opens the project database or starts its own watcher/provider runtime, and stdout is reserved for MCP frames. The deprecated Node MCP file remains only as a compatibility reference.
