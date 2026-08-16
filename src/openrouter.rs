use std::time::Duration;

use crate::model_catalog::validate_model;
use serde_json::Value;
use url::Url;

pub(crate) fn is_endpoint(endpoint: &str) -> bool {
    let Ok(endpoint) = Url::parse(endpoint) else {
        return false;
    };
    endpoint.scheme() == "https"
        && matches!(
            endpoint.host_str(),
            Some("openrouter.ai") | Some("eu.openrouter.ai")
        )
}

pub(crate) fn context_window(api_key: &str, endpoint: &str, model: &str) -> Option<usize> {
    let url = metadata_url(endpoint, model)?;

    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .build(),
    );
    let mut response = agent
        .get(url.as_str())
        .header("Authorization", &format!("Bearer {api_key}"))
        .call()
        .ok()?;
    if !(200..300).contains(&response.status().as_u16()) {
        return None;
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .ok()?;
    parse_context_window(&serde_json::from_str::<Value>(&body).ok()?)
}

fn metadata_url(endpoint: &str, model: &str) -> Option<Url> {
    if !is_endpoint(endpoint) || !validate_model(endpoint, model) {
        return None;
    }
    let (author, slug) = model.split_once('/')?;
    let mut url = Url::parse(endpoint).ok()?;
    url.set_path("/api/v1/models");
    url.set_query(None);
    url.set_fragment(None);
    url.path_segments_mut()
        .ok()?
        .push(author)
        .push(slug)
        .push("endpoints");
    Some(url)
}

fn parse_context_window(value: &Value) -> Option<usize> {
    value
        .get("data")
        .and_then(|data| data.get("endpoints"))
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|endpoint| endpoint.get("context_length").and_then(Value::as_u64))
        .filter(|tokens| *tokens > 0)
        .filter_map(|tokens| usize::try_from(tokens).ok())
        .max()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use url::Url;

    use super::{context_window, metadata_url, parse_context_window};

    #[test]
    fn only_fetches_metadata_for_openrouter_hosts() {
        assert_eq!(
            context_window(
                "secret",
                "https://example.test/v1/chat/completions",
                "a/model"
            ),
            None
        );
    }

    #[test]
    fn builds_the_openrouter_endpoints_url_from_the_chat_endpoint() {
        assert_eq!(
            metadata_url(
                "https://openrouter.ai/api/v1/chat/completions",
                "openai/gpt-oss-20b",
            )
            .as_ref()
            .map(Url::as_str),
            Some("https://openrouter.ai/api/v1/models/openai/gpt-oss-20b/endpoints")
        );
    }

    #[test]
    fn reads_positive_context_lengths_from_endpoint_records() {
        assert_eq!(
            parse_context_window(&json!({
                "data": {
                    "endpoints": [
                        {"context_length": 0},
                        {"context_length": 131072},
                        {"context_length": 32768}
                    ]
                }
            })),
            Some(131072)
        );
    }
}
