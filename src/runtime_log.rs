//! Append-only, privacy-preserving runtime diagnostics for Roven.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use directories::ProjectDirs;

use crate::storage::now_ms;

const QUALIFIER: &str = "io.github.vishal24p";
const APPLICATION: &str = "Roven";

#[derive(Clone)]
pub(crate) struct RuntimeLog {
    file: Arc<Mutex<File>>,
    path: PathBuf,
}

impl RuntimeLog {
    pub(crate) fn for_current_user() -> io::Result<Self> {
        let dirs = ProjectDirs::from(QUALIFIER, "", APPLICATION).ok_or_else(|| {
            io::Error::other("the operating-system local data directory is unavailable")
        })?;
        Self::for_file(dirs.data_local_dir().join("log.md"))
    }

    pub(crate) fn for_file(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let has_contents = path.metadata().is_ok_and(|metadata| metadata.len() > 0);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        if !has_contents {
            file.write_all(b"# Roven runtime log\n\n")?;
            file.sync_data()?;
        }
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            path,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Records operational metadata only. Callers must never pass prompts, responses, or secrets.
    pub(crate) fn record(&self, component: &str, event: &str, detail: &str) {
        let record = format!(
            "- `timestamp_ms={}` `component={}` `event={}` {}\n",
            now_ms(),
            sanitize(component),
            sanitize(event),
            sanitize(detail),
        );
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(record.as_bytes());
            let _ = file.sync_data();
        }
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\n' | '\r' | '\t' => ' ',
            '`' => '\'',
            character => character,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::RuntimeLog;

    #[test]
    fn appends_structured_markdown_records_without_line_break_injection() {
        let path =
            std::env::temp_dir().join(format!("roven-runtime-log-{}.md", uuid::Uuid::now_v7()));
        let log = RuntimeLog::for_file(&path).unwrap();

        log.record("provider", "request_failed", "http_status=429\nignored");
        log.record("agent", "turn_finished", "outcome=error");

        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.starts_with("# Roven runtime log\n"));
        assert!(contents.contains("component=provider"));
        assert!(contents.contains("event=request_failed"));
        assert!(contents.contains("http_status=429 ignored"));
        assert!(contents.contains("component=agent"));
        assert_eq!(contents.matches("\n- `timestamp_ms=").count(), 2);
    }
}
