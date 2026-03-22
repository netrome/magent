use std::path::Path;
use std::path::PathBuf;

use notify::{EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;

/// Start watching `dir` recursively for `.md` file changes.
///
/// Returns a receiver that yields the path of each changed markdown file.
/// The `notify` watcher is returned alongside — it must be kept alive for
/// events to keep flowing.
pub fn start(
    dir: &Path,
    tx: mpsc::Sender<PathBuf>,
) -> Result<impl Watcher + use<>, Box<dyn std::error::Error>> {
    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            let Ok(event) = res else {
                return;
            };

            if !is_content_change(&event.kind) {
                return;
            }

            for path in event.paths {
                if is_markdown(&path) {
                    // If the receiver is dropped, stop sending silently.
                    let _ = tx.blocking_send(path);
                }
            }
        })?;

    watcher.watch(dir, RecursiveMode::Recursive)?;
    Ok(watcher)
}

fn is_content_change(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Modify(notify::event::ModifyKind::Data(_))
            | EventKind::Modify(notify::event::ModifyKind::Name(_))
    )
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "md")
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn start__should_detect_markdown_file_changes() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("test.md");
        let (tx, mut rx) = mpsc::channel(16);
        let _watcher = start(dir.path(), tx).unwrap();

        // When
        std::fs::write(&md_path, "hello").unwrap();

        // Then
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
        assert_eq!(event.unwrap().unwrap(), md_path);
    }

    #[tokio::test]
    async fn start__should_ignore_non_markdown_files() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let txt_path = dir.path().join("test.txt");
        let md_path = dir.path().join("test.md");
        let (tx, mut rx) = mpsc::channel(16);
        let _watcher = start(dir.path(), tx).unwrap();

        // When — write a .txt file first, then a .md file
        std::fs::write(&txt_path, "ignored").unwrap();
        std::fs::write(&md_path, "seen").unwrap();

        // Then — only the .md file should come through
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
        assert_eq!(event.unwrap().unwrap(), md_path);
    }

    #[tokio::test]
    async fn start__should_detect_atomic_writes() {
        // Given — simulates how mindex saves files (write tmp + rename)
        let dir = tempfile::tempdir().unwrap();
        let tmp_path = dir.path().join(".test.md.tmp");
        let md_path = dir.path().join("test.md");
        let (tx, mut rx) = mpsc::channel(16);
        let _watcher = start(dir.path(), tx).unwrap();

        // When
        std::fs::write(&tmp_path, "atomic content").unwrap();
        std::fs::rename(&tmp_path, &md_path).unwrap();

        // Then
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
        assert_eq!(event.unwrap().unwrap(), md_path);
    }

    #[tokio::test]
    async fn start__should_detect_changes_in_subdirectories() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let sub_dir = dir.path().join("subdir");
        std::fs::create_dir(&sub_dir).unwrap();
        let md_path = sub_dir.join("nested.md");
        let (tx, mut rx) = mpsc::channel(16);
        let _watcher = start(dir.path(), tx).unwrap();

        // When
        std::fs::write(&md_path, "nested content").unwrap();

        // Then
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
        assert_eq!(event.unwrap().unwrap(), md_path);
    }
}
