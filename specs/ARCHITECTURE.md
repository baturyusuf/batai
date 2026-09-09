# Batai Technical Architecture

## 1. Önerilen teknoloji yığını

### Desktop
- **Tauri 2**
- Frontend: React + TypeScript
- Editor: Monaco Editor
- Terminal: xterm.js

### Core Runtime
- Rust (Tauri backend)
- SQLite
- notify crate / filesystem watcher
- tokio async runtime

### Git
- native `git` CLI
- worktree management
- GitHub CLI (`gh`) adapter

### Provider Integration
- ACP adapter
- Codex app-server adapter
- CLI adapter
- Local model adapter
- API fallback adapter

### Local Models
- Ollama
- llama.cpp / vLLM optional

## 2. Ana katmanlar

```text
BATAI DESKTOP
│
├── UI Layer
│   ├── Project Explorer
│   ├── Agent Organization View
│   ├── Chat Panels
│   ├── Editor
│   ├── Terminal
│   ├── Git Diff
│   ├── Task Board
│   └── Resource/Quota Dashboard
│
├── Control Plane
│   ├── ProjectManager
│   ├── DirectorController
│   ├── AgentManager
│   ├── TaskEngine
│   ├── EventEngine
│   ├── Scheduler
│   ├── ResourceManager
│   ├── MeetingManager
│   ├── PolicyEngine
│   ├── AuthorityRouter
│   ├── GitManager
│   └── SessionManager
│
├── State Layer
│   ├── SQLite runtime.db
│   └── Repository .batai/*
│
└── Adapter Layer
    ├── ACPAdapter
    ├── CodexAppServerAdapter
    ├── ClaudeCLIAdapter
    ├── GenericCLIAdapter
    ├── LocalModelAdapter
    └── APIAdapter
```

## 3. Runtime prensibi

Batai'nin kendisi LLM değildir.

Batai:
- mesaj taşır,
- dosya değişikliklerini izler,
- task dependency çözer,
- session açar/kapatır,
- worktree yaratır,
- provider quota durumunu izler,
- scheduling yapar,
- event üretir,
- agentları resume eder.

AI reasoning yalnız Director ve uzman agentlarda gerçekleşir.

## 4. State ayrımı

### Repository State
Audit edilebilir, commitlenebilir:
- `.batai/organization.json`
- `.batai/tasks/*.json`
- `.batai/agents/*/ROLE.md`
- `.batai/agents/*/DIRECTIVES.md`
- `.batai/decisions/*`
- `.batai/reports/*`

### Runtime State
Git'ten ayrı:
- sessions
- process ids
- locks
- transient events
- quota state
- retry counters
- heartbeat
- scheduler jobs

SQLite: `~/.batai/projects/<project-id>/runtime.db`
