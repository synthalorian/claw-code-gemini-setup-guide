// =============================================================================
// rust/crates/api/src/providers/google_codeassist.rs — annotated sketch
// =============================================================================
//
// This is a SKETCH of the provider file — not a complete implementation.
// The full file in claw-code is ~1500 lines and handles SSE parsing, project
// discovery, retry-on-429, request/response translation, and the
// thoughtSignature cache.  This sketch shows the load-bearing scaffolding
// you need to wire up first.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const CODE_ASSIST_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

// Embedded gemini-cli OAuth credentials (NOT secret — every gemini-cli install
// ships these.  Scoped to the Code Assist API surface only.)
const CLIENT_ID: &str = "<GOOGLE_CLIENT_ID>";
const CLIENT_SECRET: &str = "<GOOGLE_CLIENT_SECRET>";

// Refresh access tokens this many seconds before they expire
const REFRESH_SKEW_SECS: u64 = 120;

// ---------------------------------------------------------------------------
// Saved-credential file (~/.gemini/oauth_creds.json)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeminiCliCreds {
    pub access_token: String,
    pub refresh_token: String,
    pub expiry_date: i64,           // milliseconds since epoch
    pub token_type: String,
    pub scope: Option<String>,
    pub id_token: Option<String>,
    /// Cached project id (added by us, not by the gemini CLI)
    pub project_id: Option<String>,
}

fn creds_path() -> PathBuf {
    home::home_dir()
        .expect("HOME not set")
        .join(".gemini")
        .join("oauth_creds.json")
}

fn read_creds() -> std::io::Result<GeminiCliCreds> {
    let bytes = std::fs::read(creds_path())?;
    Ok(serde_json::from_slice(&bytes).expect("invalid oauth_creds.json"))
}

fn write_creds(creds: &GeminiCliCreds) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(creds).unwrap();
    std::fs::write(creds_path(), bytes)?;
    // chmod 600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(creds_path(), std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Process-wide thoughtSignature cache (THE thinking-mode replay fix)
// ---------------------------------------------------------------------------

static SIGNATURE_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, String>> {
    SIGNATURE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Capture a thoughtSignature seen in a response, keyed by tool_call_id.
pub fn capture_signature(tool_call_id: &str, signature: &str) {
    cache()
        .lock()
        .unwrap()
        .insert(tool_call_id.to_string(), signature.to_string());
}

/// Look up a cached thoughtSignature when replaying an assistant tool call.
pub fn lookup_signature(tool_call_id: &str) -> Option<String> {
    cache().lock().unwrap().get(tool_call_id).cloned()
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct GoogleCodeAssistClient {
    http: reqwest::Client,
    creds: Mutex<GeminiCliCreds>,
}

impl GoogleCodeAssistClient {
    pub fn new() -> Result<Self, String> {
        let creds = read_creds().map_err(|e| format!("read oauth_creds.json: {e}"))?;
        Ok(Self {
            http: reqwest::Client::new(),
            creds: Mutex::new(creds),
        })
    }

    /// Refresh the access token if it's within the skew window.
    async fn refresh_if_needed(&self) -> Result<String, String> {
        let mut creds = self.creds.lock().unwrap().clone();

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        if creds.expiry_date - now_ms > (REFRESH_SKEW_SECS * 1000) as i64 {
            return Ok(creds.access_token);
        }

        // Refresh
        let resp = self
            .http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &creds.refresh_token),
                ("client_id", CLIENT_ID),
                ("client_secret", CLIENT_SECRET),
            ])
            .send()
            .await
            .map_err(|e| format!("refresh request failed: {e}"))?;

        let body: Value = resp.json().await.map_err(|e| format!("refresh json: {e}"))?;
        let new_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("no access_token in refresh response: {body}"))?
            .to_string();
        let expires_in = body.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(3599);

        creds.access_token = new_token.clone();
        creds.expiry_date = now_ms + expires_in * 1000;

        // Persist + update in-memory copy
        write_creds(&creds).map_err(|e| format!("write creds: {e}"))?;
        *self.creds.lock().unwrap() = creds;

        Ok(new_token)
    }

    /// Required headers for Code Assist requests
    fn headers(&self, access_token: &str) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {access_token}").parse().unwrap(),
        );
        h.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        h.insert(
            reqwest::header::USER_AGENT,
            "google-cloud-sdk vscode_cloudshelleditor/0.1".parse().unwrap(),
        );
        h.insert(
            "X-Goog-Api-Client",
            "gl-node/22.17.0".parse().unwrap(),
        );
        h.insert(
            "Client-Metadata",
            r#"{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}"#
                .parse()
                .unwrap(),
        );
        h
    }
}

// =============================================================================
// What this sketch leaves out (and where to look in the real file):
// =============================================================================
//
// - Project discovery via :loadCodeAssist + :onboardUser LRO polling
// - SSE streaming consumer (GoogleCodeAssistStream) — parses each `data:` line,
//   tracks the latest thoughtSignature across ALL parts, fires content blocks
// - Request translator: walks the OpenAI-shaped conversation history, looks up
//   cached signatures by tool_call_id, attaches them to functionCall parts
// - Retry-on-429 with retryDelay parsing from the response body
// - Quota detection + waiting on Google's `retryDelay` ("44s" → Duration)
//
// See the real `providers/google_codeassist.rs` (~1500 lines) for the full
// implementation.  The patterns above are the parts that are easy to get
// wrong on a re-implementation.
