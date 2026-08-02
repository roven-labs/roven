//! Read-only status command.

use crate::{baseline, git, storage};

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
    if projects.is_empty() {
        anyhow::bail!("no projects are registered; run `pmemc project add <path>` first");
    }
    for project in projects {
        print_project_status(&project)?;
    }
    Ok(())
}

fn print_project_status(project: &storage::Project) -> anyhow::Result<()> {
    let data_paths = storage::default_data_paths()?;
    let baseline_record = storage::latest_baseline(&data_paths, project.id)?;
    let label = if baseline_record.is_some() {
        "baseline established"
    } else {
        "initial inspection required"
    };
    println!(
        "{}\t{label}\t{}",
        project.name,
        project.canonical_path.display()
    );
    let metadata = git::metadata(&project.canonical_path)?;
    println!(
        "branch\t{}",
        metadata.branch.as_deref().unwrap_or("detached")
    );
    println!(
        "head\t{}",
        metadata.head_commit.as_deref().unwrap_or("unborn")
    );
    match project.head_commit.as_deref() {
        Some(head_commit) => println!(
            "committed-since-registration\t{}",
            git::commit_count_since(&project.canonical_path, head_commit)?
        ),
        None => println!(
            "committed-since-registration\tnot-applicable (repository was unborn at registration)"
        ),
    }
    match baseline_record
        .as_ref()
        .and_then(|baseline| baseline.repository_commit.as_deref())
    {
        Some(commit) => println!(
            "commits-since-baseline\t{}",
            git::commit_count_since(&project.canonical_path, commit)?
        ),
        None if project.lifecycle_state == "registered_needs_inspection" => {
            println!("commits-since-baseline\tnot-applicable (initial inspection required)");
        }
        None => {
            println!("commits-since-baseline\tnot-applicable (baseline was an unborn repository)")
        }
    }
    let status = git::working_tree_status(&project.canonical_path)?;
    let status = baseline::status_since_baseline(
        &project.canonical_path,
        &status,
        baseline_record.as_ref(),
    )?;
    for path in status.committed_paths {
        println!("committed\t{path}");
    }
    for path in status.added_paths {
        println!("added\t{path}");
    }
    for path in status.modified_paths {
        println!("modified\t{path}");
    }
    for path in status.untracked_paths {
        println!("untracked\t{path}");
    }
    for path in status.staged_paths {
        println!("staged\t{path}");
    }
    for path in status.unstaged_paths {
        println!("unstaged\t{path}");
    }
    for path in status.deleted_paths {
        println!("deleted\t{path}");
    }
    for relationship in status.relationships {
        let label = match relationship.kind {
            git::PathRelationshipKind::Renamed => "renamed",
            git::PathRelationshipKind::Copied => "copied",
        };
        println!("{label}\t{}\t{}", relationship.source, relationship.target);
    }
    Ok(())
}
