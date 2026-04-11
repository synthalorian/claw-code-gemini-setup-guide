# Architecture overview

A high-level map of how Code Assist support fits into claw-code's existing provider system.

## Crate layout (after the integration)

```
claw-code/
└── rust/
    └── crates/
        ├── api/
        │   └── src/
        │       ├── lib.rs                   ← `pub mod providers;`
        │       ├── client.rs                ← ProviderClient + MessageStream enums
        │       │                              (add Google variants here)
        │       ├── types.rs                 ← MessageRequest / ContentBlock /
        │       │                              ContentBlockStopEvent
        │       │                              (thought_signature field lives here)
        │       └── providers/
        │           ├── mod.rs               ← ProviderKind enum + metadata_for_model
        │           ├── anthropic.rs
        │           ├── openai_compat.rs
        │           └── google_codeassist.rs ← NEW — the entire integration
        ├── runtime/                         ← session, conversation, permissions
        ├── tools/                           ← tool registry + streaming consumer
        │   └── src/lib.rs                   ← consumes thought_signature from
        │                                       ContentBlockStopEvent
        └── rusty-claude-cli/                ← `claw` binary entrypoint
            └── src/main.rs                  ← DEFAULT_MODEL = "gemini-3-flash-preview"
```

## Request flow

```
                        ┌──────────────────────┐
                        │   `claw` CLI args    │
                        │  (model, prompt)     │
                        └─────────┬────────────┘
                                  │
                                  ▼
              ┌───────────────────────────────────────┐
              │ providers::metadata_for_model()       │
              │   - Strip alias                       │
              │   - If gemini-* AND oauth_creds.json  │
              │     exists → ProviderKind::Google     │
              │   - Otherwise fall through to OpenAI- │
              │     compat or Anthropic               │
              └─────────┬─────────────────────────────┘
                        │
                        ▼
              ┌───────────────────────────────────────┐
              │ ProviderClient::from_model_with_*()   │
              │   match metadata.provider:            │
              │   - Anthropic → AnthropicClient       │
              │   - OpenAiCompat → OpenAiCompatClient │
              │   - Google → GoogleCodeAssistClient   │
              └─────────┬─────────────────────────────┘
                        │
                        ▼
       ┌──────────────────────────────────────────────────┐
       │ GoogleCodeAssistClient::stream_message()         │
       │                                                  │
       │  1. refresh_if_needed()                          │
       │     - Read ~/.gemini/oauth_creds.json            │
       │     - If access_token expiring soon, POST to     │
       │       oauth2.googleapis.com/token                │
       │     - Write refreshed token back                 │
       │                                                  │
       │  2. discover_project_if_needed()                 │
       │     - First call: POST :loadCodeAssist           │
       │     - If no tier: POST :onboardUser → poll LRO   │
       │     - Cache project_id in oauth_creds.json       │
       │                                                  │
       │  3. translate_request_to_codeassist()            │
       │     - Walk conversation history                  │
       │     - For each assistant ToolUse, look up cached │
       │       thoughtSignature by tool_call_id           │
       │     - Attach to functionCall part                │
       │                                                  │
       │  4. POST /v1internal:streamGenerateContent       │
       │     - SSE response                               │
       │     - Parse each `data:` line                    │
       │     - Track latest thoughtSignature across ALL   │
       │       parts in this response                     │
       │     - On functionCall: cache the signature       │
       │     - Fire ContentBlockStart / Delta / Stop      │
       │                                                  │
       │  5. On 429: parse retryDelay, sleep, retry       │
       └─────────┬────────────────────────────────────────┘
                 │
                 ▼
       ┌──────────────────────────────────────────────────┐
       │ MessageStream::Google(GoogleCodeAssistStream)    │
       │   yielded back to the runtime as a unified       │
       │   StreamEvent stream — Anthropic / OpenAi-compat │
       │   / Google all look the same downstream.         │
       └──────────────────────────────────────────────────┘
```

## Why this architecture (and not the alternatives)

### Why NOT generativelanguage.googleapis.com (OpenAI-compat)?

Pros: trivially easy — just an OpenAI-shaped client with `base_url` swapped.

Cons:
- API key only — you have to manage a separate key
- Doesn't support thoughtSignature replay → tool calling on `gemini-3-*` is broken after the second turn
- Free quota is small + capped per project, not per account

### Why NOT Vertex AI?

Pros: full Gemini feature set, better quota.

Cons:
- Requires gcloud SDK installed
- Requires a billing-enabled GCP project
- Separate auth flow (gcloud login, application-default credentials)
- Costs money

### Why Code Assist?

Pros:
- **Free tier** auto-provisions on first call (no billing account)
- Uses the **same OAuth tokens** as Google's official `gemini` CLI (no parallel credential store)
- **Full thoughtSignature support** — multi-turn tool calling actually works
- No gcloud required

Cons:
- Wire protocol is internal-ish (`v1internal:`) — could change without notice
- The thoughtSignature replay is non-obvious and easy to get wrong (this guide exists because it's load-bearing)
- Quota is per-account and tight on Flash (~5 RPM)

For a CLI tool that's used by individual developers running tool-heavy agent loops, Code Assist is the right tradeoff.

## Where the integration's "edges" live

| Concern | Lives in |
|---|---|
| Provider routing decision | `providers/mod.rs::metadata_for_model()` |
| Provider construction | `providers/mod.rs::google_codeassist_credentials_present()` + `client.rs::ProviderClient::from_model_with_anthropic_auth()` |
| OAuth refresh | `providers/google_codeassist.rs::refresh_if_needed()` |
| Project discovery | `providers/google_codeassist.rs::discover_project_if_needed()` |
| Request translation (OpenAI → Code Assist) | `providers/google_codeassist.rs::translate_request_to_codeassist()` |
| Response translation (Code Assist → OpenAI) | `providers/google_codeassist.rs::translate_response_to_openai()` (or in the stream consumer) |
| **thoughtSignature cache** | `providers/google_codeassist.rs::SIGNATURE_CACHE` |
| Stream parser | `providers/google_codeassist.rs::GoogleCodeAssistStream` |
| Default model | `rusty-claude-cli/src/main.rs::DEFAULT_MODEL` |

If you ever need to debug an issue, the cache at `SIGNATURE_CACHE` is the first place to look.
