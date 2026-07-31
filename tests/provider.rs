use pmemc::{
    inspection::{EvidenceBundle, EvidenceFile, EvidenceState},
    provider::{FakeProvider, ModelProvider, parse_response},
};

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
