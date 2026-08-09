use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncApiFixture {
    pub spec: serde_json::Value,
    #[serde(default)]
    pub expected: serde_json::Value,
    #[serde(default)]
    pub validation: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketFixture {
    pub handler: WebSocketHandler,
    pub session: WebSocketSession,
    #[serde(default)]
    pub expected: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketHandler {
    pub route: String,
    pub behavior: String,
    #[serde(default)]
    pub message_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub on_connect_action: Option<String>,
    #[serde(default)]
    pub on_disconnect_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketSession {
    #[serde(default)]
    pub messages: Vec<WebSocketMessage>,
    #[serde(default)]
    pub close_code: Option<u16>,
    #[serde(default)]
    pub abnormal_close: bool,
    #[serde(default)]
    pub expected_receive_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketMessage {
    pub direction: WebSocketMessageDirection,
    #[serde(default)]
    pub frame_type: WebSocketFrameType,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
    #[serde(default)]
    pub raw_text: Option<String>,
    #[serde(default)]
    pub payload_base64: Option<String>,
    #[serde(default)]
    pub close_code: Option<u16>,
    #[serde(default)]
    pub close_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketMessageDirection {
    Send,
    Receive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketFrameType {
    #[default]
    Json,
    Text,
    Binary,
    Ping,
    Pong,
    Close,
}

#[cfg(test)]
mod tests {
    use super::{AsyncApiFixture, WebSocketFixture, WebSocketFrameType, WebSocketMessageDirection};

    #[test]
    fn asyncapi_operation_fixture_keeps_spec_and_validation_recipe() {
        let fixture: AsyncApiFixture = serde_json::from_value(serde_json::json!({
            "spec": {"asyncapi": "3.0.0", "operations": {"publishEvent": {"action": "send"}}},
            "validation": {"message": "Event", "payload": {"id": 7}},
            "expected": {"valid": true}
        }))
        .expect("AsyncAPI fixture is valid");

        assert_eq!(fixture.spec["asyncapi"], "3.0.0");
        assert_eq!(fixture.validation.expect("validation recipe")["message"], "Event");
    }

    #[test]
    fn websocket_session_keeps_ordered_protocol_actions() {
        let fixture: WebSocketFixture = serde_json::from_value(serde_json::json!({
            "handler": {"route": "/events", "behavior": "echo"},
            "session": {"messages": [
                {"direction": "send", "payload": {"event": "ready"}},
                {"direction": "receive", "frame_type": "binary", "payload_base64": "AQI="}
            ], "close_code": 1000}
        }))
        .expect("WebSocket fixture is valid");

        assert_eq!(fixture.handler.route, "/events");
        assert_eq!(fixture.session.messages[0].direction, WebSocketMessageDirection::Send);
        assert_eq!(fixture.session.messages[1].frame_type, WebSocketFrameType::Binary);
    }
}
