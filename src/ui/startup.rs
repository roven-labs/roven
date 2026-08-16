use crate::{model_catalog::ProviderKind, profiles::ProviderProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupProviderStatus {
    NoProviderAccess,
    OpenRouterOnly,
    OllamaOnly,
    BothConfigured,
}

pub(crate) fn detect_provider_status<F, G>(
    profiles: &[ProviderProfile],
    mut has_access: F,
    mut has_provider_access: G,
) -> StartupProviderStatus
where
    F: FnMut(&ProviderProfile) -> bool,
    G: FnMut(ProviderKind) -> bool,
{
    let mut openrouter = has_provider_access(ProviderKind::OpenRouter);
    let mut ollama = has_provider_access(ProviderKind::OllamaCloud);

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

#[cfg(test)]
mod tests {
    use super::{StartupProviderStatus, detect_provider_status};
    use crate::model_catalog::ProviderKind;

    #[test]
    fn detects_provider_level_access_without_saved_profiles() {
        assert_eq!(
            detect_provider_status(&[], |_| false, |kind| kind == ProviderKind::OpenRouter),
            StartupProviderStatus::OpenRouterOnly
        );
        assert_eq!(
            detect_provider_status(&[], |_| false, |kind| kind == ProviderKind::OllamaCloud),
            StartupProviderStatus::OllamaOnly
        );
        assert_eq!(
            detect_provider_status(&[], |_| false, |_| true),
            StartupProviderStatus::BothConfigured
        );
        assert_eq!(
            detect_provider_status(&[], |_| false, |_| false),
            StartupProviderStatus::NoProviderAccess
        );
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
        StartupProviderStatus::BothConfigured => {
            vec!["OpenRouter: configured", "Ollama Cloud: configured"]
        }
    }
}
