//! Desktop console over the running Orange window.
//!
//! Ctrl+Alt+Shift+O (Ctrl+Option+Shift+O on macOS) toggles it. While closed,
//! layout, drawing, and input are forwarded to the existing UI unchanged.
//! Commands use the live `ctx.air` identity, not a second database connection.

use std::any::Any;
use std::collections::VecDeque;
use std::path::Path;

use chk::air::typing::{TypingAction, TypingState};
use chk::blocking::{Block, BlockAction};
use chk::messages::{ChatRoom, ChatRoomAction, MessageContent, RoomReadState, StickerPack, VideoClip};
use chk::profiles::{Profile, ProfileAction};
use chk::Theme;
use pelican_ui::components::avatar::AvatarContent;
use pelican_ui::components::StickerPackAction;
use ramp::prism::air::{Id, Instance, Name};
use ramp::prism::drawable::shape::Variant;
use ramp::prism::drawable::text::{Align, Font};
use ramp::prism::drawable::{Color, Drawable, Instruction, LayoutTree, Offset, Rect, Shape, Text as Label};
use ramp::prism::events::{DynEvent, Key, KeyboardEvent, KeyboardState, ListenCallback, ListenFuture, Notification, WindowEvent};
use ramp::prism::Context;

use crate::cli::media::{self, Attachment};
use crate::cli::present;
use crate::contact_links;

const LINES: usize = 14;

pub struct Console {
    inner: Box<dyn Drawable>,
    font: Font,
    open: bool,
    input: String,
    lines: VecDeque<String>,
    history: Vec<String>,
    cursor: usize,
}

impl Console {
    pub(crate) fn new(inner: impl Drawable, theme: &Theme) -> Self {
        let font = theme.fonts().get_font("FontStyle::Text").cloned()
            .expect("Orange's theme includes its text font");
        Self::from_font(inner, font)
    }

    pub(crate) fn from_font(inner: impl Drawable, font: Font) -> Self {
        Self {
            inner: Box::new(inner), font, open: false, input: String::new(),
            lines: VecDeque::from(["Orange console. Esc or Ctrl+Alt+Shift+O closes it. Type help.".into()]),
            history: Vec::new(), cursor: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {self.open}
}

impl Clone for Console {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(), font: self.font.clone(), open: self.open,
            input: self.input.clone(), lines: self.lines.clone(), history: self.history.clone(),
            cursor: self.cursor,
        }
    }
}

impl std::fmt::Debug for Console {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Console").field("open", &self.open).finish_non_exhaustive()
    }
}

impl Drawable for Console {
    fn request_width(&mut self) -> (f32, f32) {self.inner.request_width()}
    fn request_height(&mut self, width: f32) -> (f32, f32) {self.inner.request_height(width)}
    fn layout(&mut self, size: (f32, f32)) -> LayoutTree {self.inner.layout(size)}

    fn draw(&self, layout: &LayoutTree, offset: Offset, bound: Rect) -> Vec<Instruction> {
        let mut instructions = self.inner.draw(layout, offset, bound);
        if self.open {instructions.extend(self.overlay(layout.0, offset, bound));}
        instructions
    }

    fn event(&mut self, ctx: &mut Context, layout: &LayoutTree, event: Box<dyn DynEvent>) {
        if let Some(key) = event.downcast_ref::<KeyboardEvent>() {
            if is_chord(key) {
                if key.state == KeyboardState::Pressed {self.open = !self.open;}
                return;
            }
            if self.open {
                if matches!(key.state, KeyboardState::Pressed | KeyboardState::Repeated) {self.on_key(ctx, key);}
                return;
            }
        } else if self.open && !reaches_app(event.as_ref()) {
            return;
        }
        self.inner.event(ctx, layout, event);
    }

    fn changed(&mut self) -> ListenFuture<'_> {
        Box::pin(async move {
            let callback = self.inner.changed().await;
            Box::new(move |target: &mut dyn Any, ctx: &mut Context| {
                let target = target.downcast_mut::<Self>().unwrap();
                callback(target.inner.as_mut().as_any_mut(), ctx);
            }) as ListenCallback
        })
    }
}

impl Console {
    fn on_key(&mut self, ctx: &mut Context, key: &KeyboardEvent) {
        match &key.key {
            Key::Escape => self.open = false,
            Key::Enter => {
                let line = std::mem::take(&mut self.input);
                let line = line.trim().to_string();
                if line.is_empty() {return;}
                self.history.push(line.clone());
                self.cursor = self.history.len();
                self.push(format!("orange> {line}"));
                for row in run_line(ctx, &line).lines() {self.push(row.to_string());}
            }
            Key::Backspace => {self.input.pop();}
            Key::Up if !self.history.is_empty() => {
                self.cursor = self.cursor.saturating_sub(1).min(self.history.len() - 1);
                self.input = self.history[self.cursor].clone();
            }
            Key::Down => {
                if self.cursor + 1 >= self.history.len() {
                    self.cursor = self.history.len();
                    self.input.clear();
                } else {
                    self.cursor += 1;
                    self.input = self.history[self.cursor].clone();
                }
            }
            Key::Character(text) if !key.modifiers.control && !key.modifiers.alt && !key.modifiers.supermeta => {
                self.input.push_str(text);
            }
            _ => {}
        }
    }

    fn push(&mut self, line: String) {
        for row in wrap(&line, 96) {self.lines.push_back(row);}
        while self.lines.len() > 80 {self.lines.pop_front();}
    }

    fn overlay(&self, size: (f32, f32), origin: Offset, bound: Rect) -> Vec<Instruction> {
        let width = (size.0 * 0.86).clamp(280.0, 760.0);
        let height = (size.1 * 0.55).clamp(220.0, 460.0);
        let x = origin.0 + ((size.0 - width) / 2.0).max(12.0);
        let y = origin.1 + (size.1 - height - 24.0).max(12.0);
        let panel = Shape::new(Color(12, 12, 14, 232), Variant::RoundedRectangle(14.0));
        let panel_layout = LayoutTree((width, height), vec![]);
        let mut instructions = panel.draw(&panel_layout, (x, y), bound);
        let body = self.lines.iter().rev().take(LINES).collect::<Vec<_>>();
        let body = body.into_iter().rev().cloned().collect::<Vec<_>>().join("\n");
        let text = format!("{body}\norange> {}_", self.input);
        instructions.extend(paint(&self.font, &text, x + 16.0, y + 14.0, width - 32.0, bound));
        instructions
    }
}

pub(crate) fn is_chord(event: &KeyboardEvent) -> bool {
    event.modifiers.control && event.modifiers.alt && event.modifiers.shift && !event.modifiers.supermeta
        && matches!(&event.key, Key::Character(text) if matches!(text.as_str(), "o" | "O" | "ø" | "Ø"))
}

fn reaches_app(event: &dyn DynEvent) -> bool {
    event.downcast_ref::<WindowEvent>().is_some() || event.downcast_ref::<Notification>().is_some()
}

fn paint(font: &Font, text: &str, x: f32, y: f32, width: f32, bound: Rect) -> Vec<Instruction> {
    let mut label = Label::new(text, font.clone(), 15.0, Color(236, 236, 236, 255), Align::Left);
    label.width = Some(width);
    let height = label.size().1.max(1.0);
    label.draw(&LayoutTree((width, height), vec![]), (x, y), bound)
}

fn wrap(line: &str, columns: usize) -> Vec<String> {
    if line.chars().count() <= columns {return vec![line.to_string()]}
    line.chars().collect::<Vec<_>>().chunks(columns).map(|chunk| chunk.iter().collect()).collect()
}

fn run_line(ctx: &mut Context, line: &str) -> String {
    let mut args = split_args(line);
    if args.is_empty() {return String::new();}
    let json = args.iter().any(|arg| arg == "--json");
    args.retain(|arg| arg != "--json");
    let command = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[1..];
    let result = match command {
        "help" => Ok(HELP.into()),
        "whoami" | "invite" => whoami(ctx),
        "rooms" => rooms(ctx),
        "read" => read(ctx, rest),
        "send" => send(ctx, rest),
        "dm" => dm(ctx, rest),
        "send-media" => send_media(ctx, rest),
        "send-gif" => send_gif(ctx, rest),
        "gif" => gif_search(rest),
        "download" => download(ctx, rest),
        "mark-read" => mark_read(ctx, rest),
        "watch" => watch(ctx, rest),
        "contacts" => contacts(ctx),
        "contact" => contact_add(ctx, rest),
        "profile" => profile(ctx, rest),
        "group" => group(ctx, rest),
        "delete" => delete_message(ctx, rest),
        "block" => set_block(ctx, rest, true),
        "unblock" => set_block(ctx, rest, false),
        "blocks" => blocks(ctx),
        "typing" => typing(ctx, rest),
        "notify" => notify(ctx, rest),
        "stickers" => stickers(ctx),
        "sticker" => sticker_add(ctx, rest),
        "send-sticker" => send_sticker(ctx, rest),
        other => Err(format!("unknown command {other}. Type help.")),
    };
    match result {
        Ok(text) if json => serde_json::to_string(&serde_json::json!({"ok": true, "text": text})).unwrap_or(text),
        Ok(text) => text,
        Err(error) if json => serde_json::json!({"ok": false, "error": error}).to_string(),
        Err(error) => format!("error: {error}"),
    }
}

fn whoami(ctx: &mut Context) -> Result<String, String> {
    let name = ctx.air.me();
    let profile = Profile::me(ctx).pending().clone();
    Ok(format!("{}\nname {name}\ninvite {}\n{}", profile.username, contact_links::contact_url(name), profile.notes))
}

fn rooms(ctx: &mut Context) -> Result<String, String> {
    let me = ctx.air.me();
    let mut rows = Vec::new();
    for (id, room) in member_rooms(ctx) {
        let unread = unread(ctx, id, &room);
        let preview = room.messages(me).last().map(|message| present::preview(&message.body, &name_of(ctx, message.author)))
            .unwrap_or_else(|| "No messages yet.".into());
        rows.push(format!("{id}\t{}\tunread {unread}\t{preview}", room_name(ctx, &room)));
    }
    if rows.is_empty() {Ok("No chats yet.".into())} else {Ok(rows.join("\n"))}
}

fn read(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let query = args.first().ok_or("read <room>")?;
    let (id, room) = open_room(ctx, query)?;
    let since = flag(args, "--since").map(present::parse_since).transpose().map_err(|error| error.to_string())?;
    let limit = flag(args, "--limit").map(|text| text.parse::<usize>()).transpose().map_err(|_| "bad --limit")?;
    let me = ctx.air.me();
    let mut messages: Vec<_> = room.messages(me).filter(|message| since.is_none_or(|since| message.timestamp >= since)).cloned().collect();
    if let Some(limit) = limit.filter(|limit| *limit < messages.len()) {
        messages = messages.split_off(messages.len() - limit);
    }
    let mut lines = vec![format!("{} ({id})", room_name(ctx, &room))];
    if messages.is_empty() {lines.push("No messages.".into());}
    for message in messages {
        lines.push(format!("[{}] {} {}", present::stamp(message.timestamp), name_of(ctx, message.author),
            present::preview(&message.body, &name_of(ctx, message.author))));
        lines.push(format!("  id {}", message.id));
    }
    Ok(lines.join("\n"))
}

fn send(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let room = args.first().ok_or("send <room> <text>")?;
    let text = args[1..].iter().filter(|arg| !arg.starts_with("--")).cloned().collect::<Vec<_>>().join(" ");
    if text.is_empty() {return Err("send <room> <text>".into());}
    let mut instance = room_instance(ctx, room)?;
    instance.send(ChatRoomAction::SendMessage(text.clone()));
    Ok(format!("sent to {}", room_name(ctx, &instance.pending())))
}

fn dm(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let who = args.first().ok_or("dm <name-or-key> <text>")?;
    let text = args[1..].join(" ");
    if text.is_empty() {return Err("dm <name-or-key> <text>".into());}
    let person = resolve_person(ctx, who)?;
    let mut room = existing_dm(ctx, person).unwrap_or_else(|| {
        let room = ChatRoom::create(ctx, vec![person]);
        room.notify(true);
        room
    });
    room.send(ChatRoomAction::SendMessage(text));
    Ok(format!("sent to {}", room_name(ctx, &room.pending())))
}

fn send_media(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let room = args.first().ok_or("send-media <room> <file>")?;
    let file = args.get(1).ok_or("send-media <room> <file>")?;
    let mut instance = room_instance(ctx, room)?;
    let members = instance.pending().members.clone();
    match media::load_attachment(Path::new(file)).map_err(|error| error.to_string())? {
        Attachment::Photo(photo) => {instance.send(ChatRoomAction::SendPhoto(photo));}
        Attachment::Video {video, mp4} => {
            publish_clip(ctx, mp4, &members);
            instance.send(ChatRoomAction::SendVideo(video));
        }
    }
    if let Some(caption) = flag(args, "--caption") {
        instance.send(ChatRoomAction::SendMessage(caption.to_string()));
    }
    Ok("sent media".into())
}

fn send_gif(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let room = args.first().ok_or("send-gif <room> <query-or-id>")?;
    let query = args[1..].iter().filter(|arg| !arg.starts_with("--")).cloned().collect::<Vec<_>>().join(" ");
    if query.is_empty() {return Err("send-gif <room> <query-or-id>".into());}
    let index = flag(args, "--index").and_then(|text| text.parse().ok()).unwrap_or(0);
    let gif = pick_gif(&query, index, args.iter().any(|arg| arg == "--search")).map_err(|error| error.to_string())?;
    room_instance(ctx, room)?.send(ChatRoomAction::SendGif(gif));
    Ok("sent GIF".into())
}

fn gif_search(args: &[String]) -> Result<String, String> {
    if args.first().map(String::as_str) != Some("search") {return Err("gif search <query>".into());}
    let query = args[1..].join(" ");
    let gifs = media::search_gifs_now((!query.is_empty()).then_some(query.as_str()), 0).map_err(|error| error.to_string())?;
    Ok(gifs.iter().enumerate().map(|(index, gif)| format!("{index}\t{}\t{}", gif.id, gif.url)).collect::<Vec<_>>().join("\n"))
}

fn download(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let room = args.first().ok_or("download <room> <message-id>")?;
    let message_id = args.get(1).ok_or("download <room> <message-id>")?;
    let id: Id = message_id.parse().map_err(|_| "bad message id")?;
    let (_, data) = open_room(ctx, room)?;
    let message = data.messages(ctx.air.me()).find(|message| message.id == id).cloned().ok_or("message not found")?;
    let (bytes, ext) = match &message.body {
        MessageContent::Photo(photo) => (photo.jpeg().to_vec(), "jpg"),
        MessageContent::Sticker(sticker) => (sticker.bytes().to_vec(), "jpg"),
        MessageContent::Gif(gif) => (fetch_url(&gif.url)?, "gif"),
        MessageContent::Video(video) => (clip_bytes(ctx, video.clip)?, "mp4"),
        _ => return Err("that message has no file".into()),
    };
    let path = flag(args, "--out").map(Path::new).map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(&format!("orange-{}.{}", &id.to_string()[..8], ext)).to_path_buf());
    std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(format!("saved {}", path.display()))
}

fn mark_read(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let query = args.first().ok_or("mark-read <room>")?;
    let (id, room) = open_room(ctx, query)?;
    let mut reads = RoomReadState::open(ctx, id);
    let ids: Vec<Id> = reads.pending().unread(&room).collect();
    let count = ids.len();
    if !ids.is_empty() {reads.send(ids);}
    Ok(format!("marked {count}"))
}

fn watch(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let body = if let Some(room) = args.first() {
        read(ctx, std::slice::from_ref(room))?
    } else {rooms(ctx)?};
    Ok(format!("{body}\nThe overlay runs one command at a time. `orange watch` in a terminal streams newline-delimited JSON."))
}

fn contacts(ctx: &mut Context) -> Result<String, String> {
    let me = ctx.air.me();
    let mut rows: Vec<_> = ctx.air.instances::<Profile>().list().into_values().filter_map(|profile| {
        let profile = profile.pending();
        (profile.name != Some(me)).then(|| format!("{}\t{}", profile.username, profile.name.map(|name| name.to_string()).unwrap_or_default()))
    }).collect();
    rows.sort();
    if rows.is_empty() {Ok("No contacts yet.".into())} else {Ok(rows.join("\n"))}
}

fn contact_add(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    if args.first().map(String::as_str) != Some("add") {return Err("contact add <name-or-link>".into());}
    let who = args.get(1).ok_or("contact add <name-or-link>")?;
    let name = contact_links::parse_contact(who).ok_or("not an Orange name or invite link")?;
    if name == ctx.air.me() {return Err("you cannot add yourself".into());}
    let profile = Profile::from_name(ctx, name).pending().clone();
    Ok(format!("added {} ({name})", profile.username))
}

fn profile(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("set") => {
            let mut profile = Profile::me(ctx);
            if let Some(username) = flag(args, "--username") {
                if username.trim().is_empty() {return Err("username cannot be empty".into());}
                profile.send(ProfileAction::ChangeUsername(username.to_string()));
            }
            if let Some(notes) = flag(args, "--notes") {profile.send(ProfileAction::ChangeNotes(notes.to_string()));}
            if let Some(path) = flag(args, "--avatar") {
                let image = media::load_image(Path::new(path)).map_err(|error| error.to_string())?;
                profile.send(ProfileAction::ChangeAvatar(AvatarContent::image(image)));
            }
            if flag(args, "--username").is_none() && flag(args, "--notes").is_none() && flag(args, "--avatar").is_none() {
                return Err("profile set needs --username, --notes, or --avatar".into());
            }
            whoami(ctx)
        }
        _ => whoami(ctx),
    }
}

fn group(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let action = args.first().map(String::as_str).unwrap_or("");
    match action {
        "create" => {
            let name = args.get(1).ok_or("group create <name> --member <who> --member <who>")?;
            let mut people = members(ctx, args)?;
            people.retain(|name| *name != ctx.air.me());
            people.sort_unstable();
            people.dedup();
            if people.len() < 2 {return Err("a group needs two --member values".into());}
            let mut room = ChatRoom::create(ctx, people);
            room.send(ChatRoomAction::RenameGroup(name.clone()));
            room.notify(true);
            Ok(format!("created {} ({})", room.pending().group_details().name, room.id()))
        }
        "add" => {
            let room = args.get(1).ok_or("group add <room> --member <who>")?;
            let people = members(ctx, args)?;
            let mut instance = room_instance(ctx, room)?;
            ChatRoom::add_members(ctx, &mut instance, people);
            Ok("added".into())
        }
        "leave" | "remove" => {
            let room = args.get(1).ok_or("group leave <room>")?;
            let mut instance = room_instance(ctx, room)?;
            instance.send(ChatRoomAction::LeaveGroup);
            instance.notify(false);
            Ok("left the group".into())
        }
        "rename" => {
            let room = args.get(1).ok_or("group rename <room> <name>")?;
            let name = args[2..].iter().filter(|arg| !arg.starts_with("--")).cloned().collect::<Vec<_>>().join(" ");
            if name.trim().is_empty() {return Err("group rename <room> <name>".into());}
            room_instance(ctx, room)?.send(ChatRoomAction::RenameGroup(name.clone()));
            Ok(format!("renamed to {name}"))
        }
        "photo" => {
            let room = args.get(1).ok_or("group photo <room> <file>")?;
            let file = args.get(2).ok_or("group photo <room> <file>")?;
            let image = media::load_image(Path::new(file)).map_err(|error| error.to_string())?;
            room_instance(ctx, room)?.send(ChatRoomAction::SetGroupPhoto(AvatarContent::image(image)));
            Ok("updated the group photo".into())
        }
        _ => Err("group create|add|leave|remove|rename|photo".into()),
    }
}

fn delete_message(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let room = args.first().ok_or("delete <room> <message-id>")?;
    let id: Id = args.get(1).ok_or("delete <room> <message-id>")?.parse().map_err(|_| "bad message id")?;
    let (_, data) = open_room(ctx, room)?;
    let message = data.messages(ctx.air.me()).find(|message| message.id == id).cloned().ok_or("message not found")?;
    if matches!(message.body, MessageContent::System(_)) {return Err("system messages cannot be deleted".into());}
    room_instance(ctx, room)?.send(ChatRoomAction::DeleteMessage(message));
    Ok(format!("deleted {id}"))
}

fn set_block(ctx: &mut Context, args: &[String], block: bool) -> Result<String, String> {
    let who = args.first().ok_or(if block {"block <name-or-key>"} else {"unblock <name-or-key>"})?;
    let name = resolve_person(ctx, who)?;
    if name == ctx.air.me() {return Err("you cannot block yourself".into());}
    let mut record = Block::create(ctx, name);
    record.send(if block {BlockAction::Block} else {BlockAction::Unblock});
    Ok(format!("{} {name}", if block {"blocked"} else {"unblocked"}))
}

fn blocks(ctx: &mut Context) -> Result<String, String> {
    let me = ctx.air.me();
    let rows: Vec<_> = ctx.air.instances::<Block>().list().into_values().filter_map(|block| {
        let block = block.pending();
        (block.blocker == me && block.is_blocked).then(|| format!("{} {}", name_of(ctx, block.blocked), block.blocked))
    }).collect();
    if rows.is_empty() {Ok("No blocked contacts.".into())} else {Ok(rows.join("\n"))}
}

fn typing(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let room = args.first().ok_or("typing <room>")?;
    let (id, data) = open_room(ctx, room)?;
    let active = !args.iter().any(|arg| arg == "--stop");
    let mut state = typing_state(ctx, id);
    for member in data.members {
        if member == ctx.air.me() || Block::did_i_block(ctx, member) || Block::am_i_blocked(ctx, member) {continue;}
        if !state.pending().shared_with.contains(&member) {
            state.share(member);
            state.send(TypingAction::Share(member));
        }
    }
    state.send(TypingAction::Activity {active, at: TypingState::now()});
    Ok(format!("typing {}", if active {"on"} else {"off"}))
}

fn notify(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let room = args.first().ok_or("notify <room>")?;
    let (id, _) = open_room(ctx, room)?;
    Ok(format!("{id}\nPush watches stay with this app and orange-listener. This command does not acknowledge them."))
}

fn stickers(ctx: &mut Context) -> Result<String, String> {
    let pack = my_pack(ctx);
    let rows: Vec<_> = pack.pending().stickers.iter().enumerate()
        .map(|(index, sticker)| format!("{index}\t{} bytes", sticker.bytes().len())).collect();
    if rows.is_empty() {Ok("No stickers yet.".into())} else {Ok(rows.join("\n"))}
}

fn sticker_add(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    if args.first().map(String::as_str) != Some("add") {return Err("sticker add <image>".into());}
    let file = args.get(1).ok_or("sticker add <image>")?;
    let sticker = media::load_sticker(Path::new(file)).map_err(|error| error.to_string())?;
    my_pack(ctx).send(StickerPackAction::Add(sticker));
    Ok("added sticker".into())
}

fn send_sticker(ctx: &mut Context, args: &[String]) -> Result<String, String> {
    let room = args.first().ok_or("send-sticker <room> <index>")?;
    let index: usize = args.get(1).ok_or("send-sticker <room> <index>")?.parse().map_err(|_| "bad index")?;
    let sticker = my_pack(ctx).pending().stickers.get(index).cloned().ok_or("no sticker at that index")?;
    room_instance(ctx, room)?.send(ChatRoomAction::SendSticker(sticker));
    Ok("sent sticker".into())
}

fn member_rooms(ctx: &mut Context) -> Vec<(Id, ChatRoom)> {
    let me = ctx.air.me();
    let mut rooms: Vec<_> = ctx.air.instances::<ChatRoom>().list().into_iter().filter_map(|(_id, inst)| {
        let id = inst.id();
        let room = inst.pending().clone();
        room.members.contains(&me).then_some((id, room))
    }).collect();
    rooms.sort_by_key(|(_, room)| std::cmp::Reverse(room.messages(me).last().map(|message| message.timestamp).unwrap_or(0)));
    rooms
}

fn open_room(ctx: &mut Context, query: &str) -> Result<(Id, ChatRoom), String> {
    let instance = room_instance(ctx, query)?;
    Ok((instance.id(), instance.pending().clone()))
}

fn room_instance(ctx: &mut Context, query: &str) -> Result<Instance<ChatRoom>, String> {
    let me = ctx.air.me();
    let mut found = Vec::new();
    for inst in ctx.air.instances::<ChatRoom>().list().into_values() {
        let room = inst.pending().clone();
        if !room.members.contains(&me) {continue;}
        let label = room_name(ctx, &room);
        if query == inst.id().to_string() || query == room.id.to_string() || label.eq_ignore_ascii_case(query) {
            found.push(inst);
        }
    }
    if found.len() == 1 {return Ok(found.remove(0));}
    if let Ok(person) = resolve_person(ctx, query) {
        if let Some(room) = existing_dm(ctx, person) {return Ok(room);}
    }
    if found.is_empty() {Err(format!("no chat matches {query}"))} else {Err(format!("more than one chat matches {query}"))}
}

fn existing_dm(ctx: &mut Context, person: Name) -> Option<Instance<ChatRoom>> {
    let me = ctx.air.me();
    let mut found = Vec::new();
    for inst in ctx.air.instances::<ChatRoom>().list().into_values() {
        let room = inst.pending().clone();
        if room.is_group() || !room.members.contains(&me) || !room.members.contains(&person) {continue;}
        if room.members.iter().any(|name| *name != me && *name != person) {continue;}
        found.push((room.messages(me).next().is_none(), inst.id(), inst));
    }
    found.sort_by_key(|(empty, id, _)| (*empty, *id));
    found.into_iter().next().map(|(_, _, inst)| inst)
}

fn resolve_person(ctx: &mut Context, input: &str) -> Result<Name, String> {
    if let Some(name) = contact_links::parse_contact(input) {return Ok(name);}
    let mut hits = Vec::new();
    for profile in ctx.air.instances::<Profile>().list().into_values() {
        let profile = profile.pending();
        if profile.username.eq_ignore_ascii_case(input.trim()) {
            if let Some(name) = profile.name {hits.push(name);}
        }
    }
    match hits.as_slice() {
        [name] => Ok(*name),
        [] => Err(format!("{input} is not an Orange name, invite link, or known username")),
        _ => Err(format!("more than one contact is named {input}")),
    }
}

fn room_name(ctx: &mut Context, room: &ChatRoom) -> String {
    if room.is_group() {return room.group_details().name;}
    let me = ctx.air.me();
    let member = if room.members.len() == 1 {room.members[0]} else {
        room.members.iter().copied().find(|name| *name != me).unwrap_or(me)
    };
    let username = name_of(ctx, member);
    if member == me {format!("{username} (You)")} else {username}
}

fn name_of(ctx: &mut Context, name: Name) -> String {
    Profile::from_name(ctx, name).pending().username.clone()
}

fn unread(ctx: &mut Context, room: Id, data: &ChatRoom) -> usize {
    let me = ctx.air.me();
    ctx.air.instances::<RoomReadState>().list().into_values().find_map(|state| {
        let state = state.pending();
        (state.room == room && state.owner == me).then(|| state.unread(data).count())
    }).unwrap_or_else(|| data.messages(me).filter(|message| message.author != me).count())
}

fn members(ctx: &mut Context, args: &[String]) -> Result<Vec<Name>, String> {
    let mut people = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--member" {
            let who = args.next().ok_or("--member needs a name")?;
            people.push(resolve_person(ctx, who)?);
        }
    }
    if people.is_empty() {return Err("pass --member".into());}
    Ok(people)
}

fn publish_clip(ctx: &mut Context, bytes: std::sync::Arc<[u8]>, members: &[Name]) {
    let init = <VideoClip as ramp::prism::air::Contract>::Init::new(bytes);
    let clip = ctx.air.instances::<VideoClip>().create(init);
    let me = ctx.air.me();
    for member in members.iter().filter(|name| **name != me) {clip.share(*member);}
}

fn clip_bytes(ctx: &mut Context, id: Id) -> Result<Vec<u8>, String> {
    let clip = ctx.air.instances::<VideoClip>().list().into_values().find(|clip| clip.pending().id == id)
        .ok_or("video file has not arrived yet")?;
    media::clip_bytes(&clip.pending()).map_err(|error| error.to_string())
}

fn my_pack(ctx: &mut Context) -> Instance<StickerPack> {
    let me = ctx.air.me();
    ctx.air.instances::<StickerPack>().list().into_values().find(|pack| pack.pending().owner == me)
        .unwrap_or_else(|| ctx.air.instances::<StickerPack>().create(me))
}

fn typing_state(ctx: &mut Context, room: Id) -> Instance<TypingState> {
    let me = ctx.air.me();
    ctx.air.instances::<TypingState>().list().into_values().find(|state| {
        let state = state.pending();
        state.room == room && state.author == me
    }).unwrap_or_else(|| {
        let state = ctx.air.instances::<TypingState>().create(room);
        state.notify(false);
        state
    })
}

fn fetch_url(url: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(20)).build();
    let mut bytes = Vec::new();
    agent.get(url).call().map_err(|error| error.to_string())?
        .into_reader().read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn pick_gif(query: &str, index: usize, force_search: bool) -> Result<chk::messages::Gif, crate::cli::CliError> {
    let token = query.trim();
    if !force_search && !token.contains(char::is_whitespace) && token.bytes().all(|byte| byte.is_ascii_alphanumeric()) && token.len() >= 5 {
        if let Ok(gif) = media::gif_by_id(token) {return Ok(gif);}
    }
    let gifs = media::search_gifs_now(Some(token), 0)?;
    gifs.into_iter().nth(index).ok_or_else(|| crate::cli::CliError::fail(format!("GIF index {index} is past the search results")))
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2).find(|pair| pair[0] == name).map(|pair| pair[1].as_str())
}

fn split_args(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = false;
    for character in line.chars() {
        match character {
            '"' => quote = !quote,
            character if character.is_whitespace() && !quote => {
                if !current.is_empty() {args.push(std::mem::take(&mut current));}
            }
            character => current.push(character),
        }
    }
    if !current.is_empty() {args.push(current);}
    args
}

const HELP: &str = "\
whoami, rooms, read <room>, send <room> <text>, dm <name> <text>
send-media <room> <file> [--caption text], send-gif <room> <query>
gif search <query>, download <room> <message-id>, mark-read <room>
watch [room], contacts, contact add <name>, profile show, profile set
group create|add|leave|remove|rename|photo, delete <room> <message-id>
block, unblock, blocks, typing <room> [--stop], notify <room>
stickers, sticker add <image>, send-sticker <room> <index>
Add --json for a single JSON object. Esc closes this console.";

#[cfg(test)]
mod tests {
    use super::*;
    use ramp::prism::drawable::Drawable;
    use ramp::prism::events::Modifiers;

    #[derive(Clone, Debug)]
    struct Fixed;
    impl Drawable for Fixed {
        fn request_width(&mut self) -> (f32, f32) {(12.0, 48.0)}
        fn request_height(&mut self, _width: f32) -> (f32, f32) {(30.0, 30.0)}
        fn draw(&self, _layout: &LayoutTree, _offset: Offset, _bound: Rect) -> Vec<Instruction> {Vec::new()}
    }

    fn key(text: &str, control: bool, alt: bool, shift: bool, state: KeyboardState) -> KeyboardEvent {
        KeyboardEvent {
            key: Key::Character(text.into()),
            state,
            modifiers: Modifiers {shift, control, alt, supermeta: false},
        }
    }

    #[test]
    fn chord_is_ctrl_alt_shift_o_including_the_macos_option_glyph() {
        assert!(is_chord(&key("o", true, true, true, KeyboardState::Pressed)));
        assert!(is_chord(&key("O", true, true, true, KeyboardState::Pressed)));
        assert!(is_chord(&key("ø", true, true, true, KeyboardState::Pressed)));
        assert!(is_chord(&key("Ø", true, true, true, KeyboardState::Pressed)));
        assert!(!is_chord(&key("o", true, false, true, KeyboardState::Pressed)));
        assert!(!is_chord(&key("o", true, true, false, KeyboardState::Pressed)));
        assert!(!is_chord(&key("p", true, true, true, KeyboardState::Pressed)));
    }

    #[test]
    fn closed_console_keeps_the_app_layout_and_drawing() {
        let font = Font::from_bytes(include_bytes!("../vendor/ramp2/resources/font.ttf")).unwrap();
        let mut app = Fixed;
        let mut console = Console::from_font(Fixed, font);
        assert!(!console.is_open());
        assert_eq!(console.request_width(), app.request_width());
        assert_eq!(console.request_height(48.0), app.request_height(48.0));
        let app_layout = app.layout((48.0, 30.0));
        let console_layout = console.layout((48.0, 30.0));
        assert_eq!(app_layout.0, console_layout.0);
        assert_eq!(app_layout.1.len(), console_layout.1.len());
        let bound = (0.0, 0.0, 48.0, 30.0);
        assert_eq!(console.draw(&console_layout, (0.0, 0.0), bound).len(), app.draw(&app_layout, (0.0, 0.0), bound).len());
    }

    #[test]
    fn quoted_words_stay_one_argument() {
        assert_eq!(split_args(r#"send room "hello there""#), vec!["send", "room", "hello there"]);
    }
}
