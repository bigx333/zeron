//! Host-routed orchestration controls shared by the native mobile apps.
use crate::{Client, ClientError, error::Result};
use serde_json::json;
use zeron_rpc::methods;

impl Client {
    pub async fn chat_merge_receipts(
        &self,
        chat_id: &str,
    ) -> Result<Vec<zeron_proto::ContextTransfer>> {
        let row = self
            .workspace()
            .session(chat_id)
            .cloned()
            .ok_or_else(|| ClientError::NotFound("Chat no longer exists".into()))?;
        let reply = self
            .inner
            .host_rpc(
                &row.device_id,
                methods::CHAT_TRANSFERS,
                json!({"chatId":chat_id,"targetDeviceId":row.device_id}),
            )
            .await?;
        let transfers: Vec<zeron_proto::ContextTransfer> =
            serde_json::from_value(reply).map_err(|e| ClientError::Internal(e.to_string()))?;
        Ok(transfers
            .into_iter()
            .filter(|transfer| transfer.kind == "merge")
            .collect())
    }
    pub async fn chat_checkpoints(&self, chat_id: &str) -> Result<Vec<zeron_proto::RunCheckpoint>> {
        let row = self
            .workspace()
            .session(chat_id)
            .cloned()
            .ok_or_else(|| ClientError::NotFound("Chat no longer exists".into()))?;
        let reply = self
            .inner
            .host_rpc(
                &row.device_id,
                methods::CHAT_CHECKPOINTS,
                json!({"chatId":chat_id,"targetDeviceId":row.device_id}),
            )
            .await?;
        serde_json::from_value(reply).map_err(|e| ClientError::Internal(e.to_string()))
    }
    pub async fn create_chat_checkpoint(
        &self,
        chat_id: &str,
    ) -> Result<zeron_proto::RunCheckpoint> {
        let row = self
            .workspace()
            .session(chat_id)
            .cloned()
            .ok_or_else(|| ClientError::NotFound("Chat no longer exists".into()))?;
        let reply = self
            .inner
            .host_rpc(
                &row.device_id,
                methods::CREATE_CHAT_CHECKPOINT,
                json!({"chatId":chat_id,"targetDeviceId":row.device_id}),
            )
            .await?;
        serde_json::from_value(reply).map_err(|e| ClientError::Internal(e.to_string()))
    }
    pub async fn rollback_chat(
        &self,
        chat_id: &str,
        checkpoint_id: &str,
        request_id: Option<&str>,
    ) -> Result<String> {
        let row = self
            .workspace()
            .session(chat_id)
            .cloned()
            .ok_or_else(|| ClientError::NotFound("Chat no longer exists".into()))?;
        let reply = self.inner.host_rpc(&row.device_id, methods::ROLLBACK_CHAT, json!({"chatId":chat_id,"checkpointId":checkpoint_id,"clientRequestId":request_id,"targetDeviceId":row.device_id})).await?;
        reply["recoveryCheckpointId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| ClientError::Internal("Restore returned no recovery checkpoint".into()))
    }
    pub async fn cancel_task(&self, parent: &str, task: &str) -> Result<()> {
        let workspace = self.workspace();
        let row = workspace
            .session(task)
            .ok_or_else(|| ClientError::NotFound("Task no longer exists".into()))?;
        if !row
            .task
            .as_ref()
            .is_some_and(|t| t.parent_chat_id == parent)
        {
            return Err(ClientError::InvalidArgument(
                "Task belongs to another chat".into(),
            ));
        }
        self.inner
            .host_rpc(
                &row.device_id,
                methods::TASK_CANCEL,
                json!({"parentChatId":parent,"taskId":task,"targetDeviceId":row.device_id}),
            )
            .await?;
        Ok(())
    }
    pub async fn merge_chat(&self, source: &str) -> Result<String> {
        let workspace = self.workspace();
        let row = workspace
            .session(source)
            .ok_or_else(|| ClientError::NotFound("Source chat no longer exists".into()))?;
        let reply = self
            .inner
            .host_rpc(
                &row.device_id,
                methods::MERGE_CHAT,
                json!({"sourceChatId":source,"targetDeviceId":row.device_id}),
            )
            .await?;
        reply["transferId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| ClientError::Internal("Merge returned no receipt".into()))
    }
    pub async fn fork_session(&self, source: &str) -> Result<String> {
        let workspace = self.workspace();
        let row = workspace
            .session(source)
            .ok_or_else(|| ClientError::NotFound("Source chat no longer exists".into()))?;
        let id = uuid::Uuid::new_v4().to_string();
        self.inner.host_rpc(&row.device_id,methods::FORK_SIDE_CHAT,json!({"chatId":id,"sourceChatId":source,"parentChatId":row.parent_chat_id.as_deref().unwrap_or(source),"targetDeviceId":row.device_id})).await?;
        Ok(id)
    }
}
