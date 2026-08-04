//! Agent-facing room-response coordination commands.

use serde_json::{json, Value};
use uuid::Uuid;

use crate::{client::BuzzClient, error::CliError, RoomCoordinationCmd};

fn scope(channel: &str, thread: &str, turn: &str) -> Result<Value, CliError> {
    let channel_id = Uuid::parse_str(channel)
        .map_err(|error| CliError::Usage(format!("invalid channel UUID: {error}")))?;
    if thread.is_empty() || turn.is_empty() {
        return Err(CliError::Usage("thread and turn must not be empty".into()));
    }
    Ok(json!({
        "channel_id": channel_id,
        "thread_id": thread,
        "turn_id": turn,
    }))
}

fn mutation_request_id(value: Option<String>) -> Result<Uuid, CliError> {
    value.map_or_else(
        || Ok(Uuid::new_v4()),
        |value| {
            Uuid::parse_str(&value)
                .map_err(|error| CliError::Usage(format!("invalid request-id UUID: {error}")))
        },
    )
}

async fn send(client: &BuzzClient, body: Value) -> Result<(), CliError> {
    let response = client
        .post_authed_json("/api/room-coordination", &body)
        .await?;
    println!("{response}");
    Ok(())
}

/// Dispatch `buzz room-coordination` commands.
pub async fn dispatch(command: RoomCoordinationCmd, client: &BuzzClient) -> Result<(), CliError> {
    let body = match command {
        RoomCoordinationCmd::Claim {
            channel,
            thread,
            turn,
            expected_version,
            request_id,
            lease_seconds,
            contribution_budget,
        } => {
            let mut body = scope(&channel, &thread, &turn)?;
            body["operation"] = json!("claim");
            body["expected_version"] = json!(expected_version);
            body["request_id"] = json!(mutation_request_id(request_id)?);
            body["lease_seconds"] = json!(lease_seconds);
            body["contribution_budget"] = json!(contribution_budget);
            body
        }
        RoomCoordinationCmd::Renew {
            channel,
            thread,
            turn,
            expected_version,
            request_id,
            lease_seconds,
        } => {
            let mut body = scope(&channel, &thread, &turn)?;
            body["operation"] = json!("renew");
            body["expected_version"] = json!(expected_version);
            body["request_id"] = json!(mutation_request_id(request_id)?);
            body["lease_seconds"] = json!(lease_seconds);
            body
        }
        RoomCoordinationCmd::Contribute {
            channel,
            thread,
            turn,
            expected_version,
            request_id,
            fingerprint,
        } => {
            let mut body = scope(&channel, &thread, &turn)?;
            body["operation"] = json!("contribute");
            body["expected_version"] = json!(expected_version);
            body["request_id"] = json!(mutation_request_id(request_id)?);
            body["fingerprint"] = json!(fingerprint);
            body
        }
        RoomCoordinationCmd::Finalize {
            channel,
            thread,
            turn,
            expected_version,
            request_id,
            final_event,
        } => {
            let mut body = scope(&channel, &thread, &turn)?;
            body["operation"] = json!("finalize");
            body["expected_version"] = json!(expected_version);
            body["request_id"] = json!(mutation_request_id(request_id)?);
            body["final_event_id"] = json!(final_event);
            body
        }
        RoomCoordinationCmd::Read {
            channel,
            thread,
            turn,
        } => {
            let mut body = scope(&channel, &thread, &turn)?;
            body["operation"] = json!("read");
            body
        }
    };
    send(client, body).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_uses_wire_field_names() {
        let channel = Uuid::new_v4();
        assert_eq!(
            scope(&channel.to_string(), "thread", "turn").expect("valid scope"),
            json!({"channel_id":channel,"thread_id":"thread","turn_id":"turn"})
        );
    }
}
