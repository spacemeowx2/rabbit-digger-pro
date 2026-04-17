use std::process::Command;

use serde::{Deserialize, Serialize};

/// A reversible system side effect.
/// Each entry pairs a description with its undo operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideEffectEntry {
    pub description: String,
    pub undo: SideEffectUndo,
}

/// How to undo a side effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SideEffectUndo {
    /// Run a command to undo.
    Command { cmd: String, args: Vec<String> },
    /// Write content back to a file.
    WriteFile { path: String, content: String },
    /// Delete a file.
    DeleteFile { path: String },
}

impl SideEffectUndo {
    pub fn execute(&self) -> Result<(), String> {
        match self {
            SideEffectUndo::Command { cmd, args } => {
                let output = Command::new(cmd)
                    .args(args)
                    .output()
                    .map_err(|e| format!("{cmd}: {e}"))?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("{cmd} {}: {}", args.join(" "), stderr.trim()));
                }
                Ok(())
            }
            SideEffectUndo::WriteFile { path, content } => {
                std::fs::write(path, content).map_err(|e| format!("write {path}: {e}"))
            }
            SideEffectUndo::DeleteFile { path } => {
                std::fs::remove_file(path).map_err(|e| format!("delete {path}: {e}"))
            }
        }
    }
}

/// Manages system side effects with automatic rollback.
///
/// Every side effect is persisted to disk immediately after it's applied.
/// On normal shutdown (Drop), all effects are rolled back in reverse order.
/// On crash recovery, the daemon reads the persisted file and rolls back.
#[derive(Debug)]
pub struct SideEffectManager {
    entries: Vec<SideEffectEntry>,
    persist_path: Option<String>,
}

impl SideEffectManager {
    /// Create a new manager that persists to the given file path.
    pub fn new(persist_path: impl Into<String>) -> Self {
        SideEffectManager {
            entries: Vec::new(),
            persist_path: Some(persist_path.into()),
        }
    }

    /// Create a manager without persistence (for tests).
    pub fn in_memory() -> Self {
        SideEffectManager {
            entries: Vec::new(),
            persist_path: None,
        }
    }

    /// Apply a side effect. The closure runs the action; if it succeeds,
    /// the undo is registered and persisted immediately.
    pub fn apply(
        &mut self,
        description: impl Into<String>,
        action: impl FnOnce() -> Result<(), String>,
        undo: SideEffectUndo,
    ) -> Result<(), String> {
        let description = description.into();
        action()?;
        self.entries.push(SideEffectEntry { description, undo });
        self.persist();
        Ok(())
    }

    /// Roll back all side effects in reverse order.
    pub fn rollback_all(&mut self) {
        while let Some(entry) = self.entries.pop() {
            tracing::debug!("Rolling back: {}", entry.description);
            if let Err(e) = entry.undo.execute() {
                tracing::warn!("Rollback failed for '{}': {}", entry.description, e);
            }
        }
        self.clear_persisted();
    }

    /// Load persisted side effects from disk and roll them back.
    /// Used for crash recovery on daemon startup.
    pub fn recover(persist_path: &str) {
        let content = match std::fs::read_to_string(persist_path) {
            Ok(c) if !c.is_empty() => c,
            _ => return,
        };

        let entries: Vec<SideEffectEntry> = match serde_json::from_str(&content) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to parse side effects file: {e}");
                let _ = std::fs::remove_file(persist_path);
                return;
            }
        };

        if entries.is_empty() {
            let _ = std::fs::remove_file(persist_path);
            return;
        }

        tracing::info!(
            "Recovering {} side effects from previous run",
            entries.len()
        );

        // Roll back in reverse order
        for entry in entries.iter().rev() {
            tracing::debug!("Recovering: {}", entry.description);
            if let Err(e) = entry.undo.execute() {
                tracing::warn!(
                    "Recovery rollback failed for '{}': {}",
                    entry.description,
                    e
                );
            }
        }

        let _ = std::fs::remove_file(persist_path);
        tracing::info!("Recovery complete");
    }

    fn persist(&self) {
        if let Some(path) = &self.persist_path {
            if let Ok(json) = serde_json::to_string_pretty(&self.entries) {
                if let Err(e) = std::fs::write(path, json) {
                    tracing::warn!("Failed to persist side effects: {e}");
                }
            }
        }
    }

    fn clear_persisted(&self) {
        if let Some(path) = &self.persist_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for SideEffectManager {
    fn drop(&mut self) {
        self.rollback_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn test_apply_and_rollback() {
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let mut mgr = SideEffectManager::in_memory();

        mgr.apply(
            "increment counter",
            || {
                COUNTER.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            SideEffectUndo::Command {
                cmd: "true".into(),
                args: vec![],
            },
        )
        .unwrap();

        assert_eq!(COUNTER.load(Ordering::SeqCst), 1);
        assert_eq!(mgr.entries.len(), 1);

        mgr.rollback_all();
        assert_eq!(mgr.entries.len(), 0);
    }

    #[test]
    fn test_apply_failure_does_not_register() {
        let mut mgr = SideEffectManager::in_memory();

        let result = mgr.apply(
            "will fail",
            || Err("nope".into()),
            SideEffectUndo::Command {
                cmd: "true".into(),
                args: vec![],
            },
        );

        assert!(result.is_err());
        assert_eq!(mgr.entries.len(), 0);
    }

    #[test]
    fn test_drop_triggers_rollback() {
        let persist_path = std::env::temp_dir().join(format!(
            "rdp-test-se-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let persist_path_str = persist_path.to_string_lossy().to_string();

        {
            let mut mgr = SideEffectManager::new(&persist_path_str);
            mgr.apply(
                "test",
                || Ok(()),
                SideEffectUndo::Command {
                    cmd: "true".into(),
                    args: vec![],
                },
            )
            .unwrap();

            // File should exist while manager is alive
            assert!(std::fs::read_to_string(&persist_path).is_ok());
        }
        // After drop, file should be cleaned up
        assert!(!persist_path.exists());
    }
}
