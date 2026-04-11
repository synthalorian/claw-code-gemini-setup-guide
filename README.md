# claw-code × Google Gemini (Code Assist) Setup Guide

A step-by-step recipe for wiring [`claw-code`](https://github.com/ultraworkers/claw-code) — the Rust port of Anthropic's Claude Code CLI — to Google Gemini via the **Cloud Code Assist API** using the same OAuth tokens that Google's official `gemini` CLI mints. Includes the **`thoughtSignature` replay** fix that's mandatory for Gemini 3 thinking-mode multi-turn tool calling.

> This is a guide repo, not a fork. It documents how to add Code Assist support on top of [`ultraworkers/claw-code`](https://github.com/ultraworkers/claw-code), so you can either reproduce the integration from scratch on a clean checkout or understand the design if it's already there in your tree.

---

## Why this path?

Google exposes Gemini through three different surfaces. Picking the right one matters:

| Surface | Endpoint | Auth | Cost | Notes |
|---|---|---|---|---|
| Generative Language (OpenAI-compat) | `generativelanguage.googleapis.com/v1beta/openai` | API key | Paid / free quota | Easiest, but no `thoughtSignature` replay support and tool-use is shaky |
| Vertex AI (OpenAI-compat) | `aiplatform.googleapis.com/.../openapi` | gcloud OAuth + GCP project | Billed | Requires gcloud + a billing-enabled project |
| **Cloud Code Assist** | `cloudcode-pa.googleapis.com` | gemini-cli OAuth | **Free tier** | Used by the official `gemini` CLI, auto-provisions a project, no gcloud needed |

**This guide wires claw-code to Cloud Code Assist.** It's the same path Google's own `gemini` CLI uses, gives you the free tier automatically, and is the only one that survives Gemini 3 thinking-mode tool calls intact.

---

## What you get

When you finish this guide:

- `claw` runs Gemini 3 (`gemini-3-flash-preview` by default) end-to-end with full tool calling
- OAuth tokens come from `~/.gemini/oauth_creds.json` (managed by Google's `gemini` CLI — claw-code reads + refreshes them in place)
- Multi-turn tool conversations work because the `thoughtSignature` replay cache is wired correctly
- No API key, no gcloud project, no billing account required
- Existing Anthropic / OpenAI providers in claw-code keep working — Gemini auto-routes only when `~/.gemini/oauth_creds.json` exists

---

## Prerequisites

1. **Rust toolchain** — `rustup` with stable (claw-code targets stable Rust)
2. **Google's `gemini` CLI** — install from <https://github.com/google-gemini/gemini-cli> (`npm i -g @google/gemini-cli` or your distro's package). Used **once** to mint the OAuth tokens — after that claw-code refreshes them itself.
3. **A Google account** — any, including a personal Gmail. The free Code Assist tier auto-provisions on first call.
4. **`claw-code` checked out** — `git clone https://github.com/ultraworkers/claw-code.git ~/claw-code`

---

## TL;DR (for the impatient)

```bash
# 1. Mint Google OAuth tokens via the official CLI (one-time browser flow)
gemini      # follow the device-code prompt → writes ~/.gemini/oauth_creds.json

# 2. Clone upstream claw-code
git clone https://github.com/ultraworkers/claw-code.git ~/claw-code

# 3. Apply the Code Assist integration described in this guide
#    (see "How the integration works" below — or apply from your own branch
#    if you've already done this once)

# 4. Build
cd ~/claw-code/rust
cargo build --release

# 5. Run claw — it auto-detects the gemini creds and routes to Code Assist
./target/release/claw "what time is it? use the bash tool to check"
```

That's the happy path. The rest of this README is the **how it works** + **how to build it from scratch**.

---

## Architecture overview

```
            ~/.gemini/oauth_creds.json   ←── written by Google's gemini CLI
                       │
                       │ read + refresh
                       ▼
   ┌──────────────────────────────────────┐
   │ providers/google_codeassist.rs       │
   │   • GoogleCodeAssistClient           │
   │   • Project discovery (loadCodeAssist│
   │     / onboardUser LRO)               │
   │   • OpenAI ↔ Code Assist translator  │
   │   • Process-wide thoughtSignature    │
   │     cache (OnceLock<Mutex<HashMap>>) │
   │   • Retry-on-429 w/ retryDelay parse │
   └──────────────────────────────────────┘
                       │
                       │ ProviderClient::Google variant
                       ▼
              providers/mod.rs::ProviderKind::Google
                       │
                       ▼
              client.rs::ProviderClient routing
                       │
                       │ default model: gemini-3-flash-preview
                       ▼
                  rusty-claude-cli (`claw` binary)
```

**Key invariant:** `providers/mod.rs::metadata_for_model` checks for `~/.gemini/oauth_creds.json` *first*. If it exists and the model name starts with `gemini-`, it routes to `ProviderKind::Google`. Otherwise it falls back to the OpenAI-compat path against `generativelanguage.googleapis.com`.

---

## Step-by-step setup

### Step 1 — Mint Google OAuth tokens via the `gemini` CLI

```bash
# Install Google's official gemini CLI
npm install -g @google/gemini-cli   # or pacman/brew/yay equivalent

# Run it once to trigger the OAuth flow
gemini
```

This opens your browser, you sign in, the CLI writes:

```
~/.gemini/oauth_creds.json
```

The file contains `access_token`, `refresh_token`, `expiry_date`, `token_type`, `id_token`. Refresh tokens don't expire; access tokens are good for ~59 minutes.

**Verify:**

```bash
ls -la ~/.gemini/oauth_creds.json
# -rw------- ... oauth_creds.json
```

If the file isn't `0600`, fix it: `chmod 600 ~/.gemini/oauth_creds.json`.

### Step 2 — Clone claw-code

```bash
git clone https://github.com/ultraworkers/claw-code.git ~/claw-code
cd ~/claw-code
```

The cargo workspace lives under `rust/`, NOT the repo root. If upstream hasn't merged Code Assist support yet, you'll need to apply the changes documented in **How the integration works** below — or work on a topic branch (`git checkout -b google-gemini-onboarding`) and apply them there.

### Step 3 — Build

```bash
cd ~/claw-code/rust
cargo build --release
```

Verify clean build:

```bash
cargo fmt --check
cargo clippy -p api --no-deps -- -D warnings
cargo test -p api --workspace
```

> `cargo clippy --workspace -- -D warnings` may surface pre-existing warnings in the `runtime` crate (`items_after_statements`, `redundant_pattern_matching` in `worker_boot.rs`). Lint just the `api` crate when working on Gemini changes.

### Step 4 — Run it

```bash
./target/release/claw "what time is it right now? use the bash tool to check"
```

You should see claw spawn a bash tool call, get the date, and answer in plain English. If you get this, **the integration is working end-to-end** including thoughtSignature replay.

### Step 5 — (Optional) Install the binary

```bash
cp target/release/claw ~/.local/bin/claw
# or symlink
ln -sf $(pwd)/target/release/claw ~/.local/bin/claw
```

---

## How the integration works (the load-bearing parts)

If you want to *build* this from scratch on a clean claw-code checkout, here are the files you need to add or touch.

### File: `rust/crates/api/src/providers/google_codeassist.rs` (new)

This is the entire Code Assist provider — `~1500` lines. It contains:

- `CODE_ASSIST_ENDPOINT = "https://cloudcode-pa.googleapis.com"` constant
- Embedded gemini-cli OAuth client ID/secret (NOT secret — every install ships them)
- `GoogleCodeAssistClient` — owns refresh logic + project discovery
- `GoogleCodeAssistStream` — SSE streaming consumer
- `translate_request_to_codeassist()` / `translate_response_to_openai()` — the OpenAI ↔ Code Assist envelope converter
- **The `thoughtSignature` cache** — `OnceLock<Mutex<HashMap<String, String>>>` keyed by `tool_call_id` with a `(name, canonical_args)` fallback

Key embedded constants:

```rust
const CODE_ASSIST_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const CLIENT_ID: &str = "<GOOGLE_CLIENT_ID>";
const CLIENT_SECRET: &str = "<GOOGLE_CLIENT_SECRET>";
```

These are the **same credentials every gemini-cli install ships with**. They're embedded in the binary, not secret, scoped only to the Code Assist API surface. See `docs/wire-protocol.md` in this repo for the full request/response shape.

### File: `rust/crates/api/src/providers/mod.rs` (modify)

Add the `Google` variant to `ProviderKind` and route `gemini-*` model names to it when the credentials file exists:

```rust
pub mod google_codeassist;

// inside ProviderKind enum
Google,

// inside metadata_for_model()
if model_name.starts_with("gemini-") {
    if google_codeassist_credentials_present() {
        return ProviderMetadata {
            provider: ProviderKind::Google,
            // ... rest
        };
    }
    // fall through to OpenAI-compat path
}

fn google_codeassist_credentials_present() -> bool {
    home_dir()
        .map(|h| h.join(".gemini").join("oauth_creds.json").exists())
        .unwrap_or(false)
}
```

### File: `rust/crates/api/src/client.rs` (modify)

Add a `Google` variant to the `ProviderClient` enum and the `MessageStream` enum so the runtime can dispatch to it:

```rust
pub enum ProviderClient {
    Anthropic(AnthropicClient),
    OpenAiCompat(OpenAiCompatClient),
    Google(GoogleCodeAssistClient),  // NEW
}

pub enum MessageStream {
    Anthropic(AnthropicStream),
    OpenAiCompat(OpenAiCompatStream),
    Google(GoogleCodeAssistStream),  // NEW
}
```

The `from_model_with_anthropic_auth` constructor needs a `ProviderKind::Google => GoogleCodeAssistClient::new()` arm.

### File: `rust/crates/rusty-claude-cli/src/main.rs` (modify)

Set the default model to `gemini-3-flash-preview`:

```rust
const DEFAULT_MODEL: &str = "gemini-3-flash-preview";
```

And add `"google" | "google-codeassist" => "google-codeassist"` to `provider_label`.

---

## The `thoughtSignature` gotcha (THE most important non-obvious thing)

**This is what "thinking overrides" means.** Skip this and your second tool call will 400.

Gemini 3 (and `gemini-2.5-pro` w/ thinking enabled) emits a `thoughtSignature` field whenever it thinks about a tool call. On the **next turn**, when you replay that assistant message as conversation history, you **MUST** include the original `thoughtSignature` on the same `functionCall` part. If you don't, Code Assist returns:

```
400 Bad Request: Unable to submit request because function call `X` in the N. content block
is missing a `thought_signature`. Learn more: https://...
```

The error message says "missing token" — it's actually missing thinking-mode replay verification.

### Subtleties

1. **`thoughtSignature` can appear on ANY part type**, not just `functionCall`. Per Google's docs: "For non-functionCall responses, the signature appears on the last part for context replay." Track the **latest non-empty signature seen across ALL parts** in a response and let any sibling functionCall claim it.

2. **The signature lives on the `part` itself**, NOT inside `functionCall`. So `part.thoughtSignature`, not `part.functionCall.thoughtSignature`.

3. **Per-instance state is wrong** if your runtime constructs a fresh client per request (claw-code's tool loop does this). Use **process-wide state** via `OnceLock<Mutex<HashMap<...>>>`. Hermes has the same problem and solves it the same way.

4. **OpenAI's tool_call shape has no field for `thoughtSignature`.** The cache is a side channel — capture on the response side, look up by `tool_call_id` when building the next request.

5. **Cache by both id AND payload.** Some agent layers rewrite tool_call ids between turns. Falling back to a `name::canonical-args` key catches that case.

### How claw-code does it

In `providers/google_codeassist.rs`:

```rust
// Process-wide thoughtSignature cache
static SIGNATURE_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, String>> {
    SIGNATURE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// Stream side: as parts arrive, track the latest signature across ALL parts
let mut latest_signature: Option<String> = None;
for part in parts {
    if let Some(sig) = part.get("thoughtSignature").and_then(|v| v.as_str()) {
        latest_signature = Some(sig.to_string());
    }
    // ... if part has functionCall, claim the latest signature for it
    if let Some(fn_call) = part.get("functionCall") {
        let signature = part_sig.clone().or_else(|| latest_signature.clone());
        if let Some(sig) = signature.as_deref() {
            cache().lock().unwrap().insert(tool_call_id.clone(), sig.to_string());
        }
    }
}

// Request side: when replaying assistant tool calls, look up + attach
if let Some(sig) = cache().lock().unwrap().get(&tool_call.id) {
    fn_call_part["thoughtSignature"] = json!(sig);
}
```

### Regression test

claw-code includes a regression test in `crates/api/tests/openai_compat_integration.rs`:

```
stream_carries_late_arriving_gemini_thought_signature_to_stop_event
```

This test fakes a stream where the `thoughtSignature` arrives in a delta **after** the `functionCall` name (which was the original bug — the parser used to fire `ContentBlockStart` with a `None` signature and never re-emit). The fix added `thought_signature: Option<String>` to `ContentBlockStopEvent` in `api/types.rs` so the parser can stamp the latest signature onto the stop event, and the consumer in `tools/src/lib.rs` overrides the pending tool's signature when stop carries one.

If you ever rewrite the streaming parser, **keep this test green**.

---

## Free tier project provisioning (one-time, automatic)

The first time `claw` calls Code Assist with a new account, it has to discover or create the auto-provisioned project. The provider does this transparently:

1. `POST /v1internal:loadCodeAssist` — body includes `cloudaicompanionProject` (initially null), `metadata: {ideType: "IDE_UNSPECIFIED", platform: "PLATFORM_UNSPECIFIED", pluginType: "GEMINI"}`
2. If response has `currentTier` + `cloudaicompanionProject` → use it
3. Otherwise: `POST /v1internal:onboardUser` with `{tierId: "free-tier", metadata: {...}}` → returns a long-running operation (LRO)
4. Poll LRO at `/v1internal/{operationName}` GET with 5s intervals until complete
5. Cache `projectId` in `~/.gemini/oauth_creds.json` under a `project_id` key

You'll see this happen exactly once per account; subsequent runs read the cached project id.

---

## Required headers (gotchas)

When you call Code Assist endpoints, headers matter:

```
Authorization: Bearer <ya29 access token>
Content-Type: application/json
User-Agent: google-cloud-sdk vscode_cloudshelleditor/0.1
X-Goog-Api-Client: gl-node/22.17.0
Client-Metadata: {"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}
```

Project discovery uses a slightly different `User-Agent: google-api-nodejs-client/9.15.1`. If you swap these for something custom, Google may reject the request. The values match what the official `gemini` CLI sends.

---

## Verifying the install

Run any of these:

```bash
# Simple text
claw "say hi in one sentence"

# Tool calling (the hard test — exercises thoughtSignature replay)
claw "what time is it right now? use the bash tool to check"

# Multi-turn tool calling
claw "list the files in my home directory, then count how many start with a dot"
```

If multi-turn tool calling works without a 400, you're done. The thinking-mode replay is the failure mode you're testing for.

---

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `400: missing thought_signature` | The thoughtSignature cache isn't being populated or replayed | Verify `cache()` is `OnceLock`-backed (process-wide), not per-instance. Add print statements at the cache write/read sites. |
| `401 Unauthorized` | Access token expired and refresh failed | Run `gemini` once to re-mint tokens. Check `~/.gemini/oauth_creds.json` has a `refresh_token` field. |
| `403 Permission Denied` on project discovery | Account hasn't been onboarded to Code Assist free tier | The provider should auto-call `:onboardUser`. If it didn't, try `gemini` once — Google's CLI also triggers onboarding. |
| `429 Quota Exceeded` | Free tier rate limit (~5 RPM for Flash) | The provider parses `retryDelay` from the response and sleeps. If you see retry storms, check the inner retry actually waits the full quota window. |
| `claw` keeps using OpenAI-compat path instead of Code Assist | `~/.gemini/oauth_creds.json` doesn't exist or is in the wrong place | `ls -la ~/.gemini/oauth_creds.json` — must be readable by your user. |
| Build fails with `cannot find struct GoogleCodeAssistClient` | Code Assist files haven't been added to the tree yet | Apply the integration from **How the integration works** above; verify `crates/api/src/providers/google_codeassist.rs` exists. |

---

## Repository contents

```
.
├── README.md                       ← you are here
├── LICENSE
├── docs/
│   ├── wire-protocol.md            ← Code Assist API request/response shape
│   ├── thought-signature.md        ← Deep dive on the thinking-mode replay fix
│   └── architecture.md             ← How the pieces fit together
├── examples/
│   ├── google_codeassist.rs        ← Sketch of the provider file (annotated)
│   ├── providers_mod_diff.rs       ← The mod.rs changes
│   └── client_diff.rs              ← The client.rs changes
└── scripts/
    ├── verify-install.sh           ← Checks gemini creds, runs claw smoke test
    └── refresh-token.sh            ← Standalone refresh-token helper for debugging
```

---

## Credits + references

- [`ultraworkers/claw-code`](https://github.com/ultraworkers/claw-code) — the Rust port of Claude Code that this guide targets
- Anthropic's [Claude Code](https://github.com/anthropics/claude-code) — the upstream design claw-code is based on
- Google's official [`gemini` CLI](https://github.com/google-gemini/gemini-cli) — the source of the OAuth tokens and the wire protocol reference
- Nous Research's [`pi-ai`](https://github.com/NousResearch/pi-ai) library — original reference implementation for the embedded OAuth credentials

## License

MIT — see [LICENSE](LICENSE).
