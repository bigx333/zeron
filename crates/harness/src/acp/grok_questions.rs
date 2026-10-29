//! Grok's blocking ACP question extension (captured with Grok 1.0.60).
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use zeron_proto::{UserInputAnswer, UserInputQuestion};

use super::{RequestInputFn, RpcClient};
use crate::{CancellationToken, input_tasks::InputTasks};

pub(super) const METHOD: &str = "_x.ai/ask_user_question";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    session_id: String,
    tool_call_id: String,
    questions: Vec<Question>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Question {
    question: String,
    #[serde(default)]
    options: Vec<QuestionOption>,
    #[serde(default)]
    multi_select: Option<bool>,
}

#[derive(Deserialize)]
struct QuestionOption {
    label: String,
    #[serde(default)]
    preview: Option<String>,
}

impl Request {
    fn questions(&self) -> Vec<UserInputQuestion> {
        self.questions
            .iter()
            .map(|q| UserInputQuestion {
                id: super::new_message_id(),
                // Grok has no separate header field on this wire; an empty
                // header lets each client apply its own fallback label.
                header: String::new(),
                question: q.question.clone(),
                options: q.options.iter().map(|o| o.label.clone()).collect(),
                multi_select: q.multi_select.unwrap_or(false),
                prefill: None,
                multiline: false,
            })
            .collect()
    }

    fn response(&self, asked: &[UserInputQuestion], answers: &[UserInputAnswer]) -> Value {
        let mut by_question = serde_json::Map::new();
        let mut annotations = serde_json::Map::new();
        for (wire, question) in self.questions.iter().zip(asked) {
            let Some(answer) = answers.iter().find(|a| a.question_id == question.id) else {
                continue;
            };
            let mut selected = Vec::new();
            let mut notes = Vec::new();
            for label in answer
                .labels
                .iter()
                .filter(|label| !label.trim().is_empty())
            {
                if wire.options.iter().any(|o| o.label == *label) {
                    selected.push(label.clone());
                } else {
                    notes.push(label.clone());
                }
            }
            let mut annotation = serde_json::Map::new();
            if !wire.multi_select.unwrap_or(false)
                && selected.len() == 1
                && let Some(preview) = wire
                    .options
                    .iter()
                    .find(|o| o.label == selected[0])
                    .and_then(|o| o.preview.as_ref())
            {
                annotation.insert("preview".into(), json!(preview));
            }
            if !notes.is_empty() {
                selected.push("Other".into());
                annotation.insert("notes".into(), json!(notes.join(", ")));
            }
            if !selected.is_empty() {
                by_question.insert(wire.question.clone(), json!(selected));
                if !annotation.is_empty() {
                    annotations.insert(wire.question.clone(), json!(annotation));
                }
            }
        }
        if by_question.is_empty() {
            return json!({"outcome":"cancelled"});
        }
        let mut response = json!({"outcome":"accepted", "answers":by_question});
        if !annotations.is_empty() {
            response["annotations"] = json!(annotations);
        }
        response
    }
}

#[derive(Default)]
pub(super) struct Questions {
    tasks: InputTasks,
    cancel: CancellationToken,
}

impl Questions {
    pub(super) fn request(
        &mut self,
        client: &RpcClient,
        id: Value,
        params: &Value,
        request_input: &Arc<RequestInputFn>,
        session_id: &str,
    ) {
        let request: Request = match serde_json::from_value(params.clone()) {
            Ok(request) => request,
            Err(_) => {
                client.respond_error(&id, -32602, "invalid Grok question request");
                return;
            }
        };
        if request.session_id != session_id {
            client.respond(&id, json!({"outcome":"cancelled"}));
            return;
        }
        if request.tool_call_id.is_empty()
            || request.questions.is_empty()
            || request
                .questions
                .iter()
                .any(|q| q.question.trim().is_empty())
        {
            client.respond_error(&id, -32602, "empty Grok question request");
            return;
        }
        let client = client.clone();
        let bridge = Arc::clone(request_input);
        let cancel = self.cancel.clone();
        self.tasks.spawn(request.tool_call_id.clone(), async move {
            let asked = request.questions();
            let answers = tokio::select! {
                _ = cancel.cancelled() => Vec::new(),
                answers = bridge(asked.clone()) => answers.unwrap_or_default(),
            };
            client.respond(&id, request.response(&asked, &answers));
        });
    }

    pub(super) fn notification(&mut self, method: &str, params: &Value) {
        let update = &params["update"];
        let tool = if method == "_x.ai/session_notification"
            && update["sessionUpdate"] == "interaction_resolved"
        {
            update["tool_call_id"].as_str()
        } else if method == "session/update"
            && update["sessionUpdate"] == "tool_call_update"
            && matches!(update["status"].as_str(), Some("completed" | "failed"))
        {
            update["toolCallId"].as_str()
        } else {
            None
        };
        if let Some(tool) = tool {
            // Grok owns its configured timeout (including "wait forever").
            // Its resolution notification withdraws the UI without a late reply.
            self.tasks.cancel(tool);
        }
    }

    pub(super) fn cancel_all(&mut self) {
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_questions_and_accepted_response_round_trip() {
        let capture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/grok-question-request.json"
        ))
        .unwrap();
        let request: Request = serde_json::from_value(capture["params"].clone()).unwrap();
        let questions = request.questions();
        assert_eq!(questions[0].question, "Color");
        assert_eq!(questions[0].options, ["Blue", "Green"]);
        assert!(!questions[0].multi_select);
        assert!(questions[1].multi_select);
        let answers = vec![
            UserInputAnswer {
                question_id: questions[0].id.clone(),
                labels: vec!["Blue".into()],
            },
            UserInputAnswer {
                question_id: questions[1].id.clone(),
                labels: vec!["Apple".into(), "Pear".into()],
            },
        ];
        let expected: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/grok-question-response.json"
        ))
        .unwrap();
        assert_eq!(request.response(&questions, &answers), expected);
        assert_eq!(
            request.response(&questions, &[]),
            json!({"outcome":"cancelled"})
        );
        let free_text = vec![UserInputAnswer {
            question_id: questions[0].id.clone(),
            labels: vec!["violet instead".into()],
        }];
        assert_eq!(
            request.response(&questions, &free_text),
            json!({
                "outcome":"accepted", "answers":{"Color":["Other"]},
                "annotations":{"Color":{"notes":"violet instead"}}
            })
        );
    }
}
