//! Exact native forks use the installed app-server's inclusive `lastTurnId`.
//! No prompt is sent and the source native conversation remains unchanged.
use super::*;

impl CodexHarness {
    pub(super) async fn fork_history(
        &self,
        source: &zeron_proto::ProviderPoint,
    ) -> Result<String, HarnessError> {
        let turn = source
            .turn_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                HarnessError::Protocol(
                    "Native fork needs a completed provider turn reference".into(),
                )
            })?;
        let exe = self.resolve_executable()?;
        let mut command = Command::new(&exe);
        command
            .arg("app-server")
            .current_dir(&source.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        crate::compose_child_path(&mut command, &exe);
        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HarnessError::Protocol("Missing app-server stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HarnessError::Protocol("Missing app-server stdout".into()))?;
        let (client, mut incoming) = RpcClient::new(stdin, stdout);
        // Drain unsolicited notifications; decline all server requests. This
        // control-only operation must never approve a side effect.
        let receiver = client.clone();
        let drain = tokio::spawn(async move {
            while let Some(event) = incoming.recv().await {
                if let Incoming::Request { id, .. } = event {
                    receiver.respond_error(
                        &id,
                        -32601,
                        "History operation does not accept approvals",
                    );
                }
            }
        });
        let operation = async {
            client.request("initialize", json!({"clientInfo":{"name":"zeron-history","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
            client.notify("initialized", None);
            let result = client.request("thread/fork", json!({"threadId":source.session_id,"lastTurnId":turn,"cwd":source.cwd,"excludeTurns":true})).await?;
            let id = result["thread"]["id"]
                .as_str()
                .filter(|id| !id.is_empty() && *id != source.session_id)
                .ok_or_else(|| {
                    HarnessError::Protocol("Native fork returned no independent thread".into())
                })?;
            // Older CLIs may accept unknown input fields. Verify the new
            // thread actually ends at the requested turn before using it.
            let latest = client
                .request(
                    "thread/turns/list",
                    json!({"threadId":id,"limit":1,"sortDirection":"desc","itemsView":"notLoaded"}),
                )
                .await?;
            if latest["data"][0]["id"].as_str() != Some(turn) {
                return Err(HarnessError::Protocol(
                    "Provider did not honor the requested native fork boundary".into(),
                ));
            }
            Ok(id.to_owned())
        };
        let result = tokio::time::timeout(Duration::from_secs(30), operation)
            .await
            .map_err(|_| HarnessError::Protocol("Native fork timed out".into()))
            .and_then(|r| r);
        shutdown_child(&mut child, self.kill_grace).await;
        drain.abort();
        result
    }
}
