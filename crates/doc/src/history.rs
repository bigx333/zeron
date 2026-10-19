//! Provider continuity and small checkpoint receipts. Transcript backups and
//! filesystem objects live on the owning host; restoring writes ordinary CRDT
//! list operations so existing viewers converge without a second timeline.
use crate::{DocError, SessionDoc};
use loro::{LoroValue, ToJson, ValueOrContainer};
use zeron_proto::{HarnessId, ProviderPoint, ProviderSessionState, RunCheckpoint};

impl SessionDoc {
    pub fn acknowledge_task_input(&self, request_id: &str) -> Result<(), DocError> {
        self.doc()
            .get_map("taskInputAcks")
            .insert(request_id, true)?;
        self.doc().commit();
        Ok(())
    }
    pub fn task_input_acknowledged(&self, request_id: &str) -> bool {
        matches!(
            self.doc().get_map("taskInputAcks").get(request_id),
            Some(ValueOrContainer::Value(LoroValue::Bool(true)))
        )
    }
    /// A user's Stop holds automatic child updates until an explicit new send.
    /// Store this outside the transcript so it survives process restarts/rewinds.
    pub fn child_updates_paused(&self) -> bool {
        matches!(
            self.doc().get_map("meta").get("childUpdatesPaused"),
            Some(ValueOrContainer::Value(LoroValue::Bool(true)))
        )
    }
    pub fn set_child_updates_paused(&self, paused: bool) -> Result<(), DocError> {
        self.doc()
            .get_map("meta")
            .insert("childUpdatesPaused", paused)?;
        self.doc().commit();
        Ok(())
    }
    pub fn history_was_restored(&self) -> bool {
        self.doc()
            .get_map("meta")
            .get("restoredCheckpoint")
            .is_some()
    }
    pub fn history_revision(&self) -> Option<String> {
        match self.doc().get_map("meta").get("historyRevision") {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => Some(s.to_string()),
            _ => None,
        }
    }
    pub fn mark_history_restore(
        &self,
        id: &str,
        revision: &str,
        harness: Option<HarnessId>,
    ) -> Result<(), DocError> {
        let meta = self.doc().get_map("meta");
        meta.insert("restoredCheckpoint", id)?;
        meta.insert("historyRevision", revision)?;
        if let Some(harness) = harness {
            meta.insert("providerHarness", serde_json::to_string(&harness)?)?;
        }
        self.doc().commit();
        Ok(())
    }
    pub fn provider_sessions(&self) -> Vec<ProviderSessionState> {
        match self.doc().get_map("meta").get("providerSessions") {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => {
                serde_json::from_str(&s).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }
    pub fn set_provider_sessions(&self, states: &[ProviderSessionState]) -> Result<(), DocError> {
        self.doc()
            .get_map("meta")
            .insert("providerSessions", serde_json::to_string(states)?)?;
        self.doc().commit();
        Ok(())
    }
    pub fn remember_provider(
        &self,
        point: ProviderPoint,
        message: Option<&str>,
    ) -> Result<(), DocError> {
        let mut states = self.provider_sessions();
        let previous = states
            .iter()
            .find(|s| s.point.harness == point.harness && s.point.session_id == point.session_id)
            .cloned();
        states.retain(|s| !(s.point.harness == point.harness && s.point.cwd == point.cwd));
        let mut point = point;
        if point.turn_id.is_none() {
            point.turn_id = previous.as_ref().and_then(|s| s.point.turn_id.clone());
        }
        states.push(ProviderSessionState {
            point,
            context_ready: previous.as_ref().is_some_and(|s| s.context_ready),
            last_message_id: message
                .map(str::to_owned)
                .or_else(|| previous.and_then(|s| s.last_message_id)),
        });
        self.set_provider_sessions(&states)
    }
    pub fn mark_provider_context(&self, session: &str) -> Result<(), DocError> {
        let mut states = self.provider_sessions();
        for state in &mut states {
            if state.point.session_id == session {
                state.context_ready = true;
            }
        }
        self.set_provider_sessions(&states)
    }
    pub fn provider_session(&self, harness: HarnessId, cwd: &str) -> Option<ProviderSessionState> {
        self.provider_sessions().into_iter().rev().find(|s| {
            s.point.harness == harness
                && (s.point.cwd == cwd
                    || std::fs::canonicalize(cwd)
                        .is_ok_and(|a| std::fs::canonicalize(&s.point.cwd).is_ok_and(|b| a == b)))
        })
    }
    pub fn run_checkpoints(&self) -> Result<Vec<RunCheckpoint>, DocError> {
        let value = self
            .doc()
            .get_map("runCheckpoints")
            .get_deep_value()
            .to_json_value();
        let mut points: Vec<RunCheckpoint> = value
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
            .filter_map(|v| v.as_str())
            .filter_map(|s| serde_json::from_str(s).ok())
            .collect();
        points.sort_by(|a, b| {
            b.created_at_ms
                .cmp(&a.created_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(points)
    }
    pub fn record_run_checkpoint(&self, point: &RunCheckpoint) -> Result<(), DocError> {
        self.doc()
            .get_map("runCheckpoints")
            .insert(&point.id, serde_json::to_string(point)?)?;
        self.doc().commit();
        Ok(())
    }
}
