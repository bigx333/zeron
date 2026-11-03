//! Provider input tasks are owned by their wire request, not detached from it.
use std::collections::HashMap;
use std::future::Future;

#[derive(Default)]
pub(crate) struct InputTasks {
    tasks: HashMap<String, tokio::task::JoinHandle<()>>,
}

impl InputTasks {
    pub fn spawn(&mut self, id: String, task: impl Future<Output = ()> + Send + 'static) {
        self.tasks.retain(|_, task| !task.is_finished());
        if let Some(previous) = self.tasks.insert(id, tokio::spawn(task)) {
            previous.abort();
        }
    }

    pub fn cancel(&mut self, id: &str) {
        if let Some(task) = self.tasks.remove(id) {
            // Dropping request_input's receiver lets the engine resolve its input.
            task.abort();
        }
    }
}

impl Drop for InputTasks {
    fn drop(&mut self) {
        for task in self.tasks.values() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_one_request_and_teardown_close_only_owned_receivers() {
        let (mut first, first_rx) = tokio::sync::oneshot::channel::<()>();
        let (mut second, second_rx) = tokio::sync::oneshot::channel::<()>();
        let mut tasks = InputTasks::default();
        tasks.spawn("first".into(), async move {
            let _ = first_rx.await;
        });
        tasks.spawn("second".into(), async move {
            let _ = second_rx.await;
        });
        tasks.cancel("first");
        tokio::time::timeout(std::time::Duration::from_secs(1), first.closed())
            .await
            .unwrap();
        assert!(!second.is_closed());
        drop(tasks);
        tokio::time::timeout(std::time::Duration::from_secs(1), second.closed())
            .await
            .unwrap();
    }
}
