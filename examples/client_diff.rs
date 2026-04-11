// =============================================================================
// rust/crates/api/src/client.rs — minimal diff to dispatch to Code Assist
// =============================================================================
//
// Apply these edits to the real client.rs in claw-code's tree.

use crate::providers::google_codeassist::{GoogleCodeAssistClient, GoogleCodeAssistStream};

// 1. Add a Google variant to the ProviderClient enum:

pub enum ProviderClient {
    Anthropic(AnthropicClient),
    OpenAiCompat(OpenAiCompatClient),
    Google(GoogleCodeAssistClient), // <-- NEW
}

// 2. Add a Google variant to the MessageStream enum:

pub enum MessageStream {
    Anthropic(AnthropicStream),
    OpenAiCompat(OpenAiCompatStream),
    Google(GoogleCodeAssistStream), // <-- NEW
}

// 3. In ProviderClient::from_model_with_anthropic_auth(), construct the client:

impl ProviderClient {
    pub fn from_model_with_anthropic_auth(
        model: &str,
        anthropic_auth: AuthMethod,
    ) -> Result<Self, ApiError> {
        let metadata = metadata_for_model(model);

        match metadata.provider {
            ProviderKind::Anthropic => {
                Ok(ProviderClient::Anthropic(AnthropicClient::new(anthropic_auth)?))
            }
            ProviderKind::OpenAiCompat => {
                Ok(ProviderClient::OpenAiCompat(OpenAiCompatClient::new(metadata)?))
            }
            // NEW:
            ProviderKind::Google => {
                Ok(ProviderClient::Google(GoogleCodeAssistClient::new()?))
            }
        }
    }

    // 4. In stream_message(), dispatch to the Google variant:

    pub async fn stream_message(
        &self,
        request: MessageRequest,
    ) -> Result<MessageStream, ApiError> {
        match self {
            ProviderClient::Anthropic(c) => Ok(MessageStream::Anthropic(c.stream(request).await?)),
            ProviderClient::OpenAiCompat(c) => Ok(MessageStream::OpenAiCompat(c.stream(request).await?)),
            // NEW:
            ProviderClient::Google(c) => Ok(MessageStream::Google(c.stream(request).await?)),
        }
    }
}

// 5. In MessageStream's poll/next implementation, dispatch the Google variant:

impl MessageStream {
    pub async fn next_event(&mut self) -> Option<Result<StreamEvent, ApiError>> {
        match self {
            MessageStream::Anthropic(s) => s.next_event().await,
            MessageStream::OpenAiCompat(s) => s.next_event().await,
            // NEW:
            MessageStream::Google(s) => s.next_event().await,
        }
    }
}
