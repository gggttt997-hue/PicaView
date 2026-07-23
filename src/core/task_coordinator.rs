use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::AbortHandle;
use tracing::{debug, info};

/// Backend task coordinator for managing the lifecycle of long-running tasks
#[derive(Debug)]
pub struct TaskCoordinator {
    /// Task tracker, stores task ID and its corresponding AbortHandle
    tasks: Arc<RwLock<HashMap<String, AbortHandle>>>,
}

impl TaskCoordinator {
    /// Initialize a new TaskCoordinator
    pub fn new() -> Self {
        info!("[TaskCoordinator] Init: Creating new coordinator instance");
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new task
    pub async fn register_task(&self, id: String, handle: AbortHandle) {
        let mut tasks = self.tasks.write().await;

        // If a task with the same ID already exists, abort the old task (last-request-priority strategy)
        if let Some(old_handle) = tasks.remove(&id) {
            old_handle.abort();
            debug!("[TaskCoordinator] Aborted duplicate task: {}", id);
        }

        // [Evidence] Location: TaskCoordinator::register_task
        // [Capture] id, active_count: tasks.len() + 1
        // [Rationale] Monitor changes in the active task pool size.
        debug!("[Evidence] Location: TaskCoordinator::register_task, Capture: id={}, active_count={}, Rationale: Task registration.", id, tasks.len() + 1);

        tasks.insert(id, handle);
    }

    /// Explicitly abort a task
    pub async fn abort_task(&self, id: &str) -> bool {
        let mut tasks = self.tasks.write().await;
        if let Some(handle) = tasks.remove(id) {
            handle.abort();

            // [Evidence] Location: TaskCoordinator::abort_task
            // [Capture] id, success: true, remaining: tasks.len()
            // [Rationale] Confirm physical abort was triggered successfully.
            debug!("[Evidence] Location: TaskCoordinator::abort_task, Capture: id={}, success=true, remaining={}, Rationale: Physical task abort.", id, tasks.len());
            true
        } else {
            false
        }
    }

    /// Cleanup logic after task completion
    pub async fn cleanup_task(&self, id: &str) {
        let mut tasks = self.tasks.write().await;
        tasks.remove(id);
    }

    /// Check if a task is currently running
    pub async fn is_running(&self, id: &str) -> bool {
        self.tasks.read().await.contains_key(id)
    }
}

impl Default for TaskCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_task_abort() {
        let coordinator = TaskCoordinator::new();
        let task_id = "test_task".to_string();

        // Create a dummy task that sleeps for 10 seconds
        let handle = tokio::spawn(async {
            sleep(Duration::from_secs(10)).await;
        });

        coordinator
            .register_task(task_id.clone(), handle.abort_handle())
            .await;
        assert!(coordinator.is_running(&task_id).await);

        // Abort the task
        let aborted = coordinator.abort_task(&task_id).await;

        // Expect abort to succeed
        assert!(aborted, "Should be able to successfully abort the task");
        assert!(
            !coordinator.is_running(&task_id).await,
            "Task should not be in the running map after abort"
        );
    }
}
