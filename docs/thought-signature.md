# The `thoughtSignature` Replay Fix

This is the **single most important non-obvious thing** about the Code Assist API. Skip the fix and your second tool call will return:

```
400 Bad Request: Unable to submit request because function call `X` in the N. content block
is missing a `thought_signature`. Learn more: https://...
```

The error message says "missing token" — it's actually about thinking-mode replay verification.

## The bug

Gemini 3 (and `gemini-2.5-pro` w/ thinking enabled) emits a `thoughtSignature` field whenever it produces a `functionCall` part. On the **next turn**, when you replay that assistant message as conversation history, you **MUST** include the original `thoughtSignature` on the same `functionCall` part. Google verifies it server-side as an integrity check that the conversation history hasn't been tampered with.

If you're translating between OpenAI's tool-call shape (which has no signature field) and Code Assist's content-part shape, the signature gets dropped on the floor unless you cache it as a side channel.

## Five subtleties that bit us

### 1. The signature can appear on ANY part type

Per Google's documentation: *"For non-functionCall responses, the signature appears on the last part for context replay."*

So a response might look like:

```json
{
  "parts": [
    {"text": "Let me check that for you."},
    {"thought": true, "text": "I should call the bash tool..."},
    {"functionCall": {"name": "bash", "args": {"cmd": "ls"}}},
    {"thoughtSignature": "abc123..."}
  ]
}
```

The signature is on the **fourth** part, but it belongs to the **third** part's functionCall. **Track the latest non-empty signature seen across ALL parts in a response**, and let any sibling functionCall claim it.

### 2. The signature lives on the part itself

Not inside `functionCall`:

```
✓ part.thoughtSignature
✗ part.functionCall.thoughtSignature
```

### 3. Per-instance state breaks the cache

If your runtime constructs a fresh client per request (claw-code's tool loop does this; hermes' `_create_request_openai_client(shared=False)` does this), then a per-instance `HashMap<String, String>` will be **empty on every replay**.

Use **process-wide state**:

```rust
// Rust
static SIGNATURE_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, String>> {
    SIGNATURE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
```

```python
# Python
_SIGNATURE_CACHE: dict[str, str] = {}  # module-level
_SIGNATURE_CACHE_LOCK = threading.Lock()
```

### 4. OpenAI's tool_call shape has no field for signatures

The cache must be a side channel:

- **Capture** on the response side, keyed by `tool_call_id`
- **Look up** by `tool_call_id` when building the next request body

### 5. Cache by both id AND payload

Some agent layers (LangChain, custom runtimes) rewrite `tool_call_id`s between turns. To survive that, cache under **both** the id AND a canonical `(name, args)` payload key:

```python
cache[tool_call_id] = signature
cache[f"{name}::{canonical_args_json}"] = signature
```

When replaying, look up id first, then fall back to the payload key.

## How claw-code implements it

Source: `rust/crates/api/src/providers/google_codeassist.rs`

```rust
// Process-wide thoughtSignature cache (Gemini 3 thinking-mode replay)
//
// Gemini 3 thinking-mode requires every assistant tool call to carry its
// original `thoughtSignature` when replayed in subsequent turns.  The
// MessageRequest type already has a `thought_signature` field on
// AssistantContentBlock::ToolUse, but the runtime side may construct fresh
// clients per request — so we cache process-wide.
//
//   1. On the response side: capture `thoughtSignature` from each part.
//      The signature can appear on ANY part type, so track the latest
//      non-empty signature across the whole response.
//   2. On the request side: when emitting a replay of an assistant tool
//      call, look up the cached signature by tool_call_id and attach it
//      to the functionCall part.
static SIGNATURE_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
```

The streaming parser tracks `latest_signature` and stamps it onto every functionCall:

```rust
// Track latest thoughtSignature across ALL parts in this candidate.
let mut latest_signature: Option<String> = None;
for part in parts {
    let part_sig = part
        .get("thoughtSignature")
        .and_then(|v| v.as_str())
        .map(String::from);
    if part_sig.is_some() {
        latest_signature = part_sig.clone();
    }
    // ...handle the part (text, thought, functionCall)...
    let thought_signature = part_sig.clone().or_else(|| latest_signature.clone());
    if let Some(sig) = thought_signature.as_deref() {
        cache().lock().unwrap().insert(tool_call_id.clone(), sig.to_string());
    }
}
```

The request translator looks up + attaches when replaying:

```rust
InputContentBlock::ToolUse { id, name, input, thought_signature } => {
    let mut fn_call = json!({
        "functionCall": {
            "name": name,
            "args": input,
        }
    });
    let sig: Option<String> = thought_signature
        .clone()
        .or_else(|| cache().lock().unwrap().get(id).cloned());
    if let Some(sig) = sig {
        fn_call["thoughtSignature"] = json!(sig);
    }
    parts.push(fn_call);
}
```

## How hermes implements it

Source: `~/.hermes/hermes-agent/agent/google_codeassist_protocol.py` (Python equivalent)

Same idea — module-level dict keyed by `tool_call_id` with a `(name, canonical_args)` payload fallback. The hermes implementation has additional `[CA-DEBUG]` print statements (commented out in production) at the cache write/read sites that are useful when debugging signature drops.

## Regression test

claw-code has a regression test in `crates/api/tests/openai_compat_integration.rs`:

```
stream_carries_late_arriving_gemini_thought_signature_to_stop_event
```

This test fakes a stream where the `thoughtSignature` arrives in a delta **after** the `functionCall` name (which was the original bug — the parser used to fire `ContentBlockStart` with a `None` signature and never re-emit). The fix added `thought_signature: Option<String>` to `ContentBlockStopEvent` in `api/types.rs` so the parser can stamp the latest signature onto the stop event, and the consumer in `tools/src/lib.rs` overrides the pending tool's signature when stop carries one.

If you ever rewrite the streaming parser, **keep this test green**.

## Debugging checklist

When you get a `400: missing thought_signature`:

1. **Confirm the cache is process-wide**, not per-instance
2. Add print statements at the cache **write** site (response handling) — does the signature actually get captured?
3. Add print statements at the cache **read** site (request building) — does the lookup succeed?
4. Check the tool_call_id matches between turns — some agent layers mutate it
5. If id mismatch, fall back to a `(name, canonical_args)` payload key
6. Verify your stream parser tracks signatures across **all** part types, not just `functionCall` parts
