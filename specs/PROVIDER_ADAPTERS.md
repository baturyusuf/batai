# Provider Adapter Specification

## 1. Amaç

Batai provider-specific detayları AgentManager'dan ayırır.

## 2. Ortak Interface

Her adapter şu yetenekleri mümkün olduğu kadar sağlar:

- authenticate()
- list_models()
- create_session()
- resume_session()
- send_message()
- stream_output()
- cancel_turn()
- get_usage_state()
- get_session_state()
- close_session()

## 3. Adapter Türleri

### ACPAdapter
ACP destekleyen external agents.

### CodexAppServerAdapter
Codex app-server / native protocol.

### CLIAdapter
Claude Code, Gemini CLI ve diğer resmi CLI'lar.

### LocalModelAdapter
Ollama / llama.cpp / vLLM.

### APIAdapter
Fallback ve resmi API kullanımı.

## 4. Auth Modes

- subscription
- oauth
- cli_session
- local
- api_key

## 5. Provider Capability Matrix

Her provider kaydı:
- supports_resume
- supports_usage_state
- supports_model_switch
- supports_reasoning_effort
- supports_subscription_auth
- supports_tool_calls
- supports_mcp
- supports_worktree
- supports_streaming

## 6. Fallback

Director agent başına fallback policy verebilir:

```json
{
  "primary": "codex_subscription",
  "fallbacks": [
    "local_qwen",
    "openai_api"
  ],
  "fallback_requires_director": true
}
```

Default:
- quota dolunca otomatik provider değiştirme YOK,
- Director açıkça izin verirse değişebilir.
