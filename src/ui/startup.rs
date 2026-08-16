use crate::{model_catalog::ProviderKind, profiles::ProviderProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupProviderStatus {
    NoProviderAccess,
    OpenRouterOnly,
    OllamaOnly,
    BothConfigured,
}

pub(crate) fn detect_provider_status<F>(
    profiles: &[ProviderProfile],
    mut has_access: F,
) -> StartupProviderStatus
where
    F: FnMut(&ProviderProfile) -> bool,
{
    let mut openrouter = false;
    let mut ollama = false;

    for profile in profiles {
        if !has_access(profile) {
            continue;
        }
        match ProviderKind::from_endpoint(&profile.endpoint) {
            Some(ProviderKind::OpenRouter) => openrouter = true,
            Some(ProviderKind::OllamaCloud) => ollama = true,
            None => {}
        }
    }

    match (openrouter, ollama) {
        (false, false) => StartupProviderStatus::NoProviderAccess,
        (true, false) => StartupProviderStatus::OpenRouterOnly,
        (false, true) => StartupProviderStatus::OllamaOnly,
        (true, true) => StartupProviderStatus::BothConfigured,
    }
}

pub(crate) fn banner_lines(status: StartupProviderStatus) -> Vec<&'static str> {
    match status {
        StartupProviderStatus::NoProviderAccess => vec![
            "OpenRouter: missing",
            "Ollama Cloud: missing",
            "Next step: roven auth set",
        ],
        StartupProviderStatus::OpenRouterOnly => {
            vec!["OpenRouter: configured", "Ollama Cloud: missing"]
        }
        StartupProviderStatus::OllamaOnly => {
            vec!["OpenRouter: missing", "Ollama Cloud: configured"]
        }
        StartupProviderStatus::BothConfigured => vec![
            "OpenRouter: configured",
            "Ollama Cloud: configured",
        ],
    }
}
