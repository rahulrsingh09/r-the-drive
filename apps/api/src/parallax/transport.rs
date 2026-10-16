//! Read-only subscription protocol and retry decisions, independent of storage.
use super::*;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

pub(super) fn connection_request(tokens: &RivianTokenBundle) -> Result<http::Request<()>> {
    let mut request = WS_URL.into_client_request()?;
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", "graphql-transport-ws".parse()?);
    for (name, value, error) in [
        (
            "A-Sess",
            tokens.app_session_token.as_str(),
            "invalid Rivian app session header",
        ),
        (
            "U-Sess",
            tokens.user_session_token.as_str(),
            "invalid Rivian user session header",
        ),
    ] {
        request
            .headers_mut()
            .insert(name, value.parse().context(error)?);
    }
    if !tokens.csrf_token.is_empty() {
        request.headers_mut().insert(
            "Csrf-Token",
            tokens
                .csrf_token
                .parse()
                .context("invalid Rivian CSRF header")?,
        );
    }
    Ok(request)
}

pub(super) fn connection_init(tokens: &RivianTokenBundle, client_id: Uuid) -> Message {
    Message::Text(
        json!({
            "type": "connection_init",
            "payload": {
                "client-name": "com.rivian.ios.consumer-apollo-ios",
                "client-version": "1.13.0-1494",
                "dc-cid": format!("m-ios-{client_id}"),
                "u-sess": tokens.user_session_token,
            }
        })
        .to_string()
        .into(),
    )
}

pub(super) enum SocketAction {
    Reply(Message),
    Envelope(Value),
    Rejected,
    Completed,
    Closed(Option<CloseFrame>),
    Ignore,
}

impl SocketAction {
    pub fn classify(message: Message) -> Self {
        match message {
            Message::Ping(payload) => Self::Reply(Message::Pong(payload)),
            Message::Close(frame) => Self::Closed(frame),
            Message::Text(text) => {
                let mut frame: Value = serde_json::from_str(&text).unwrap_or_default();
                if frame.get("id").and_then(Value::as_str) != Some(SUBSCRIPTION_ID) {
                    return Self::Ignore;
                }
                match frame.get("type").and_then(Value::as_str) {
                    Some("next") => frame
                        .pointer_mut("/payload/data/parallaxMessages")
                        .map(|envelope| Self::Envelope(envelope.take()))
                        .unwrap_or(Self::Ignore),
                    Some("error") => Self::Rejected,
                    Some("complete") => Self::Completed,
                    _ => Self::Ignore,
                }
            }
            _ => Self::Ignore,
        }
    }
}

pub(super) struct RetrySchedule {
    seconds: u64,
}
impl Default for RetrySchedule {
    fn default() -> Self {
        Self { seconds: 2 }
    }
}
impl RetrySchedule {
    pub fn delay(&self) -> Duration {
        Duration::from_secs(self.seconds)
    }
    pub fn advance(&mut self) {
        self.seconds = (self.seconds * 2).min(120);
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

pub(super) async fn wait_to_retry(shutdown: &mut broadcast::Receiver<()>, delay: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        _ = shutdown.recv() => false,
    }
}
