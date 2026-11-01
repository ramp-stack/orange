//! Photos, videos, stickers, and the same Giphy search CHK's picker uses.

use std::path::Path;
use std::time::Duration;

use chk::messages::{Gif, Photo, Video};
use pelican_ui::components::{Gifs, Sticker};

use super::{CliError, Result};

pub enum Attachment {
    Photo(Photo),
    Video {
        video: Video,
        mp4: std::sync::Arc<[u8]>,
    },
}

pub fn giphy_key() -> Option<String> {
    if let Ok(key) = std::env::var("GIPHY_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() && key.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Some(key);
        }
    }
    if let Some(key) = dotenv_key() {
        return Some(key);
    }
    option_env!("GIPHY_API_KEY")
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

fn dotenv_key() -> Option<String> {
    let contents = std::fs::read_to_string(".env").ok()?;
    contents.lines().find_map(|line| {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let (name, value) = line.split_once('=')?;
        if name.trim() != "GIPHY_API_KEY" {
            return None;
        }
        let value = value.split('#').next().unwrap().trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(value);
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

pub async fn search_gifs(query: Option<String>, offset: usize) -> Result<Vec<Gif>> {
    let gifs = Gifs::default();
    let Some(key) = giphy_key() else {
        return Err(CliError::fail(
            "GIPHY_API_KEY is not set. Put it in .env or the environment and rebuild, the same way the Orange app does.",
        ));
    };
    gifs.set_api_key(&key);
    let mut task = gifs.search(query, offset);
    let batch = match task.next().await {
        Some(Ok(batch)) => batch,
        Some(Err(error)) => return Err(CliError::fail(format!("GIF search failed: {error:?}"))),
        None => return Err(CliError::fail("GIF search failed")),
    };
    if batch.gifs.is_empty() {
        return Err(CliError::fail("GIF search returned nothing"));
    }
    Ok(batch.gifs)
}

/// Blocking search with the same Giphy query the picker uses. The in-app
/// console cannot await the picker's background task.
pub fn search_gifs_now(query: Option<&str>, offset: usize) -> Result<Vec<Gif>> {
    let key = giphy_key().ok_or_else(|| {
        CliError::fail("GIPHY_API_KEY is not set. Put it in .env or the environment, the same way the Orange app does.")
    })?;
    let endpoint = if query.is_some_and(|query| !query.is_empty()) {"search"} else {"trending"};
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    let mut request = agent.get(&format!("https://api.giphy.com/v1/gifs/{endpoint}"))
        .query("api_key", &key).query("limit", "24")
        .query("offset", &offset.to_string()).query("rating", "g");
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        request = request.query("q", query);
    }
    let body: serde_json::Value = request.call()
        .map_err(|error| CliError::fail(format!("GIF search failed: {error}")))?
        .into_json()
        .map_err(|error| CliError::fail(format!("GIF search failed: {error}")))?;
    let gifs = body.get("data").and_then(|value| value.as_array())
        .map(|rows| rows.iter().filter_map(parse_gif).collect::<Vec<_>>())
        .unwrap_or_default();
    if gifs.is_empty() {return Err(CliError::fail("GIF search returned nothing"));}
    Ok(gifs)
}

pub fn gif_by_id(id: &str) -> Result<Gif> {
    let key = giphy_key().ok_or_else(|| {
        CliError::fail("GIPHY_API_KEY is not set. Put it in .env or the environment, the same way the Orange app does.")
    })?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    let body: serde_json::Value = agent
        .get(&format!("https://api.giphy.com/v1/gifs/{id}"))
        .query("api_key", &key)
        .call()
        .map_err(|error| CliError::fail(format!("GIF lookup failed: {error}")))?
        .into_json()
        .map_err(|error| CliError::fail(format!("GIF lookup failed: {error}")))?;
    parse_gif(body.get("data").unwrap_or(&body))
        .ok_or_else(|| CliError::fail(format!("no GIF with id {id}")))
}

/// Same fields CHK's picker keeps from a Giphy `fixed_width` rendition.
fn parse_gif(value: &serde_json::Value) -> Option<Gif> {
    let text = |key: &str| value.get(key)?.as_str().map(str::to_owned);
    let number = |parent: &serde_json::Value, key: &str| parent.get(key)?.as_str()?.parse().ok();
    let images = value.get("images")?;
    let full = images.get("fixed_width")?;
    let small = images.get("fixed_width_small").unwrap_or(full);
    Some(Gif {
        id: text("id")?,
        url: text_at(full, "url")?,
        width: number(full, "width")?,
        height: number(full, "height")?,
        preview: text_at(small, "url")?,
    })
}

fn text_at(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

pub fn load_attachment(path: &Path) -> Result<Attachment> {
    let file = std::fs::File::open(path)
        .map_err(|error| CliError::fail(format!("{}: {error}", path.display())))?;
    if let Ok(image) = media::Image::read(file)
        && let Some(photo) = Photo::from_image(&image)
    {
        return Ok(Attachment::Photo(photo));
    }
    let file = std::fs::File::open(path)
        .map_err(|error| CliError::fail(format!("{}: {error}", path.display())))?;
    let movie = media::Media::read(file).map_err(|_| CliError::fail(format!(
        "{} is not a photo or video Orange can send. Chats carry photos, videos, Giphy GIFs, and stickers.",
        path.display(),
    )))?;
    if movie.video.dimensions().is_none() {
        return Err(CliError::fail(format!(
            "{} has no video track",
            path.display()
        )));
    }
    let mut cursor = movie.video.cursor();
    let frame = cursor
        .video()
        .ok_or_else(|| CliError::fail("could not read a video frame"))?;
    let poster = Photo::encode(&frame.image, 640, 75)
        .ok_or_else(|| CliError::fail("could not build a video poster"))?;
    let format = movie
        .video
        .format()
        .filter(|format| matches!(format, media::Format::Mp4_Mpeg4p2_Aac { .. }))
        .unwrap_or(media::Format::Mp4_Mpeg4p2_Aac { bitrate: 8_000_000 });
    let mut output = Vec::new();
    for chunk in movie.encode(format).map_err(CliError::fail)? {
        output.extend(chunk.map_err(CliError::fail)?);
    }
    let mp4: std::sync::Arc<[u8]> = output.into();
    // `Mp4` is reachable through VideoClip's public contract init type.
    let init = <chk::messages::VideoClip as air::Contract>::Init::new(mp4.clone());
    Ok(Attachment::Video {
        video: Video {
            clip: init.id(),
            poster,
            duration_ms: movie.duration().as_millis() as u64,
        },
        mp4,
    })
}

pub fn load_image(path: &Path) -> Result<media::Image> {
    let file = std::fs::File::open(path)
        .map_err(|error| CliError::fail(format!("{}: {error}", path.display())))?;
    media::Image::read(file).map_err(|error| CliError::fail(format!("{}: {error}", path.display())))
}

pub fn load_sticker(path: &Path) -> Result<Sticker> {
    let image = load_image(path)?;
    Sticker::from_photo(&image)
        .ok_or_else(|| CliError::fail("could not build a sticker from that image"))
}

pub fn clip_bytes(clip: &chk::messages::VideoClip) -> Result<Vec<u8>> {
    let value = serde_json::to_value(clip).map_err(|error| CliError::fail(error.to_string()))?;
    let text = value
        .get("mp4")
        .and_then(|value| value.as_str())
        .ok_or_else(|| CliError::fail("video clip has no media payload"))?;
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, text)
        .map_err(|error| CliError::fail(error.to_string()))
}
