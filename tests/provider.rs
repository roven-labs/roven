use pmemc::{
    inspection::{EvidenceBundle, EvidenceFile, EvidenceState},
    provider::{
        FakeProvider, ModelProvider, OpenRouterTransportError, Proposal, ProposedConfidence,
        ProposedLifecycle, ProviderFailureCategory, ProviderInvocationMetadata, ProviderResponse,
        parse_response,
    },
    storage,
};

mod support;

use std::{
    collections::VecDeque,
    fs,
    process::Command,
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
    fn complete(
        &self,
        _api_key: &str,
        request: &Value,
    ) -> Result<String, OpenRouterTransportError> {
        self.requests
            .lock()
            .expect("requests lock should work")
            .push(request.clone());
        self.results
            .lock()
            .expect("results lock should work")
            .pop_front()
            .expect("test response should be available")
            .map_err(|category| OpenRouterTransportError {
                category,
                detail: category.to_string(),
            })
    }
}

fn bundle() -> EvidenceBundle {
    bundle_with_state(EvidenceState::Committed)
}

fn bundle_with_state(state: EvidenceState) -> EvidenceBundle {
    EvidenceBundle {
        schema_version: 1,
        project_id: "project-1".into(),
        initial_inspection: true,
        files: vec![EvidenceFile {
            path: "src/lib.rs".into(),
            state,
            content: "pub fn run() {}\n".into(),
            redacted: false,
        }],
    }
}

fn validated_repository() -> (
    support::TemporaryDirectory,
    pmemc::git::ValidatedRepositoryState,
) {
    let repository = support::TemporaryDirectory::new();
    for arguments in [
        vec!["init", "-b", "main"],
        vec!["config", "user.email", "pmemc-test@example.invalid"],
        vec!["config", "user.name", "PMEMC Test"],
    ] {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(repository.path())
            .output()
            .expect("git should run");
        assert!(output.status.success(), "git failed: {output:?}");
    }
    fs::write(repository.path().join("src-lib.rs"), "fixture\n")
        .expect("fixture should be written");
    for arguments in [vec!["add", "."], vec!["commit", "-m", "fixture"]] {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(repository.path())
            .output()
            .expect("git should run");
        assert!(output.status.success(), "git failed: {output:?}");
    }
    let validated = pmemc::git::validate_repository_for_inspection(repository.path())
        .expect("fixture should validate");
    (repository, validated)
}

#[test]
fn missing_model_uses_the_free_router_default() {
    let config =
        OpenRouterConfig::from_environment_with(|_| None).expect("default model should be valid");

    assert_eq!(config.model_id(), "openrouter/free");
}

#[test]
fn explicit_model_override_remains_supported() {
    let config = OpenRouterConfig::from_environment_with(|name| {
        (name == "PMEMC_OPENROUTER_MODEL").then(|| "provider/model".into())
    })
    .expect("explicit model should be valid");

    assert_eq!(config.model_id(), "provider/model");
}

#[test]
fn fake_provider_returns_a_schema_validated_response() {
    let response = parse_response(
        r#"{
            "schema_version": 1,
            "proposals": [{
                "fact_kind": "repository_observation",
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
                "fact_kind": "repository_observation",
                "statement": "Unsupported claim.",
                "lifecycle": "committed",
                "confidence": "exact",
                "evidence_paths": ["secret.env"]
            }],
            "questions": []
        }"#,
        &bundle(),
    );
    let invalid_fact_kind = parse_response(
        r#"{"schema_version":1,"proposals":[{"fact_kind":"Repository Fact","statement":"Bad kind.","lifecycle":"committed","confidence":"exact","evidence_paths":["src/lib.rs"]}],"questions":[]}"#,
        &bundle(),
    );
    let duplicate_evidence = parse_response(
        r#"{"schema_version":1,"proposals":[{"fact_kind":"repository_observation","statement":"Duplicated evidence.","lifecycle":"committed","confidence":"exact","evidence_paths":["src/lib.rs","src/lib.rs"]}],"questions":[]}"#,
        &bundle(),
    );

    assert!(unknown_field.is_err());
    assert!(unknown_evidence.is_err());
    assert!(invalid_fact_kind.is_err());
    assert!(duplicate_evidence.is_err());
}

#[test]
fn invalid_provider_response_exposes_only_a_safe_bounded_reason() {
    let error = parse_response("provider-secret-and-invalid-json", &bundle())
        .expect_err("invalid provider content should be rejected");
    let display = error.to_string();

    assert!(display.contains("model content did not match PMEMC response schema"));
    assert!(!display.contains("provider-secret-and-invalid-json"));
    assert_eq!(
        error.failure_category(),
        Some(ProviderFailureCategory::InvalidResponse)
    );
}

#[test]
fn provider_rejects_committed_proposals_backed_by_working_tree_evidence() {
    let response = parse_response(
        r#"{
            "schema_version": 1,
            "proposals": [{
                "fact_kind": "repository_observation",
                "statement": "The project exposes a run function.",
                "lifecycle": "committed",
                "confidence": "exact",
                "evidence_paths": ["src/lib.rs"]
            }],
            "questions": []
        }"#,
        &bundle_with_state(EvidenceState::Unstaged),
    );

    assert!(response.is_err());
}

#[test]
fn inferred_and_user_confirmed_proposals_can_be_finalized() {
    for confidence in [
        ProposedConfidence::Inferred,
        ProposedConfidence::UserConfirmed,
    ] {
        let data_directory = support::TemporaryDirectory::new();
        let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
        let (_repository, validated) = validated_repository();
        let project = storage::add_project(
            &data_paths,
            &validated.root,
            None,
            Some(&validated.head_commit),
        )
        .expect("project should be stored");
        let response = ProviderResponse {
            schema_version: 1,
            proposals: vec![Proposal {
                fact_kind: "repository_observation".into(),
                statement: "The project exposes a run function.".into(),
                lifecycle: ProposedLifecycle::Committed,
                confidence,
                evidence_paths: vec!["src/lib.rs".into()],
            }],
            questions: Vec::new(),
        };
        let provider = FakeProvider::new(response);

        let _attempt_id = pmemc::submit_approved_bundle(
            &data_paths,
            project.id,
            &validated,
            &bundle(),
            &provider,
        )
        .expect("fake submission should succeed");
        storage::record_review_decision(&data_paths, 1, &storage::ReviewDecision::Approve)
            .expect("proposal approval should be recorded");
        storage::finalize_review(&data_paths, project.id)
            .expect("valid provider confidence should finalize");

        let connection = rusqlite::Connection::open(data_paths.database_path())
            .expect("database should be readable");
        let stored_confidence: String = connection
            .query_row("SELECT confidence FROM fact_evidence", [], |row| row.get(0))
            .expect("fact evidence should be stored");
        assert_eq!(stored_confidence, confidence.as_str());
    }
}

#[test]
fn provider_flags_materially_different_statements_as_conflicts() {
    let data_directory = support::TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let project = storage::add_project(
        &data_paths,
        data_directory.path().join("repository").as_path(),
        None,
        None,
    )
    .expect("project should be stored");
    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    connection
        .execute(
            "INSERT INTO verified_facts (project_id, fact_kind, statement, lifecycle_state, verification_status) VALUES (?1, 'database', 'The project uses SQLite.', 'committed', 'verified')",
            [project.id],
        )
        .expect("existing fact should be stored");
    let fact_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO fact_evidence (fact_id, project_id, relative_path, repository_commit, working_tree_state, excerpt, evidence_type, confidence) VALUES (?1, ?2, 'src/lib.rs', NULL, 'committed', 'database evidence', 'source', 'exact')",
            rusqlite::params![fact_id, project.id],
        )
        .expect("existing evidence should be stored");
    let bundle_json = serde_json::to_string(&bundle()).expect("bundle should serialize");
    let attempt_id = storage::stage_inspection_attempt(&data_paths, project.id, 1, &bundle_json)
        .expect("attempt should be staged");
    let response = ProviderResponse {
        schema_version: 1,
        proposals: vec![Proposal {
            fact_kind: "database".into(),
            statement: "The project uses PostgreSQL.".into(),
            lifecycle: ProposedLifecycle::Committed,
            confidence: ProposedConfidence::Exact,
            evidence_paths: vec!["src/lib.rs".into()],
        }],
        questions: Vec::new(),
    };
    let metadata = ProviderInvocationMetadata::new("fake", "offline-test-model", 1)
        .expect("metadata should be valid");
    storage::store_provider_response(&data_paths, attempt_id, &metadata, &response)
        .expect("response should be stored");

    let conflict_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM conflicts", [], |row| row.get(0))
        .expect("conflict count should be readable");
    assert_eq!(conflict_count, 1);
}

#[test]
fn provider_results_are_pending_review_with_invocation_metadata() {
    let data_directory = support::TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let project = storage::add_project(
        &data_paths,
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
                "fact_kind": "repository_observation",
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
    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    connection
        .execute(
            "INSERT INTO verified_facts (project_id, fact_kind, statement, lifecycle_state, verification_status) VALUES (?1, 'repository_observation', 'not The project exposes a run function.', 'committed', 'verified')",
            [project.id],
        )
        .expect("existing fact should be stored");
    let existing_fact_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO fact_evidence (fact_id, project_id, relative_path, working_tree_state, excerpt, evidence_type, confidence) VALUES (?1, ?2, 'src/lib.rs', 'committed', 'fixture excerpt', 'source', 'exact')",
            [existing_fact_id, project.id],
        )
        .expect("existing fact evidence should be stored");

    storage::store_provider_response(&data_paths, attempt_id, &metadata, &response)
        .expect("response should be persisted");

    let pending_review_project = storage::project_by_id(&data_paths, project.id)
        .expect("project should be readable")
        .expect("project should exist");
    assert_eq!(
        pending_review_project.lifecycle_state,
        "inspection_pending_review"
    );

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
    let conflict: (String, String) = connection
        .query_row(
            "SELECT rationale, status FROM conflicts WHERE proposal_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("contradictory proposal should be recorded as pending conflict");
    assert_eq!(
        conflict,
        (
            "different statement for the same fact kind and evidence path".into(),
            "pending".into(),
        )
    );

    let pending = storage::pending_review_proposals(&data_paths, project.id)
        .expect("pending review proposals should include immutable evidence and metadata");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, 1);
    assert_eq!(pending[0].inspection_attempt_id, attempt_id);
    assert_eq!(pending[0].statement, "The project exposes a run function.");
    assert_eq!(pending[0].provider_id, "fake");
    assert_eq!(pending[0].model_id, "offline-test-model");
    assert_eq!(pending[0].conflicts.len(), 1);
    assert_eq!(
        pending[0].conflicts[0].existing_statement,
        "not The project exposes a run function."
    );
    assert_eq!(pending[0].conflicts[0].evidence.len(), 1);
    assert_eq!(
        (
            pending[0].conflicts[0].evidence[0].path.as_str(),
            pending[0].conflicts[0].evidence[0]
                .working_tree_state
                .as_str(),
            pending[0].conflicts[0].evidence[0].evidence_type.as_str(),
        ),
        ("src/lib.rs", "committed", "source")
    );
    assert_eq!(
        pending[0]
            .evidence
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["src/lib.rs"]
    );

    connection
        .execute(
            "UPDATE proposals SET evidence_paths_json = '[\"missing.rs\"]' WHERE id = 1",
            [],
        )
        .expect("fixture proposal should be corruptible for a fail-closed check");
    let malformed = storage::pending_review_proposals(&data_paths, project.id);
    assert!(matches!(
        malformed,
        Err(storage::StorageError::InvalidStoredEvidence)
    ));
    connection
        .execute(
            "UPDATE proposals SET evidence_paths_json = '[\"src/lib.rs\"]' WHERE id = 1",
            [],
        )
        .expect("fixture proposal should be restored");
    let bypassed_review =
        storage::record_review_decision(&data_paths, 1, &storage::ReviewDecision::Approve);
    assert!(matches!(
        bypassed_review,
        Err(storage::StorageError::InvalidReviewDecision)
    ));
    storage::resolve_proposal_conflicts(
        &data_paths,
        1,
        storage::ConflictResolution::CorrectAndSupersede {
            statement: "The project exposes an operator-approved run function.".into(),
        },
    )
    .expect("correcting and superseding should resolve every proposal conflict atomically");
    let proposal_and_decision: (String, String, String, String) = connection
        .query_row(
            "SELECT p.statement, p.status, d.action, d.corrected_statement FROM proposals p JOIN review_decisions d ON d.proposal_id = p.id WHERE p.id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("original proposal and conflict resolution should be retained together");
    assert_eq!(
        proposal_and_decision,
        (
            "The project exposes a run function.".into(),
            "approved".into(),
            "corrected_and_superseded_existing".into(),
            "The project exposes an operator-approved run function.".into(),
        )
    );
    let fact_state: String = connection
        .query_row(
            "SELECT verification_status FROM verified_facts WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("verified fact should remain authoritative until finalization");
    assert_eq!(fact_state, "verified");
    let conflict_status: String = connection
        .query_row(
            "SELECT status FROM conflicts WHERE proposal_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("resolution should remain pending finalization");
    assert_eq!(conflict_status, "corrected_and_supersede_requested");

    connection
        .execute(
            "INSERT INTO proposals (inspection_attempt_id, provider_invocation_id, fact_kind, statement, lifecycle_state, confidence, evidence_paths_json, status) SELECT 1, id, 'repository_observation', 'preserve candidate', 'committed', 'exact', '[\"src/lib.rs\"]', 'pending_review' FROM provider_invocations LIMIT 1",
            [],
        )
        .expect("preserve fixture proposal should be stored");
    let preserve_proposal_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO conflicts (proposal_id, existing_fact_id, rationale, status) VALUES (?1, 1, 'fixture contradiction', 'pending')",
            [preserve_proposal_id],
        )
        .expect("preserve fixture conflict should be stored");
    storage::resolve_proposal_conflicts(
        &data_paths,
        preserve_proposal_id,
        storage::ConflictResolution::PreserveExisting,
    )
    .expect("preserving should resolve the conflict without changing existing facts");

    connection
        .execute(
            "INSERT INTO proposals (inspection_attempt_id, provider_invocation_id, fact_kind, statement, lifecycle_state, confidence, evidence_paths_json, status) SELECT 1, id, 'repository_observation', 'supersede candidate', 'committed', 'exact', '[\"src/lib.rs\"]', 'pending_review' FROM provider_invocations LIMIT 1",
            [],
        )
        .expect("supersede fixture proposal should be stored");
    let supersede_proposal_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO conflicts (proposal_id, existing_fact_id, rationale, status) VALUES (?1, 1, 'fixture contradiction', 'pending')",
            [supersede_proposal_id],
        )
        .expect("supersede fixture conflict should be stored");
    storage::resolve_proposal_conflicts(
        &data_paths,
        supersede_proposal_id,
        storage::ConflictResolution::SupersedeExisting,
    )
    .expect("superseding should record a finalization request without changing facts early");
    let extra_decisions = connection
        .prepare(
            "SELECT p.status, d.action, c.status FROM proposals p JOIN review_decisions d ON d.proposal_id = p.id JOIN conflicts c ON c.proposal_id = p.id WHERE p.id IN (?1, ?2) ORDER BY p.id",
        )
        .expect("extra decisions should be queryable")
        .query_map([preserve_proposal_id, supersede_proposal_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("extra decisions should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("extra decisions should decode");
    assert_eq!(
        extra_decisions,
        [
            (
                "rejected".into(),
                "preserved_existing".into(),
                "preserved".into()
            ),
            (
                "approved".into(),
                "superseded_existing".into(),
                "supersede_requested".into(),
            ),
        ]
    );
}

#[test]
fn approved_bundle_submission_uses_the_fake_provider_without_network_access() {
    let data_directory = support::TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let (_repository, validated) = validated_repository();
    let project = storage::add_project(
        &data_paths,
        &validated.root,
        None,
        Some(&validated.head_commit),
    )
    .expect("project should be stored");
    let response = parse_response(
        r#"{
            "schema_version": 1,
            "proposals": [{
                "fact_kind": "repository_observation",
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

    let attempt_id =
        pmemc::submit_approved_bundle(&data_paths, project.id, &validated, &bundle(), &provider)
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
    let (_repository, validated) = validated_repository();
    let project = storage::add_project(
        &data_paths,
        &validated.root,
        None,
        Some(&validated.head_commit),
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
            fact_kind: "repository_observation".into(),
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
    assert!(prompt.contains("fact_kind"));
    assert_eq!(
        request["response_format"]["json_schema"]["schema"]["properties"]["proposals"]["items"]["properties"]
            ["fact_kind"]["pattern"],
        "^[a-z0-9_]+$"
    );
    assert!(prompt.contains("src/lib.rs"));
}

#[test]
fn provider_metadata_records_the_actual_routed_model() {
    let transport = ScriptedTransport::new([Ok(json!({
        "model": "provider/actual-free-model",
        "choices": [{
            "message": {
                "content": "{\"schema_version\":1,\"proposals\":[],\"questions\":[]}"
            }
        }]
    })
    .to_string())]);
    let provider = OpenRouterProvider::with_transport(
        OpenRouterConfig::new("openrouter/free", Duration::from_millis(1), 1)
            .expect("config should be valid"),
        "test-key",
        transport,
    );

    provider
        .propose(&bundle())
        .expect("provider response should validate");

    assert_eq!(provider.metadata().model_id, "provider/actual-free-model");
}

#[test]
fn submitted_provider_invocation_persists_the_actual_routed_model() {
    let data_directory = support::TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let (_repository, validated) = validated_repository();
    let project = storage::add_project(
        &data_paths,
        &validated.root,
        None,
        Some(&validated.head_commit),
    )
    .expect("project should be stored");
    let transport = ScriptedTransport::new([Ok(json!({
        "model": "provider/actual-free-model",
        "choices": [{
            "message": {"content": "{\"schema_version\":1,\"proposals\":[],\"questions\":[]}"}
        }]
    })
    .to_string())]);
    let provider = OpenRouterProvider::with_transport(
        OpenRouterConfig::new("openrouter/free", Duration::from_millis(1), 1)
            .expect("config should be valid"),
        "test-key",
        transport,
    );

    pmemc::submit_approved_bundle(&data_paths, project.id, &validated, &bundle(), &provider)
        .expect("submission should succeed");

    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    let model_id: String = connection
        .query_row("SELECT model_id FROM provider_invocations", [], |row| {
            row.get(0)
        })
        .expect("provider invocation should be stored");
    assert_eq!(model_id, "provider/actual-free-model");
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
