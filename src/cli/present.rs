//! JSON and text shapes for the CLI. `watch` lines are one object, no pretty print.

use air::{Id, Name};
use chk::messages::{Message, MessageContent, SystemMessage};
use serde_json::{Value, json};

use super::CliError;

pub fn watch_event(room: Id, message: &Message, username: &str) -> Value {
    json!({
        "event": "message",
        "room": room.to_string(),
        "sender": message.author.to_string(),
        "sender_username": username,
        "timestamp": message.timestamp,
        "id": message.id.to_string(),
        "content": content_json(&message.body, username),
    })
}

pub fn write_watch_line(value: &Value) {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value).expect("watch event is json");
    use std::io::Write;
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

pub fn content_json(body: &MessageContent, username: &str) -> Value {
    match body {
        MessageContent::Text(text) => json!({"type": "text", "text": text}),
        MessageContent::Gif(gif) => json!({
            "type": "gif",
            "id": gif.id,
            "url": gif.url,
            "preview": gif.preview,
            "width": gif.width,
            "height": gif.height,
        }),
        MessageContent::Photo(photo) => {
            let (width, height) = photo.size();
            json!({"type": "photo", "width": width, "height": height, "bytes": photo.jpeg().len()})
        }
        MessageContent::Video(video) => {
            let (width, height) = video.poster.size();
            json!({
                "type": "video",
                "clip": video.clip.to_string(),
                "duration_ms": video.duration_ms,
                "width": width,
                "height": height,
                "poster_bytes": video.poster.jpeg().len(),
            })
        }
        MessageContent::Sticker(sticker) => json!({
            "type": "sticker",
            "bytes": sticker.bytes().len(),
            "width": sticker.image().dimensions.0,
            "height": sticker.image().dimensions.1,
        }),
        MessageContent::System(notice) => json!({
            "type": "system",
            "text": system_text(notice, username),
            "member": notice.member().to_string(),
        }),
    }
}

pub fn system_text(notice: &SystemMessage, username: &str) -> String {
    notice.text(username)
}

pub fn preview(body: &MessageContent, username: &str) -> String {
    match body {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Gif(_) => "GIF".into(),
        MessageContent::Photo(_) => "Photo".into(),
        MessageContent::Video(_) => "Video".into(),
        MessageContent::Sticker(_) => "Sticker".into(),
        MessageContent::System(notice) => system_text(notice, username),
    }
}

pub fn short_name(name: &Name) -> String {
    let text = name.to_string();
    if text.len() < 22 {
        return text;
    }
    format!("{}...{}", &text[..17], &text[text.len() - 4..])
}

pub fn stamp(nanos: u64) -> String {
    let secs = (nanos / 1_000_000_000) as i64;
    let nsec = (nanos % 1_000_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, nsec)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| nanos.to_string())
}

pub fn parse_since(text: &str) -> Result<u64, CliError> {
    let text = text.trim();
    if let Ok(number) = text.parse::<u64>() {
        return Ok(if number < 1_000_000_000_000 {
            number.saturating_mul(1_000_000_000)
        } else if number < 1_000_000_000_000_000 {
            number.saturating_mul(1_000_000)
        } else {
            number
        });
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(text)
        .map_err(|_| CliError::usage("--since expects unix seconds or an RFC3339 time"))?;
    Ok(parsed.timestamp_nanos_opt().unwrap_or(0) as u64)
}

pub fn print_value(json: bool, value: &Value) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        );
    } else {
        println!("{}", human(value));
    }
}

fn human(value: &Value) -> String {
    if let Some(rooms) = value.get("rooms").and_then(|rooms| rooms.as_array()) {
        if rooms.is_empty() {
            return "No chats yet.".into();
        }
        return rooms
            .iter()
            .map(|room| {
                format!(
                    "{}\t{}\tunread {}\t{}",
                    room["id"].as_str().unwrap_or(""),
                    room["name"].as_str().unwrap_or(""),
                    room["unread"].as_u64().unwrap_or(0),
                    room["preview"].as_str().unwrap_or(""),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(messages) = value
        .get("messages")
        .and_then(|messages| messages.as_array())
    {
        let mut lines = vec![format!(
            "{} ({})",
            value["name"].as_str().unwrap_or("chat"),
            value["id"].as_str().unwrap_or(""),
        )];
        if messages.is_empty() {
            lines.push("No messages.".into())
        }
        for message in messages {
            lines.push(format!(
                "[{}] {} {}: {}",
                message["time"].as_str().unwrap_or(""),
                message["sender_username"].as_str().unwrap_or(""),
                message["id"].as_str().unwrap_or(""),
                message["preview"].as_str().unwrap_or(""),
            ));
            if let Some(content) = message.get("content") {
                if content["type"] != "text" {
                    lines.push(format!("  {content}"));
                }
            }
        }
        return lines.join("\n");
    }
    if let Some(contacts) = value.get("contacts").and_then(|items| items.as_array()) {
        if contacts.is_empty() {
            return "No contacts yet.".into();
        }
        return contacts
            .iter()
            .map(|contact| {
                format!(
                    "{}\t{}",
                    contact["username"].as_str().unwrap_or(""),
                    contact["name"].as_str().unwrap_or(""),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(name) = value.get("name").and_then(|name| name.as_str())
        && value.get("invite").is_some()
        && value.get("messages").is_none()
    {
        return format!(
            "{}\nname {}\ninvite {}\ndata {}",
            value["username"].as_str().unwrap_or(""),
            name,
            value["invite"].as_str().unwrap_or(""),
            value["data_dir"].as_str().unwrap_or(""),
        );
    }
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chk::messages::Message;

    fn name() -> Name {
        "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            .parse()
            .unwrap()
    }

    #[test]
    fn watch_line_is_one_object_with_the_bot_fields() {
        let message = Message {
            body: MessageContent::Text("ping".into()),
            author: name(),
            timestamp: 1_700_000_000_000_000_000,
            hidden_from: vec![],
            id: Id::from(9),
        };
        let event = watch_event(Id::from(3), &message, "Ada");
        for key in ["event", "room", "sender", "timestamp", "content"] {
            assert!(event.get(key).is_some(), "{key}");
        }
        assert_eq!(event["event"], "message");
        assert_eq!(event["sender"], name().to_string());
        assert_eq!(event["content"]["type"], "text");
        assert_eq!(event["content"]["text"], "ping");
        assert_eq!(event["timestamp"], 1_700_000_000_000_000_000u64);
        let line = serde_json::to_string(&event).unwrap();
        assert!(!line.contains('\n'));
        assert!(line.ends_with('}'));
    }

    #[test]
    fn since_accepts_unix_seconds() {
        assert_eq!(
            parse_since("1700000000").unwrap(),
            1_700_000_000_000_000_000
        );
    }
}
