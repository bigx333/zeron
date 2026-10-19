use crate::{DocError, SessionDoc};
use loro::{LoroValue, ValueOrContainer};
use zeron_proto::RootRunOutcome;

impl SessionDoc {
    pub fn task_cancellation_reason(&self) -> Option<String> {
        match self.doc().get_map("meta").get("taskCancellationReason") {
            Some(ValueOrContainer::Value(LoroValue::String(reason))) => Some(reason.to_string()),
            _ => None,
        }
    }
    pub fn set_task_cancellation_reason(&self, reason: &str) -> Result<(), DocError> {
        self.doc()
            .get_map("meta")
            .insert("taskCancellationReason", reason)?;
        self.doc().commit();
        Ok(())
    }
    pub fn frozen_task_result(&self) -> Option<zeron_proto::TaskResult> {
        match self.doc().get_map("meta").get("frozenTaskResult") {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => serde_json::from_str(&s).ok(),
            _ => None,
        }
    }
    pub fn freeze_task_result(&self, result: &zeron_proto::TaskResult) -> Result<(), DocError> {
        if self.frozen_task_result().is_some() {
            return Ok(());
        }
        self.doc()
            .get_map("meta")
            .insert("frozenTaskResult", serde_json::to_string(result)?)?;
        self.doc().commit();
        Ok(())
    }
    pub fn root_run_outcome(&self) -> Option<RootRunOutcome> {
        match self.doc().get_map("meta").get("rootRunOutcome") {
            Some(ValueOrContainer::Value(LoroValue::String(value))) => {
                serde_json::from_str(&value).ok()
            }
            _ => None,
        }
    }
    pub fn record_root_run_outcome(&self, outcome: &RootRunOutcome) -> Result<(), DocError> {
        self.doc()
            .get_map("meta")
            .insert("rootRunOutcome", serde_json::to_string(outcome)?)?;
        self.doc().commit();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_brief_survives_restart_and_keeps_its_receipt_after_launch() {
        let doc = SessionDoc::init("child").unwrap();
        let brief = "A durable brief with Unicode: 漢字 🛰️".repeat(3000);
        doc.stage_task_brief("request-fingerprint", &brief).unwrap();
        let loro = loro::LoroDoc::new();
        loro.import(&doc.export_snapshot().unwrap()).unwrap();
        let restored = SessionDoc::from_doc(loro);
        assert_eq!(restored.task_brief().as_deref(), Some(brief.as_str()));
        restored.clear_task_brief().unwrap();
        assert!(restored.task_brief().is_none());
        assert!(
            restored
                .stage_task_brief("different-request", "different work")
                .is_err()
        );
        assert!(restored.task_brief().is_none());
        restored
            .stage_task_brief("request-fingerprint", &brief)
            .unwrap();
        assert_eq!(restored.task_brief().as_deref(), Some(brief.as_str()));
    }
}

impl SessionDoc {
    /// Stage a large brief in the child document before publishing its small
    /// registry row. Keep the fingerprint as a receipt after the brief clears.
    pub fn stage_task_brief(&self, fingerprint: &str, brief: &str) -> Result<(), DocError> {
        let meta = self.doc().get_map("meta");
        if let Some(ValueOrContainer::Value(LoroValue::String(existing))) =
            meta.get("delegatedTaskFingerprint")
            && existing.as_str() != fingerprint
        {
            return Err(DocError::Schema(
                "Request id was already used with a different task".into(),
            ));
        }
        meta.insert("delegatedTaskFingerprint", fingerprint)?;
        meta.insert("delegatedTaskBrief", brief)?;
        self.doc().commit();
        Ok(())
    }
    pub fn task_brief(&self) -> Option<String> {
        match self.doc().get_map("meta").get("delegatedTaskBrief") {
            Some(ValueOrContainer::Value(LoroValue::String(value))) => Some(value.to_string()),
            _ => None,
        }
    }
    pub fn clear_task_brief(&self) -> Result<(), DocError> {
        self.doc().get_map("meta").delete("delegatedTaskBrief")?;
        self.doc().commit();
        Ok(())
    }
}
