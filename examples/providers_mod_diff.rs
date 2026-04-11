// =============================================================================
// rust/crates/api/src/providers/mod.rs — minimal diff to wire in Code Assist
// =============================================================================
//
// This file shows the SHAPE of the changes needed in providers/mod.rs.  It's
// not directly compilable on its own — apply these edits to the real file
// in claw-code's tree.

// 1. Declare the new module at the top of the file:

pub mod google_codeassist;

// 2. Add a Google variant to ProviderKind:

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderKind {
    Anthropic,
    OpenAiCompat,
    Google, // <-- NEW
}

// 3. Add a helper that checks for the gemini-cli credentials file:

fn google_codeassist_credentials_present() -> bool {
    home_dir()
        .map(|h| h.join(".gemini").join("oauth_creds.json").exists())
        .unwrap_or(false)
}

// 4. In metadata_for_model(), prefer Code Assist when the credentials exist
//    and the model name starts with "gemini-":

pub fn metadata_for_model(model_name: &str) -> ProviderMetadata {
    let trimmed = model_name.trim();

    if trimmed.starts_with("gemini-") {
        if google_codeassist_credentials_present() {
            return ProviderMetadata {
                provider: ProviderKind::Google,
                canonical_model: trimmed.to_string(),
                base_url: "https://cloudcode-pa.googleapis.com".to_string(),
                auth_env: None, // OAuth, not API key
            };
        }
        // Fall through to OpenAI-compat path against generativelanguage.googleapis.com
    }

    // ...existing routing for other providers...
    unreachable!()
}

// 5. In resolve_model_alias(), add an arm for Google so gemini-* aliases work:

fn resolve_model_alias(provider: ProviderKind, trimmed: &str) -> String {
    match provider {
        ProviderKind::Anthropic => trimmed.to_string(),
        ProviderKind::OpenAiCompat => trimmed.to_string(),
        ProviderKind::Google => trimmed.to_string(), // <-- NEW
    }
}
