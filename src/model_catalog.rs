use url::Url;

const OPENROUTER_HOSTS: &[&str] = &["openrouter.ai", "eu.openrouter.ai"];
const OLLAMA_CLOUD_MODELS: &[&str] = &[
    "deepseek-v4-flash:cloud",
    "deepseek-v4-pro:cloud",
    "gemma4:31b-cloud",
    "glm-5.1:cloud",
    "glm-5.2:cloud",
    "gpt-oss:20b-cloud",
    "gpt-oss:120b-cloud",
    "kimi-k2.6:cloud",
    "minimax-m2.7:cloud",
    "minimax-m3:cloud",
    "mistral-large-3:675b-cloud",
    "qwen3.5:cloud",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderKind {
    OpenRouter,
    OllamaCloud,
}

pub(crate) trait ModelCatalog {
    fn validate(&self, model_id: &str) -> bool;
}

struct OpenRouterCatalog;
struct OllamaCloudCatalog;

static OPENROUTER_CATALOG: OpenRouterCatalog = OpenRouterCatalog;
static OLLAMA_CATALOG: OllamaCloudCatalog = OllamaCloudCatalog;

impl ProviderKind {
    pub(crate) fn from_endpoint(endpoint: &str) -> Option<Self> {
        let endpoint = Url::parse(endpoint).ok()?;
        if endpoint.scheme() != "https" {
            return None;
        }
        if endpoint.host_str() == Some("ollama.com")
            && endpoint.path().trim_end_matches('/') == "/api/chat"
        {
            return Some(Self::OllamaCloud);
        }
        OPENROUTER_HOSTS
            .contains(&endpoint.host_str()?)
            .then_some(Self::OpenRouter)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn api_key_env_var(self) -> &'static str {
        match self {
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::OllamaCloud => "OLLAMA_API_KEY",
        }
    }
}

impl ModelCatalog for OpenRouterCatalog {
    fn validate(&self, model_id: &str) -> bool {
        let Some((author, slug)) = model_id.split_once('/') else {
            return false;
        };
        !author.trim().is_empty() && !slug.trim().is_empty()
    }
}

impl ModelCatalog for OllamaCloudCatalog {
    fn validate(&self, model_id: &str) -> bool {
        OLLAMA_CLOUD_MODELS.contains(&model_id)
    }
}

pub(crate) fn catalog_for(kind: ProviderKind) -> &'static dyn ModelCatalog {
    match kind {
        ProviderKind::OpenRouter => &OPENROUTER_CATALOG,
        ProviderKind::OllamaCloud => &OLLAMA_CATALOG,
    }
}

pub(crate) fn validate_model(endpoint: &str, model_id: &str) -> bool {
    let Some(kind) = ProviderKind::from_endpoint(endpoint) else {
        return true;
    };
    catalog_for(kind).validate(model_id)
}

#[cfg(test)]
mod tests {
    use super::{ProviderKind, catalog_for};

    #[test]
    fn classifies_known_provider_endpoints() {
        assert_eq!(
            ProviderKind::from_endpoint("https://openrouter.ai/api/v1/chat/completions"),
            Some(ProviderKind::OpenRouter)
        );
        assert_eq!(
            ProviderKind::from_endpoint("https://ollama.com/api/chat"),
            Some(ProviderKind::OllamaCloud)
        );
        assert_eq!(
            ProviderKind::from_endpoint("https://ollama.com/v1/chat/completions"),
            None
        );
    }

    #[test]
    fn ollama_catalog_accepts_only_allowlisted_models() {
        let catalog = catalog_for(ProviderKind::OllamaCloud);

        assert!(catalog.validate("minimax-m3:cloud"));
        assert!(catalog.validate("gemma4:31b-cloud"));
        assert!(catalog.validate("gpt-oss:120b-cloud"));
        assert!(catalog.validate("gpt-oss:20b-cloud"));
        assert!(catalog.validate("kimi-k2.6:cloud"));
        assert!(!catalog.validate("llama3.1:8b"));
        assert!(!catalog.validate("totally-unknown:cloud"));
    }
}
