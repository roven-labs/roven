//! Project registration, listing, showing, forgetting, and history commands.

use std::io::{self, Write};

use crate::{baseline, cli, git, storage};

pub(crate) fn history(project_id: &str) -> anyhow::Result<()> {
    let data_paths = storage::default_data_paths()?;
    let id = super::resolve_project_id(&data_paths, project_id)?;
    for entry in storage::project_history(&data_paths, id)? {
        println!("{}\t{}\t{}", entry.created_at, entry.kind, entry.detail);
    }
    Ok(())
}

pub(crate) fn run(command: cli::ProjectCommand) -> anyhow::Result<()> {
    let data_paths = storage::default_data_paths()?;
    match command {
        cli::ProjectCommand::Add { path } => {
            let metadata = git::metadata(&path)?;
            let name = metadata
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project");
            let project = storage::add_project(
                &data_paths,
                name,
                &metadata.root,
                metadata.branch.as_deref(),
                metadata.head_commit.as_deref(),
            )?;
            println!(
                "registered {} as {} (project-{})",
                project.canonical_path.display(),
                project.display_name,
                project.id
            );
            Ok(())
        }
        cli::ProjectCommand::List => {
            for project in storage::list_projects(&data_paths)? {
                let metadata = git::metadata(&project.canonical_path)?;
                let baseline_record = storage::latest_baseline(&data_paths, project.id)?;
                let status = git::working_tree_status(&project.canonical_path)?;
                let status = baseline::status_since_baseline(
                    &project.canonical_path,
                    &status,
                    baseline_record.as_ref(),
                )?;
                let commits_since_baseline = match baseline_record
                    .as_ref()
                    .and_then(|baseline| baseline.repository_commit.as_deref())
                {
                    Some(commit) => git::commit_count_since(&project.canonical_path, commit)?,
                    None => 0,
                };
                let changes_detected =
                    commits_since_baseline != 0 || baseline::changed_path_count(&status) != 0;
                println!(
                    "project-{}\t{}\t{}\t{}\tbranch={}\tlast-approved-inspection={}\tchanges-detected={}",
                    project.id,
                    project.display_name,
                    project.canonical_path.display(),
                    project.lifecycle_state,
                    metadata.branch.as_deref().unwrap_or("detached"),
                    baseline_record
                        .as_ref()
                        .map(|baseline| baseline.created_at.as_str())
                        .unwrap_or("none"),
                    if baseline_record.is_none() {
                        "initial-inspection-required"
                    } else if changes_detected {
                        "yes"
                    } else {
                        "no"
                    },
                );
            }
            Ok(())
        }
        cli::ProjectCommand::Show { project_id } => {
            let id = super::resolve_project_id(&data_paths, &project_id)?;
            let project = storage::project_by_id(&data_paths, id)?
                .ok_or_else(|| anyhow::anyhow!("project {project_id} is not registered"))?;
            println!(
                "project-{}\t{}\t{}\t{}\tbranch={}\thead={}",
                project.id,
                project.display_name,
                project.canonical_path.display(),
                project.lifecycle_state,
                project.current_branch.as_deref().unwrap_or("detached"),
                project.head_commit.as_deref().unwrap_or("unborn")
            );
            let memory = storage::project_memory(&data_paths, id)?;
            match memory.baseline {
                Some(baseline) => println!(
                    "baseline\tattempt={}\tcommit={}\tbranch={}\tworking-tree={}\tfingerprints={}\tat={}",
                    baseline.inspection_attempt_id,
                    baseline.repository_commit.as_deref().unwrap_or("unborn"),
                    baseline.repository_branch.as_deref().unwrap_or("detached"),
                    baseline.working_tree_status_json,
                    baseline.uncommitted_fingerprints_json,
                    baseline.created_at
                ),
                None => println!("baseline\tnone"),
            }
            for fact in memory.verified_facts {
                println!(
                    "fact-{}\t{}\t{}\t{}\t{}",
                    fact.id,
                    fact.fact_kind,
                    fact.lifecycle_state,
                    fact.verification_status,
                    fact.statement
                );
            }
            for question in memory.unresolved_questions {
                println!("unresolved-question\t{question}");
            }
            println!("evidence-count\t{}", memory.evidence_count);
            println!("proposal-count\t{}", memory.proposal_count);
            println!("decision-count\t{}", memory.decision_count);
            Ok(())
        }
        cli::ProjectCommand::Forget {
            project_id,
            confirm_name,
        } => forget(&data_paths, &project_id, confirm_name.as_deref()),
    }
}

fn forget(
    data_paths: &storage::DataPaths,
    project_reference: &str,
    confirm_name: Option<&str>,
) -> anyhow::Result<()> {
    let id = super::resolve_project_id(data_paths, project_reference)?;
    let project = storage::project_by_id(data_paths, id)?
        .ok_or_else(|| anyhow::anyhow!("project {project_reference} is not registered"))?;
    let memory = storage::project_memory(data_paths, id)?;
    println!("PMEMC project forget preview");
    println!("project\t{} (project-{})", project.display_name, project.id);
    println!("repository\t{}", project.canonical_path.display());
    println!("verified-facts\t{}", memory.verified_facts.len());
    println!("evidence\t{}", memory.evidence_count);
    println!("proposals\t{}", memory.proposal_count);
    println!("decisions\t{}", memory.decision_count);
    println!("Repository files will not be changed.");
    println!("This permanently removes this project's PMEMC memory and registration.");

    let confirmed = match confirm_name {
        Some(name) => name == project.display_name,
        None => {
            print!("Type {} to confirm: ", project.display_name);
            io::stdout().flush()?;
            let mut response = String::new();
            io::stdin().read_line(&mut response)?;
            response.trim() == project.display_name
        }
    };
    if !confirmed {
        println!("forget cancelled; no PMEMC records were changed");
        return Ok(());
    }

    let summary = storage::forget_project(data_paths, id)?;
    println!(
        "PMEMC memory and registration forgotten for {} (project-{})",
        summary.display_name, summary.project_id
    );
    println!(
        "removed\tfacts={} evidence={} proposals={} questions={} decisions={} inspections={}",
        summary.verified_fact_count,
        summary.evidence_count,
        summary.proposal_count,
        summary.question_count,
        summary.decision_count,
        summary.inspection_count
    );
    println!(
        "Repository files were not changed: {}",
        summary.canonical_path.display()
    );
    println!("Register it again with `pmemc project add <path>`.");
    Ok(())
}
