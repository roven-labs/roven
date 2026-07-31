//! Domain types independent from the CLI and storage adapters.

/// Stable identifier assigned to a registered project.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectId(String);

impl ProjectId {
    /// Create a project identifier from its persisted value.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrow the persisted project identifier value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Durable lifecycle state for a registered project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectLifecycle {
    /// The project has been registered but not inspected.
    RegisteredNeedsInspection,
    /// An inspection produced proposals awaiting review.
    InspectionPendingReview,
    /// The latest inspection baseline has been approved.
    Baselined,
    /// Repository changes exist after the approved baseline.
    ChangesDetected,
}
