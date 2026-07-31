use pmemc::{
    inspection::{EvidenceBundle, EvidenceFile, EvidenceState},
    provider::{
        FakeProvider, ModelProvider, Proposal, ProposedConfidence, ProposedLifecycle,
        ProviderFailureCategory, ProviderInvocationMetadata, ProviderResponse, parse_response,
    },
    storage,
};

mod support;

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Value, json};

use pmemc::provider::{OpenRouterConfig, OpenRouterProvider, OpenRouterTransport};

#[derive(Clone)]
struct ScriptedTransport {
    results: Arc<Mutex<VecDeque<Result<String, ProviderFailureCategory>>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedTransport {
    fn new(results: impl IntoIterator<Item = Result<String, ProviderFailureCategory>>) -> Self {
        Self {
            results: Arc::new(Mutex::new(results.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .expect("requests lock should work")
            .clone()
    }
}

impl OpenRouterTransport for ScriptedTransport {
    fn complete(&self, _api_key: &str, request: &Value) -> Result<String, ProviderFailureCategory> {
        self.requests
            .lock()
            .expect("requests lock should work")
            .push(request.clone());
        self.results
            .lock()
            .expect("results lock should work")
            .pop_front()
            .expect("test response should be available")
    }
}

fn bundle() -> EvidenceBundle {
    EvidenceBundle {
        schema_version: 1,
        project_id: "project-1".into(),
        initial_inspection: true,
        files: vec![EvidenceFile {
            path: "src/lib.rs".into(),
            state: EvidenceState::Committed,
            content: "pub fn run() {}\n".into(),
            redacted: false,
        }],
    }
}

#[test]
fn fake_provider_returns_a_schema_validated_response() {
    let response = parse_response(
        r#"{
            "schema_version": 1,
            "proposals": [{
                "statement": "The project exposes a run function.",
                "lifecycle": "committed",
                "confidence": "exact",
                "evidence_paths": ["src/lib.rs"]
            }],
            "questions": ["What is the intended command-line audience?"]
        }"#,
        &bundle(),
    )
    .expect("response should validate");
    let provider = FakeProvider::new(response.clone());

    assert_eq!(
        provider.propose(&bundle()).expect("fake should respond"),
        response
    );
}

#[test]
fn provider_response_rejects_unknown_fields_and_unselected_evidence() {
    let unknown_field = parse_response(
        r#"{"schema_version":1,"proposals":[],"questions":[],"unexpected":true}"#,
        &bundle(),
    );
    let unknown_evidence = parse_response(
        r#"{
            "schema_version": 1,
            "proposals": [{
                "statement": "Unsupported claim.",
                "lifecycle": "committed",
                "confidence": "exact",
                "evidence_paths": ["secret.env"]
            }],
            "questions": []
        }"#,
        &bundle(),
    );

    assert!(unknown_field.is_err());
    assert!(unknown_evidence.is_err());
}

#[test]
fn provider_results_are_pending_review_with_invocation_metadata() {
    let data_directory = support::TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let project = storage::add_project(
        &data_paths,
        "fixture",
        data_directory.path().join("repository").as_path(),
        None,
        None,
    )
    .expect("project should be stored");
    let bundle_json = serde_json::to_string(&bundle()).expect("bundle should serialize");
    let attempt_id = storage::stage_inspection_attempt(&data_paths, project.id, 1, &bundle_json)
        .expect("attempt should be staged");
    let staged_project = storage::project_by_id(&data_paths, project.id)
        .expect("project should be readable")
        .expect("project should exist");
    assert_eq!(
        staged_project.lifecycle_state,
        "registered_needs_inspection"
    );
    let response = parse_response(
        r#"{
            "schema_version": 1,
            "proposals": [{
                "statement": "The project exposes a run function.",
                "lifecycle": "committed",
                "confidence": "exact",
                "evidence_paths": ["src/lib.rs"]
            }],
            "questions": ["What is the intended command-line audience?"]
        }"#,
        &bundle(),
    )
    .expect("response should validate");
    let metadata = ProviderInvocationMetadata::new("fake", "offline-test-model", 1)
        .expect("metadata should be valid");

    storage::store_provider_response(&data_paths, attempt_id, &metadata, &response)
        .expect("response should be persisted");

    let pending_review_project = storage::project_by_id(&data_paths, project.id)
        .expect("project should be readable")
        .expect("project should exist");
    assert_eq!(
        pending_review_project.lifecycle_state,
        "inspection_pending_review"
    );

    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    let invocation: (String, String, i64, String) = connection
        .query_row(
            "SELECT provider_id, model_id, prompt_schema_version, status FROM provider_invocations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("invocation metadata should be present");
    assert_eq!(
        invocation,
        (
            "fake".into(),
            "offline-test-model".into(),
            1,
            "succeeded".into()
        )
    );
    let proposal_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM proposals WHERE status = 'pending_review'",
            [],
            |row| row.get(0),
        )
        .expect("proposal should be pending review");
    let question_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM questions WHERE status = 'pending_review'",
            [],
            |row| row.get(0),
        )
        .expect("question should be pending review");
    assert_eq!((proposal_count, question_count), (1, 1));

    let pending = storage::pending_review_proposals(&data_paths, project.id)
        .expect("pending review proposals should include immutable evidence and metadata");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, 1);
    assert_eq!(pending[0].inspection_attempt_id, attempt_id);
    assert_eq!(pending[0].statement, "The project exposes a run function.");
    assert_eq!(pending[0].provider_id, "fake");
    assert_eq!(pending[0].model_id, "offline-test-model");
    assert_eq!(
        pending[0]
            .evidence
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["src/lib.rs"]
    );
}

#[test]
fn approved_bundle_submission_uses_the_fake_provider_without_network_access() {
    let data_directory = support::TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let project = storage::add_project(
        &data_paths,
        "fixture",
        data_directory.path().join("repository").as_path(),
        None,
        None,
    )
    .expect("project should be stored");
    let response = parse_response(
        r#"{
            "schema_version": 1,
            "proposals": [{
                "statement": "The project exposes a run function.",
                "lifecycle": "committed",
                "confidence": "exact",
                "evidence_paths": ["src/lib.rs"]
            }],
            "questions": []
        }"#,
        &bundle(),
    )
    .expect("response should validate");
    let provider = FakeProvider::new(response);

    let attempt_id = pmemc::submit_approved_bundle(&data_paths, project.id, &bundle(), &provider)
        .expect("fake submission should succeed");

    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    let proposal_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM proposals WHERE inspection_attempt_id = ?1",
            [attempt_id],
            |row| row.get(0),
        )
        .expect("proposal count should be readable");
    let updated_project = storage::project_by_id(&data_paths, project.id)
        .expect("project should be readable")
        .expect("project should exist");
    assert_eq!(proposal_count, 1);
    assert_eq!(updated_project.lifecycle_state, "inspection_pending_review");
}

#[test]
fn provider_failure_restores_the_previous_project_lifecycle() {
    let data_directory = support::TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let project = storage::add_project(
        &data_paths,
        "fixture",
        data_directory.path().join("repository").as_path(),
        None,
        None,
    )
    .expect("project should be stored");
    let bundle_json = serde_json::to_string(&bundle()).expect("bundle should serialize");
    let attempt_id = storage::stage_inspection_attempt(&data_paths, project.id, 1, &bundle_json)
        .expect("attempt should be staged");

    let metadata = ProviderInvocationMetadata::new("fake", "offline-test-model", 1)
        .expect("metadata should be valid");
    storage::record_provider_failure(
        &data_paths,
        attempt_id,
        &metadata,
        ProviderFailureCategory::TimedOut,
    )
    .expect("failure should be recoverable");

    let restored = storage::project_by_id(&data_paths, project.id)
        .expect("project should be readable")
        .expect("project should still exist");
    assert_eq!(restored.lifecycle_state, "registered_needs_inspection");
    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    let status: String = connection
        .query_row(
            "SELECT status FROM inspection_attempts WHERE id = ?1",
            [attempt_id],
            |row| row.get(0),
        )
        .expect("attempt should exist");
    assert_eq!(status, "provider_failed");
    let failure_metadata: (String, String, i64, String) = connection
        .query_row(
            "SELECT provider_id, model_id, prompt_schema_version, failure_category FROM provider_invocations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("failure invocation should be present");
    assert_eq!(
        failure_metadata,
        (
            "fake".into(),
            "offline-test-model".into(),
            1,
            "timed_out".into(),
        )
    );
}

#[test]
fn failed_provider_attempt_can_retry_without_losing_invocation_history() {
    let data_directory = support::TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let project = storage::add_project(
        &data_paths,
        "fixture",
        data_directory.path().join("repository").as_path(),
        None,
        None,
    )
    .expect("project should be stored");
    let bundle_json = serde_json::to_string(&bundle()).expect("bundle should serialize");
    let attempt_id = storage::stage_inspection_attempt(&data_paths, project.id, 1, &bundle_json)
        .expect("attempt should be staged");
    let metadata = ProviderInvocationMetadata::new("fake", "offline-test-model", 1)
        .expect("metadata should be valid");
    storage::record_provider_failure(
        &data_paths,
        attempt_id,
        &metadata,
        ProviderFailureCategory::RateLimited,
    )
    .expect("failure should be recoverable");

    storage::retry_provider_attempt(&data_paths, attempt_id).expect("attempt should be retryable");
    let response = parse_response(
        r#"{"schema_version":1,"proposals":[],"questions":[]}"#,
        &bundle(),
    )
    .expect("response should validate");
    storage::store_provider_response(&data_paths, attempt_id, &metadata, &response)
        .expect("retry response should be stored");

    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    let invocation_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM provider_invocations", [], |row| {
            row.get(0)
        })
        .expect("both invocations should be retained");
    let attempt_status: String = connection
        .query_row(
            "SELECT status FROM inspection_attempts WHERE id = ?1",
            [attempt_id],
            |row| row.get(0),
        )
        .expect("attempt should exist");
    assert_eq!(invocation_count, 2);
    assert_eq!(attempt_status, "pending_review");
}

#[test]
fn persistence_revalidates_untrusted_provider_response_against_staged_evidence() {
    let data_directory = support::TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let project = storage::add_project(
        &data_paths,
        "fixture",
        data_directory.path().join("repository").as_path(),
        None,
        None,
    )
    .expect("project should be stored");
    let bundle_json = serde_json::to_string(&bundle()).expect("bundle should serialize");
    let attempt_id = storage::stage_inspection_attempt(&data_paths, project.id, 1, &bundle_json)
        .expect("attempt should be staged");
    let metadata = ProviderInvocationMetadata::new("fake", "offline-test-model", 1)
        .expect("metadata should be valid");
    let invalid_response = ProviderResponse {
        schema_version: 1,
        proposals: vec![Proposal {
            statement: "This cites evidence outside the approved bundle.".into(),
            lifecycle: ProposedLifecycle::Committed,
            confidence: ProposedConfidence::Exact,
            evidence_paths: vec!["secret.env".into()],
        }],
        questions: Vec::new(),
    };

    let result =
        storage::store_provider_response(&data_paths, attempt_id, &metadata, &invalid_response);

    assert!(result.is_err());
    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    let invocation_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM provider_invocations", [], |row| {
            row.get(0)
        })
        .expect("database should be readable");
    let proposal_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM proposals", [], |row| row.get(0))
        .expect("database should be readable");
    assert_eq!((invocation_count, proposal_count), (0, 0));
    let preserved_project = storage::project_by_id(&data_paths, project.id)
        .expect("project should be readable")
        .expect("project should exist");
    assert_eq!(
        preserved_project.lifecycle_state,
        "registered_needs_inspection"
    );
}

#[test]
fn openrouter_provider_sends_a_versioned_prompt_and_validates_chat_content() {
    let transport = ScriptedTransport::new([Ok(json!({
        "choices": [{
            "message": {
                "content": json!({
                    "schema_version": 1,
                    "proposals": [],
                    "questions": ["Which target users should the CLI support?"]
                })
                .to_string()
            }
        }]
    })
    .to_string())]);
    let provider = OpenRouterProvider::with_transport(
        OpenRouterConfig::new("test/model", Duration::from_millis(1), 1)
            .expect("config should be valid"),
        "test-key",
        transport.clone(),
    );

    let response = provider
        .propose(&bundle())
        .expect("provider response should validate");

    assert_eq!(
        response.questions,
        ["Which target users should the CLI support?"]
    );
    let request = transport.requests().pop().expect("request should be sent");
    assert_eq!(request["model"], "test/model");
    assert_eq!(request["response_format"]["type"], "json_schema");
    assert_eq!(request["response_format"]["json_schema"]["strict"], true);
    assert_eq!(
        request["response_format"]["json_schema"]["schema"]["additionalProperties"],
        false
    );
    let prompt = request["messages"][1]["content"]
        .as_str()
        .expect("user prompt should be text");
    assert!(prompt.contains("PMEMC proposal schema version: 1"));
    assert!(prompt.contains("src/lib.rs"));
}

#[test]
fn openrouter_provider_retries_a_rate_limited_request_within_its_bound() {
    let success = json!({
        "choices": [{
            "message": {"content": "{\"schema_version\":1,\"proposals\":[],\"questions\":[]}"}
        }]
    })
    .to_string();
    let transport =
        ScriptedTransport::new([Err(ProviderFailureCategory::RateLimited), Ok(success)]);
    let provider = OpenRouterProvider::with_transport(
        OpenRouterConfig::new("test/model", Duration::from_millis(1), 2)
            .expect("config should be valid"),
        "test-key",
        transport.clone(),
    );

    provider
        .propose(&bundle())
        .expect("second request should succeed");

    assert_eq!(transport.requests().len(), 2);
}

#[test]
fn openrouter_provider_preserves_failure_categories_without_exposing_a_key() {
    let transport = ScriptedTransport::new([Err(ProviderFailureCategory::Unauthorized)]);
    let provider = OpenRouterProvider::with_transport(
        OpenRouterConfig::new("test/model", Duration::from_millis(1), 1)
            .expect("config should be valid"),
        "test-key-must-not-appear-in-error",
        transport,
    );

    let error = provider
        .propose(&bundle())
        .expect_err("request should fail");

    assert_eq!(
        error.failure_category(),
        Some(ProviderFailureCategory::Unauthorized)
    );
    assert!(
        !error
            .to_string()
            .contains("test-key-must-not-appear-in-error")
    );
}

#[test]
fn openrouter_provider_rejects_a_partial_chat_completion() {
    let transport = ScriptedTransport::new([Ok(json!({"choices": []}).to_string())]);
    let provider = OpenRouterProvider::with_transport(
        OpenRouterConfig::new("test/model", Duration::from_millis(1), 1)
            .expect("config should be valid"),
        "test-key",
        transport,
    );

    let error = provider
        .propose(&bundle())
        .expect_err("partial completion should fail validation");

    assert_eq!(
        error.failure_category(),
        Some(ProviderFailureCategory::InvalidResponse)
    );
}

#[test]
fn openrouter_provider_classifies_a_timeout_without_retrying() {
    let transport = ScriptedTransport::new([Err(ProviderFailureCategory::TimedOut)]);
    let provider = OpenRouterProvider::with_transport(
        OpenRouterConfig::new("test/model", Duration::from_millis(1), 3)
            .expect("config should be valid"),
        "test-key",
        transport.clone(),
    );

    let error = provider
        .propose(&bundle())
        .expect_err("timeout should fail without another request");

    assert_eq!(
        error.failure_category(),
        Some(ProviderFailureCategory::TimedOut)
    );
    assert_eq!(transport.requests().len(), 1);
}
