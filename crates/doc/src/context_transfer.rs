//! Bounded app-owned context artifacts, stored alongside the conversation.
use crate::{DocError, SessionDoc};
use loro::{LoroValue, ValueOrContainer};
use zeron_proto::{ContextTransfer, HarnessId};

impl SessionDoc {
    pub fn context_transfers(&self) -> Result<Vec<ContextTransfer>, DocError> {
        match self.doc().get_map("meta").get("contextTransfers") {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => Ok(serde_json::from_str(&s)?),
            _ => Ok(Vec::new()),
        }
    }
    pub fn record_context_transfer(&self, transfer: ContextTransfer) -> Result<(), DocError> {
        let mut transfers = self.context_transfers()?;
        if transfers.iter().any(|t| t.id == transfer.id) {
            return Ok(());
        }
        transfers.push(transfer);
        // Keep recent artifacts inspectable without unbounded meta growth.
        if transfers.len() > 32 {
            transfers.drain(..transfers.len() - 32);
        }
        self.doc()
            .get_map("meta")
            .insert("contextTransfers", serde_json::to_string(&transfers)?)?;
        self.doc().commit();
        Ok(())
    }
    pub fn consume_context_transfers(&self, harness: HarnessId) -> Result<(), DocError> {
        let mut transfers = self.context_transfers()?;
        let mut changed = false;
        for transfer in &mut transfers {
            if transfer.target_harness == Some(harness) && transfer.status == "prepared" {
                transfer.status = "consumed".into();
                changed = true;
            }
        }
        if changed {
            self.doc()
                .get_map("meta")
                .insert("contextTransfers", serde_json::to_string(&transfers)?)?;
            self.doc().commit();
        }
        self.doc()
            .get_map("meta")
            .insert("providerHarness", serde_json::to_string(&harness)?)?;
        self.doc().commit();
        Ok(())
    }
    pub fn provider_harness(&self) -> Option<HarnessId> {
        match self.doc().get_map("meta").get("providerHarness") {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => serde_json::from_str(&s).ok(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artifacts_survive_snapshot_and_consume_only_the_selected_provider() {
        let doc = SessionDoc::init("chat").unwrap();
        for i in 0..34 {
            doc.record_context_transfer(ContextTransfer {
                id: i.to_string(),
                kind: "provider_handoff".into(),
                source_chat_id: "chat".into(),
                source_message_id: Some("source".into()),
                target_harness: Some(if i % 2 == 0 {
                    HarnessId::Codex
                } else {
                    HarnessId::ClaudeCode
                }),
                created_at: i,
                status: "prepared".into(),
                content: "Visible context: 漢字 🛰️".into(),
                truncated: true,
            })
            .unwrap();
        }
        doc.consume_context_transfers(HarnessId::Codex).unwrap();
        let loro = loro::LoroDoc::new();
        loro.import(&doc.export_snapshot().unwrap()).unwrap();
        let restored = SessionDoc::from_doc(loro);
        let transfers = restored.context_transfers().unwrap();
        assert_eq!(transfers.len(), 32);
        assert_eq!(transfers[0].id, "2");
        assert!(
            transfers
                .iter()
                .filter(|t| t.target_harness == Some(HarnessId::Codex))
                .all(|t| t.status == "consumed")
        );
        assert!(
            transfers
                .iter()
                .filter(|t| t.target_harness == Some(HarnessId::ClaudeCode))
                .all(|t| t.status == "prepared")
        );
        assert_eq!(restored.provider_harness(), Some(HarnessId::Codex));
    }
}
