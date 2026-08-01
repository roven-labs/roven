//! Interactive proposal and conflict review command.

use std::io::{self, Write};

use crate::{inspection, storage};

pub(crate) fn run(project_id: Option<String>) -> anyhow::Result<()> {
    let data_paths = storage::default_data_paths()?;
    let projects = match project_id {
        Some(project_id) => {
            let id = super::resolve_project_id(&data_paths, &project_id)?;
            let project = storage::project_by_id(&data_paths, id)?
                .ok_or_else(|| anyhow::anyhow!("project {project_id} is not registered"))?;
            vec![project]
        }
        None => storage::list_projects(&data_paths)?,
    };
    for project in projects {
        let proposals = storage::pending_review_proposals(&data_paths, project.id)?;
        for proposal in proposals {
            println!("proposal-{}\t{}", proposal.id, proposal.statement);
            println!("lifecycle\t{}", proposal.lifecycle_state);
            println!("confidence\t{}", proposal.confidence);
            println!("provider\t{}\t{}", proposal.provider_id, proposal.model_id);
            println!(
                "commit\t{}",
                proposal
                    .repository_commit
                    .as_deref()
                    .unwrap_or("working-tree-only")
            );
            for evidence in &proposal.evidence {
                let state = match evidence.state {
                    inspection::EvidenceState::Committed => "committed",
                    inspection::EvidenceState::Staged => "staged",
                    inspection::EvidenceState::Unstaged => "unstaged",
                    inspection::EvidenceState::StagedAndUnstaged => "staged-and-unstaged",
                    inspection::EvidenceState::Untracked => "untracked",
                };
                println!("evidence\t{}\t{}", evidence.path, state);
            }
            for locator in &proposal.evidence_locators {
                println!(
                    "locator\t{}\t{}\t{}",
                    locator.path, locator.line, locator.symbol_id
                );
            }
            for conflict in &proposal.conflicts {
                println!("conflict-{}\t{}", conflict.id, conflict.rationale);
                println!("existing-fact\t{}", conflict.existing_statement);
                for evidence in &conflict.evidence {
                    println!(
                        "existing-evidence\t{}\t{}\t{}\t{}\t{:?}-{:?}\t{}",
                        evidence.path,
                        evidence
                            .repository_commit
                            .as_deref()
                            .unwrap_or("working-tree-only"),
                        evidence.working_tree_state,
                        evidence.evidence_type,
                        evidence.line_start,
                        evidence.line_end,
                        evidence.symbol_id.as_deref().unwrap_or("-")
                    );
                }
            }
            if !proposal.conflicts.is_empty() {
                print!(
                    "[p]reserve existing facts, [u]supersede them, [c]orrect and supersede, or [s]kip? "
                );
                io::stdout().flush()?;
                let mut action = String::new();
                io::stdin().read_line(&mut action)?;
                match action.trim().to_ascii_lowercase().as_str() {
                    "p" | "preserve" => storage::resolve_proposal_conflicts(
                        &data_paths,
                        proposal.id,
                        storage::ConflictResolution::PreserveExisting,
                    )?,
                    "u" | "supersede" => storage::resolve_proposal_conflicts(
                        &data_paths,
                        proposal.id,
                        storage::ConflictResolution::SupersedeExisting,
                    )?,
                    "c" | "correct" => {
                        print!("Corrected statement: ");
                        io::stdout().flush()?;
                        let mut statement = String::new();
                        io::stdin().read_line(&mut statement)?;
                        storage::resolve_proposal_conflicts(
                            &data_paths,
                            proposal.id,
                            storage::ConflictResolution::CorrectAndSupersede {
                                statement: statement.trim().into(),
                            },
                        )?;
                    }
                    _ => println!("proposal-{} left pending", proposal.id),
                }
                continue;
            }
            print!("[a]pprove, [c]orrect, [r]eject, or [s]kip? ");
            io::stdout().flush()?;
            let mut action = String::new();
            io::stdin().read_line(&mut action)?;
            match action.trim().to_ascii_lowercase().as_str() {
                "a" | "approve" => storage::record_review_decision(
                    &data_paths,
                    proposal.id,
                    &storage::ReviewDecision::Approve,
                )?,
                "c" | "correct" => {
                    print!("Corrected statement: ");
                    io::stdout().flush()?;
                    let mut statement = String::new();
                    io::stdin().read_line(&mut statement)?;
                    storage::record_review_decision(
                        &data_paths,
                        proposal.id,
                        &storage::ReviewDecision::CorrectAndApprove {
                            statement: statement.trim().into(),
                        },
                    )?;
                }
                "r" | "reject" => {
                    print!("Reason (optional): ");
                    io::stdout().flush()?;
                    let mut reason = String::new();
                    io::stdin().read_line(&mut reason)?;
                    storage::record_review_decision(
                        &data_paths,
                        proposal.id,
                        &storage::ReviewDecision::Reject {
                            reason: (!reason.trim().is_empty()).then(|| reason.trim().into()),
                        },
                    )?;
                }
                "s" | "skip" => println!("proposal-{} left pending", proposal.id),
                _ => println!("proposal-{} left pending", proposal.id),
            }
        }
        if storage::review_ready(&data_paths, project.id)? {
            print!("Finalize this reviewed inspection? [y/N] ");
            io::stdout().flush()?;
            let mut response = String::new();
            io::stdin().read_line(&mut response)?;
            if matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                let finalization = storage::finalize_review(&data_paths, project.id)?;
                println!(
                    "finalized inspection {} with {} verified facts",
                    finalization.inspection_attempt_id, finalization.accepted_fact_count
                );
            } else {
                println!("review remains ready for finalization");
            }
        }
    }
    Ok(())
}
