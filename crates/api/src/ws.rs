use crate::routes::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use futures_util::StreamExt;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.ws_gateway.subscribe();

    // Send initial hello
    if socket
        .send(Message::Text("Connected to event stream".to_string()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            Some(msg) = socket.next() => {
                if msg.is_err() { break; }
            }
            Ok(event) = rx.recv() => {
                let json = serde_json::to_string(&event).unwrap();
                if socket.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    }
}
