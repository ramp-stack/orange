//! AIR session used by the CLI.
//!
//! Readonly sessions (`orange watch`) never write the cache and never send a
//! storage create. They also never emit `Notification::Received`, which is the
//! acknowledgement `orange-listener.service` consumes before it wakes a bot.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use std::io::Read;

use air::{Contract, Id, Location, Name, Resolver};
use chk::air::typing::{TypingAction, TypingState};
use chk::blocking::{Block, BlockAction};
use chk::messages::{
    ChatRoom, ChatRoomAction, Message, MessageContent, RoomReadState, StickerPack, VideoClip,
};
use chk::profiles::{Profile, ProfileAction, Username};
use pelican_ui::components::StickerPackAction;
use pelican_ui::components::avatar::AvatarContent;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::media::{self, Attachment};
use super::present::{self, watch_event};
use super::store::{self, Store};
use super::{CliError, Options, Result};

struct Tracked<C: Contract> {
    instance: air::Instance<C>,
    pending_outs: usize,
    notify: bool,
    started: bool,
}

enum Record {
    Room(Tracked<ChatRoom>),
    Profile(Tracked<Profile>),
    Reads(Tracked<RoomReadState>),
    Block(Tracked<Block>),
    Typing(Tracked<TypingState>),
    Clip(Tracked<VideoClip>),
    Pack(Tracked<StickerPack>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PipeId {
    Catalog(air::RequestType),
    Instance(Location),
}

struct LivePipe {
    epoch: u64,
    task: tokio::task::JoinHandle<()>,
}

struct Incoming {
    epoch: u64,
    id: PipeId,
    body: IncomingBody,
}

enum IncomingBody {
    Response(air::Response),
    Closed,
}

pub struct Session {
    store: Store,
    dir: std::path::PathBuf,
    secret: air::Secret,
    air: air::Air,
    records: HashMap<Location, Record>,
    resolver: air::DefaultResolver,
    client: Option<air_server::Client>,
    events: mpsc::UnboundedReceiver<Incoming>,
    event_tx: mpsc::UnboundedSender<Incoming>,
    pipes: HashMap<PipeId, LivePipe>,
    next_epoch: u64,
    readonly: bool,
    offline: bool,
    timeout: Duration,
    verbose: bool,
}

impl Session {
    pub async fn open(options: &Options) -> Result<Self> {
        let dir = store::data_dir(options.data_dir.as_deref())?;
        let store = if options.readonly {
            Store::open_readonly(&dir)?
        } else {
            Store::open_writable(&dir)?
        };
        let secret = if options.readonly {
            store.load_identity()?.ok_or_else(|| {
                CliError::fail(format!("no Orange identity in {}", dir.display(),))
            })?
        } else {
            store.load_or_create_identity()?
        };
        let existed = store.load_air(&secret)?.is_some();
        let air = store.load_air(&secret)?.unwrap_or_else(|| {
            air::Air::new(secret.clone()).expect("root secret opens an Orange account")
        });
        if !options.readonly && !existed {
            store.save_air(&secret, &air)?;
        }
        let (event_tx, events) = mpsc::unbounded_channel();
        let mut session = Self {
            store,
            dir,
            secret,
            air,
            records: HashMap::new(),
            resolver: air::DefaultResolver::default(),
            client: None,
            events,
            event_tx,
            pipes: HashMap::new(),
            next_epoch: 1,
            readonly: options.readonly,
            offline: options.offline,
            timeout: options.timeout,
            verbose: options.verbose,
        };
        session.load_known()?;
        if !session.offline {
            session.sync().await?;
        }
        Ok(session)
    }

    pub fn me(&self) -> Name {
        self.secret.name()
    }

    pub async fn whoami(&mut self) -> Result<Value> {
        self.profile(true).await
    }

    pub async fn profile(&mut self, invite: bool) -> Result<Value> {
        self.ensure_profile(self.me()).await?;
        let profile = self.profile_of(self.me())?;
        let mut value = json!({
            "ok": true,
            "username": profile.username,
            "name": self.me().to_string(),
            "notes": profile.notes,
            "data_dir": self.dir.display().to_string(),
        });
        if invite {
            value["invite"] = json!(crate::contact_links::contact_url(self.me()));
        }
        Ok(value)
    }

    pub async fn profile_set(
        &mut self,
        username: Option<&str>,
        notes: Option<&str>,
        avatar: Option<&Path>,
    ) -> Result<Value> {
        self.reject_readonly()?;
        if username.is_none() && notes.is_none() && avatar.is_none() {
            return Err(CliError::usage(
                "profile set needs --username, --notes, or --avatar",
            ));
        }
        if let Some(username) = username
            && username.trim().is_empty()
        {
            return Err(CliError::usage("username cannot be empty"));
        }
        let location = self.ensure_profile(self.me()).await?;
        if let Some(username) = username {
            self.profile_action(
                location,
                ProfileAction::ChangeUsername(username.to_string()),
            )
            .await?;
        }
        if let Some(notes) = notes {
            self.profile_action(location, ProfileAction::ChangeNotes(notes.to_string()))
                .await?;
        }
        if let Some(path) = avatar {
            let image = media::load_image(path)?;
            self.profile_action(
                location,
                ProfileAction::ChangeAvatar(AvatarContent::image(image)),
            )
            .await?;
        }
        self.profile(true).await
    }

    pub async fn contacts(&mut self) -> Result<Value> {
        let mut contacts: Vec<_> = self
            .profile_list()
            .into_iter()
            .filter(|profile| profile.name != Some(self.me()))
            .map(|profile| {
                json!({
                    "username": profile.username,
                    "name": profile.name.map(|name| name.to_string()).unwrap_or_default(),
                    "notes": profile.notes,
                })
            })
            .collect();
        contacts.sort_by(|a, b| {
            a["username"]
                .as_str()
                .unwrap_or("")
                .cmp(b["username"].as_str().unwrap_or(""))
        });
        Ok(json!({"ok": true, "contacts": contacts}))
    }

    pub async fn contact_add(&mut self, who: &str) -> Result<Value> {
        self.reject_readonly()?;
        let name = self.parse_key(who)?;
        if name == self.me() {
            return Err(CliError::fail("you cannot add yourself as a contact"));
        }
        self.ensure_profile(name).await?;
        self.contacts().await
    }

    pub async fn rooms(&mut self) -> Result<Value> {
        let me = self.me();
        let mut rooms = Vec::new();
        for (location, id, room) in self.room_snapshots() {
            let unread = self.unread_count(id, &room);
            let (preview, stamp) = match room.messages(me).last() {
                Some(message) => (
                    present::preview(&message.body, &self.username(message.author)),
                    message.timestamp,
                ),
                None => ("No messages yet.".into(), 0),
            };
            rooms.push((
                stamp,
                json!({
                    "id": id.to_string(),
                    "init_id": room.id.to_string(),
                    "name": self.room_name(&room),
                    "group": room.is_group(),
                    "unread": unread,
                    "members": room.members.iter().map(|name| name.to_string()).collect::<Vec<_>>(),
                    "preview": preview,
                    "location": location_debug(location),
                }),
            ));
        }
        rooms.sort_by_key(|room| std::cmp::Reverse(room.0));
        Ok(
            json!({"ok": true, "rooms": rooms.into_iter().map(|(_, room)| room).collect::<Vec<_>>()}),
        )
    }

    pub async fn read(
        &mut self,
        room: &str,
        since: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Value> {
        let location = self.resolve_room(room)?;
        let id = self.instance_id(location)?;
        let data = self.room_of(location)?;
        let since = since.map(present::parse_since).transpose()?;
        let me = self.me();
        let mut messages: Vec<_> = data
            .messages(me)
            .filter(|message| since.is_none_or(|since| message.timestamp >= since))
            .cloned()
            .collect();
        if let Some(limit) = limit
            && messages.len() > limit
        {
            messages = messages.split_off(messages.len() - limit);
        }
        let unread = self.unread_count(id, &data);
        Ok(json!({
            "ok": true,
            "id": id.to_string(),
            "name": self.room_name(&data),
            "group": data.is_group(),
            "unread": unread,
            "messages": messages.iter().map(|message| self.message_value(id, message)).collect::<Vec<_>>(),
        }))
    }

    pub async fn send_text(&mut self, room: &str, text: &str) -> Result<Value> {
        let location = self.resolve_room(room)?;
        let (message, confirmed) = self
            .room_action(location, ChatRoomAction::SendMessage(text.to_string()))
            .await?;
        Ok(self.sent_value(location, &message, confirmed))
    }

    pub async fn dm(&mut self, who: &str, text: &str) -> Result<Value> {
        let person = self.resolve_person(who)?;
        let location = self.ensure_dm(person).await?;
        let (message, confirmed) = self
            .room_action(location, ChatRoomAction::SendMessage(text.to_string()))
            .await?;
        Ok(self.sent_value(location, &message, confirmed))
    }

    pub async fn send_media(
        &mut self,
        room: &str,
        file: &str,
        caption: Option<&str>,
    ) -> Result<Value> {
        let location = self.resolve_room(room)?;
        let members = self.room_of(location)?.members.clone();
        let (message, confirmed) = match media::load_attachment(Path::new(file))? {
            Attachment::Photo(photo) => {
                self.room_action(location, ChatRoomAction::SendPhoto(photo))
                    .await?
            }
            Attachment::Video { video, mp4 } => {
                self.publish_clip(mp4, &members).await?;
                self.room_action(location, ChatRoomAction::SendVideo(video))
                    .await?
            }
        };
        if let Some(caption) = caption
            && !caption.is_empty()
        {
            let _ = self
                .room_action(location, ChatRoomAction::SendMessage(caption.to_string()))
                .await?;
        }
        Ok(self.sent_value(location, &message, confirmed))
    }

    pub async fn send_gif(
        &mut self,
        room: &str,
        query: &str,
        index: usize,
        force_search: bool,
    ) -> Result<Value> {
        let gif = self.pick_gif(query, index, force_search).await?;
        let location = self.resolve_room(room)?;
        let (message, confirmed) = self
            .room_action(location, ChatRoomAction::SendGif(gif))
            .await?;
        Ok(self.sent_value(location, &message, confirmed))
    }

    pub async fn gif_search(&mut self, query: &str, index: usize) -> Result<Value> {
        let gifs = media::search_gifs((!query.is_empty()).then(|| query.to_string()), 0).await?;
        let listed: Vec<_> = gifs
            .iter()
            .enumerate()
            .map(|(number, gif)| {
                json!({
                    "index": number,
                    "id": gif.id,
                    "url": gif.url,
                    "width": gif.width,
                    "height": gif.height,
                })
            })
            .collect();
        Ok(json!({"ok": true, "selected": index, "gifs": listed}))
    }

    pub async fn download(
        &mut self,
        room: &str,
        message: &str,
        out: Option<&Path>,
    ) -> Result<Value> {
        let location = self.resolve_room(room)?;
        let id = message
            .parse::<Id>()
            .map_err(|_| CliError::fail("message id is not a 64-character hex id"))?;
        let data = self.room_of(location)?;
        let message = data
            .messages(self.me())
            .find(|message| message.id == id)
            .cloned()
            .ok_or_else(|| CliError::fail("message not found in this chat"))?;
        let (bytes, extension) = self.media_bytes(&message).await?;
        let path = out.map(Path::to_path_buf).unwrap_or_else(|| {
            std::path::PathBuf::from(format!("orange-{}.{}", &id.to_string()[..8], extension))
        });
        std::fs::write(&path, bytes)?;
        Ok(
            json!({"ok": true, "path": path.display().to_string(), "bytes": std::fs::metadata(&path)?.len()}),
        )
    }

    pub async fn mark_read(&mut self, room: &str) -> Result<Value> {
        self.reject_readonly()?;
        let location = self.resolve_room(room)?;
        let room_id = self.instance_id(location)?;
        let data = self.room_of(location)?;
        let read_at = self.ensure_reads(room_id).await?;
        let ids: Vec<Id> = self.read_of(read_at)?.unread(&data).collect();
        let count = ids.len();
        if !ids.is_empty() {
            self.send_reads(read_at, ids).await?;
        }
        Ok(json!({"ok": true, "room": room_id.to_string(), "marked": count}))
    }

    /// Stream messages that arrive after this call.
    ///
    /// Does not mark anything read, does not write the cache, and does not
    /// acknowledge notification watches.
    pub async fn watch(&mut self, only_room: Option<&str>, include_own: bool) -> Result<()> {
        self.watch_with(only_room, include_own, |value| {
            present::write_watch_line(&value)
        })
        .await
    }

    pub async fn watch_with(
        &mut self,
        only_room: Option<&str>,
        include_own: bool,
        mut emit: impl FnMut(Value),
    ) -> Result<()> {
        let only = only_room.map(|room| self.resolve_room(room)).transpose()?;
        let mut seen = HashSet::new();
        self.visit_messages(&mut |room, message| {
            seen.insert((room, message.id));
        });
        eprintln!("orange watch: listening (not acknowledging orange-listener)");
        loop {
            tokio::select! {
                incoming = self.events.recv() => {
                    if let Some(incoming) = incoming {self.apply(incoming).await?;}
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
            let _ = self.store.refresh();
            for event in self.take_new(&mut seen, include_own, only) {
                emit(event);
            }
        }
    }

    pub async fn delete_message(&mut self, room: &str, message: &str) -> Result<Value> {
        let location = self.resolve_room(room)?;
        let id = message
            .parse::<Id>()
            .map_err(|_| CliError::fail("message id is not a 64-character hex id"))?;
        let message = self
            .room_of(location)?
            .messages(self.me())
            .find(|message| message.id == id)
            .cloned()
            .ok_or_else(|| CliError::fail("message not found"))?;
        if matches!(message.body, MessageContent::System(_)) {
            return Err(CliError::fail("system messages cannot be deleted"));
        }
        self.room_action(location, ChatRoomAction::DeleteMessage(message))
            .await?;
        Ok(json!({"ok": true, "deleted": id.to_string()}))
    }

    pub async fn set_block(&mut self, who: &str, block: bool) -> Result<Value> {
        self.reject_readonly()?;
        let name = self.resolve_person(who)?;
        if name == self.me() {
            return Err(CliError::fail("you cannot block yourself"));
        }
        let location = self.ensure_block(name).await?;
        self.send_block(
            location,
            if block {
                BlockAction::Block
            } else {
                BlockAction::Unblock
            },
        )
        .await?;
        Ok(json!({"ok": true, "name": name.to_string(), "blocked": block}))
    }

    pub async fn blocks(&mut self) -> Result<Value> {
        let me = self.me();
        let mut blocked = Vec::new();
        for record in self.records.values() {
            let Record::Block(tracked) = record else {
                continue;
            };
            let Some(block) = tracked.instance.pending() else {
                continue;
            };
            if block.blocker == me && block.is_blocked {
                blocked.push(json!({
                    "name": block.blocked.to_string(),
                    "username": self.username(block.blocked),
                }));
            }
        }
        Ok(json!({"ok": true, "blocks": blocked}))
    }

    pub async fn typing(&mut self, room: &str, active: bool) -> Result<Value> {
        self.reject_readonly()?;
        let location = self.resolve_room(room)?;
        let room_id = self.instance_id(location)?;
        let typing = self.ensure_typing(room_id).await?;
        let members = self.room_of(location)?.members.clone();
        for member in members {
            if member == self.me() || self.blocked_pair(member) {
                continue;
            }
            self.share_location(member, typing).await?;
            self.send_typing(typing, TypingAction::Share(member))
                .await?;
        }
        self.send_typing(
            typing,
            TypingAction::Activity {
                active,
                at: TypingState::now(),
            },
        )
        .await?;
        Ok(json!({"ok": true, "room": room_id.to_string(), "active": active}))
    }

    pub async fn notify(&mut self, room: &str) -> Result<Value> {
        let location = self.resolve_room(room)?;
        let notify = match self.records.get(&location) {
            Some(Record::Room(tracked)) => tracked.notify,
            _ => false,
        };
        Ok(json!({
            "ok": true,
            "room": self.instance_id(location)?.to_string(),
            "local_flag": notify,
            "note": "Push watches stay with the GUI and orange-listener. This command does not acknowledge or remove them.",
        }))
    }

    pub async fn stickers(&mut self) -> Result<Value> {
        self.ensure_pack().await?;
        let stickers = self.pack_stickers();
        let listed: Vec<_> = stickers
            .iter()
            .enumerate()
            .map(|(index, sticker)| {
                json!({
                    "index": index,
                    "bytes": sticker.bytes().len(),
                    "width": sticker.image().dimensions.0,
                    "height": sticker.image().dimensions.1,
                })
            })
            .collect();
        Ok(json!({"ok": true, "stickers": listed}))
    }

    pub async fn sticker_add(&mut self, file: &str) -> Result<Value> {
        self.reject_readonly()?;
        let sticker = media::load_sticker(Path::new(file))?;
        let location = self.ensure_pack().await?;
        self.send_sticker_pack(location, StickerPackAction::Add(sticker))
            .await?;
        self.stickers().await
    }

    pub async fn send_sticker(&mut self, room: &str, index: usize) -> Result<Value> {
        self.ensure_pack().await?;
        let stickers = self.pack_stickers();
        let sticker = stickers
            .get(index)
            .cloned()
            .ok_or_else(|| CliError::fail(format!("no sticker at index {index}")))?;
        let location = self.resolve_room(room)?;
        let (message, confirmed) = self
            .room_action(location, ChatRoomAction::SendSticker(sticker))
            .await?;
        Ok(self.sent_value(location, &message, confirmed))
    }

    pub async fn group(
        &mut self,
        action: &str,
        args: &[String],
        options: &Options,
    ) -> Result<Value> {
        match action {
            "create" => {
                let name = args.first().map(String::as_str).unwrap_or("");
                if name.trim().is_empty() {
                    return Err(CliError::usage(
                        "group create <name> --member <who> --member <who>",
                    ));
                }
                if options.members.len() < 2 {
                    return Err(CliError::usage(
                        "a group needs two --member values; use dm for one person",
                    ));
                }
                self.group_create(name, &options.members).await
            }
            "add" => {
                let room = args
                    .first()
                    .ok_or_else(|| CliError::usage("group add <room> --member <who>"))?;
                if options.members.is_empty() {
                    return Err(CliError::usage("group add needs --member"));
                }
                self.group_add(room, &options.members).await
            }
            "leave" | "remove" => {
                if action == "remove" && !options.members.is_empty() {
                    let person = self.resolve_person(&options.members[0])?;
                    if person != self.me() {
                        return Err(CliError::fail(
                            "Orange groups have no kick. `group remove <room>` leaves the group.",
                        ));
                    }
                }
                let room = args
                    .first()
                    .ok_or_else(|| CliError::usage(format!("group {action} <room>")))?;
                self.group_leave(room).await
            }
            "rename" => {
                let room = args
                    .first()
                    .ok_or_else(|| CliError::usage("group rename <room> <name>"))?;
                let name = args[1..].join(" ");
                if name.trim().is_empty() {
                    return Err(CliError::usage("group rename <room> <name>"));
                }
                self.group_rename(room, &name).await
            }
            "photo" => {
                let room = args
                    .first()
                    .ok_or_else(|| CliError::usage("group photo <room> <file>"))?;
                let file = args
                    .get(1)
                    .ok_or_else(|| CliError::usage("group photo <room> <file>"))?;
                self.group_photo(room, file).await
            }
            _ => Err(CliError::usage(
                "group create|add|leave|remove|rename|photo",
            )),
        }
    }
}

impl Session {
    async fn sync(&mut self) -> Result<()> {
        if self.offline {
            return Ok(());
        }
        self.connect().await?;
        let start = Instant::now();
        let mut quiet = Instant::now();
        let mut saw = false;
        loop {
            tokio::select! {
                incoming = self.events.recv() => {
                    if let Some(incoming) = incoming {
                        self.apply(incoming).await?;
                        saw = true;
                        quiet = Instant::now();
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(200)) => {
                    if saw && quiet.elapsed() >= Duration::from_millis(500) {return Ok(())}
                    if start.elapsed() >= self.timeout {return Ok(())}
                }
            }
        }
    }

    async fn connect(&mut self) -> Result<()> {
        if self.client.is_some() || self.offline {
            return Ok(());
        }
        let identity = (*self.resolver.resolve(Name::orange_me(), None).await).clone();
        self.client = Some(connect_client(identity).await?);
        let requests = self.air.start();
        if !self.readonly {
            self.store.save_air(&self.secret, &self.air)?;
        }
        for (kind, request) in requests {
            self.spawn_pipe(PipeId::Catalog(kind), request).await?;
        }
        self.kick_all().await
    }

    fn load_known(&mut self) -> Result<()> {
        let locations: Vec<Location> = [
            ChatRoom::id(),
            Profile::id(),
            RoomReadState::id(),
            Block::id(),
            TypingState::id(),
            VideoClip::id(),
            StickerPack::id(),
        ]
        .into_iter()
        .flat_map(|contract| self.air.list(&contract))
        .collect();
        for location in locations {
            if self.records.contains_key(&location) {
                continue;
            }
            if let Some(record) = self.record_from_location(location)? {
                self.records.insert(location, record);
            }
        }
        Ok(())
    }

    fn record_from_location(&self, location: Location) -> Result<Option<Record>> {
        if location.contract == ChatRoom::id() {
            return Ok(Some(Record::Room(self.load_tracked(location)?)));
        }
        if location.contract == Profile::id() {
            return Ok(Some(Record::Profile(self.load_tracked(location)?)));
        }
        if location.contract == RoomReadState::id() {
            return Ok(Some(Record::Reads(self.load_tracked(location)?)));
        }
        if location.contract == Block::id() {
            return Ok(Some(Record::Block(self.load_tracked(location)?)));
        }
        if location.contract == TypingState::id() {
            return Ok(Some(Record::Typing(self.load_tracked(location)?)));
        }
        if location.contract == VideoClip::id() {
            return Ok(Some(Record::Clip(self.load_tracked(location)?)));
        }
        if location.contract == StickerPack::id() {
            return Ok(Some(Record::Pack(self.load_tracked(location)?)));
        }
        Ok(None)
    }

    fn load_tracked<C: Contract>(&self, location: Location) -> Result<Tracked<C>> {
        if let Some((instance, pending_outs, notify)) =
            self.store.load_instance(&self.secret, location)?
        {
            return Ok(Tracked {
                instance,
                pending_outs,
                notify,
                started: false,
            });
        }
        let instance = air::Instance::<C>::receive(self.secret.clone(), location)
            .map_err(|error| CliError::fail(format!("could not open chat record: {error}")))?;
        Ok(Tracked {
            instance,
            pending_outs: 0,
            notify: false,
            started: false,
        })
    }

    async fn apply(&mut self, incoming: Incoming) -> Result<()> {
        if self
            .pipes
            .get(&incoming.id)
            .is_none_or(|pipe| pipe.epoch != incoming.epoch)
        {
            return Ok(());
        }
        match incoming.body {
            IncomingBody::Closed => Ok(()),
            IncomingBody::Response(response) => match incoming.id {
                PipeId::Catalog(kind) => self.apply_catalog(kind, response).await,
                PipeId::Instance(location) => self.apply_instance(location, response).await,
            },
        }
    }

    async fn apply_catalog(
        &mut self,
        kind: air::RequestType,
        response: air::Response,
    ) -> Result<()> {
        let discovered = self.air.response(&mut self.resolver, kind, response).await;
        if !self.readonly {
            self.store.save_air(&self.secret, &self.air)?;
        }
        if let Some(location) = discovered {
            self.open_location(location).await?;
        }
        let follow = self.air.requests();
        for (kind, request) in follow {
            self.spawn_pipe(PipeId::Catalog(kind), request).await?;
        }
        Ok(())
    }

    async fn apply_instance(&mut self, location: Location, response: air::Response) -> Result<()> {
        if self.verbose {
            eprintln!("orange: instance response {response:?}");
        }
        let Some(mut record) = self.records.remove(&location) else {
            return Ok(());
        };
        let next = drive_response(&mut record, &mut self.resolver, response).await?;
        if !self.readonly {
            self.save_record(&record)?;
        }
        self.records.insert(location, record);
        if let Some(request) = next {
            self.spawn_pipe(PipeId::Instance(location), request).await?;
        }
        Ok(())
    }

    async fn open_location(&mut self, location: Location) -> Result<()> {
        if self.records.contains_key(&location) {
            return Ok(());
        }
        let Some(record) = self.record_from_location(location)? else {
            return Ok(());
        };
        self.records.insert(location, record);
        self.kick_location(location).await
    }

    async fn kick_all(&mut self) -> Result<()> {
        let locations: Vec<_> = self.records.keys().copied().collect();
        for location in locations {
            self.kick_location(location).await?;
        }
        Ok(())
    }

    async fn kick_location(&mut self, location: Location) -> Result<()> {
        let Some(mut record) = self.records.remove(&location) else {
            return Ok(());
        };
        let request = self
            .client
            .is_some()
            .then(|| take_request(&mut record, self.readonly))
            .flatten();
        if !self.readonly {
            self.save_record(&record)?;
        }
        self.records.insert(location, record);
        if let Some(request) = request {
            self.spawn_pipe(PipeId::Instance(location), request).await?;
        }
        Ok(())
    }

    async fn spawn_pipe(&mut self, id: PipeId, request: air::Request) -> Result<()> {
        if self.readonly && !matches!(request, air::Request::Read(_)) {
            if self.verbose {
                eprintln!("orange: left a queued write for orange-listener");
            }
            return Ok(());
        }
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| CliError::fail("not connected"))?;
        if self.verbose {
            eprintln!("orange: send {id:?}");
        }
        let rx = client.send(request).await;
        let epoch = self.next_epoch;
        self.next_epoch += 1;
        if let Some(old) = self.pipes.remove(&id) {
            old.task.abort();
        }
        let tx = self.event_tx.clone();
        let task = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(response) => {
                        if tx
                            .send(Incoming {
                                epoch,
                                id,
                                body: IncomingBody::Response(response),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(Incoming {
                            epoch,
                            id,
                            body: IncomingBody::Closed,
                        });
                        break;
                    }
                }
            }
        });
        self.pipes.insert(id, LivePipe { epoch, task });
        Ok(())
    }

    fn save_record(&self, record: &Record) -> Result<()> {
        if self.readonly {
            return Ok(());
        }
        match record {
            Record::Room(tracked) => self.store.save_instance(
                &self.secret,
                &tracked.instance,
                tracked.pending_outs,
                tracked.notify,
            ),
            Record::Profile(tracked) => self.store.save_instance(
                &self.secret,
                &tracked.instance,
                tracked.pending_outs,
                tracked.notify,
            ),
            Record::Reads(tracked) => self.store.save_instance(
                &self.secret,
                &tracked.instance,
                tracked.pending_outs,
                tracked.notify,
            ),
            Record::Block(tracked) => self.store.save_instance(
                &self.secret,
                &tracked.instance,
                tracked.pending_outs,
                tracked.notify,
            ),
            Record::Typing(tracked) => self.store.save_instance(
                &self.secret,
                &tracked.instance,
                tracked.pending_outs,
                tracked.notify,
            ),
            Record::Clip(tracked) => self.store.save_instance(
                &self.secret,
                &tracked.instance,
                tracked.pending_outs,
                tracked.notify,
            ),
            Record::Pack(tracked) => self.store.save_instance(
                &self.secret,
                &tracked.instance,
                tracked.pending_outs,
                tracked.notify,
            ),
        }
    }

    pub(crate) async fn room_action(
        &mut self,
        location: Location,
        action: ChatRoomAction,
    ) -> Result<(Message, bool)> {
        self.reject_readonly()?;
        self.member_or_err(location)?;
        // Server timestamps differ from the local id, so confirm by message body.
        let deleting = match &action {
            ChatRoomAction::DeleteMessage(message) => Some(message.clone()),
            _ => None,
        };
        {
            let Record::Room(tracked) = self
                .records
                .get_mut(&location)
                .ok_or_else(|| CliError::fail("chat not found"))?
            else {
                return Err(CliError::fail("chat not found"));
            };
            tracked.instance.send(action);
        }
        let message = if let Some(message) = deleting.clone() {
            message
        } else {
            self.room_of(location)?
                .messages(self.me())
                .last()
                .cloned()
                .ok_or_else(|| CliError::fail("the chat did not record that action"))?
        };
        let body = message.body.clone();
        let prior = self.confirmed_copies(location, &body);
        self.kick_location(location).await?;
        let confirmed = if deleting.is_some() {
            self.wait_until(|session| session.confirmed_copies(location, &body) < prior)
                .await
        } else {
            self.wait_until(|session| session.confirmed_copies(location, &body) > prior)
                .await
        };
        let message = self
            .room_of(location)?
            .messages(self.me())
            .find(|message| message.body == body)
            .cloned()
            .unwrap_or(message);
        Ok((message, confirmed))
    }

    async fn send_reads(&mut self, location: Location, ids: Vec<Id>) -> Result<()> {
        self.reject_readonly()?;
        {
            let Record::Reads(tracked) = self
                .records
                .get_mut(&location)
                .ok_or_else(|| CliError::fail("read state not open"))?
            else {
                return Err(CliError::fail("read state not open"));
            };
            tracked.instance.send(ids);
        }
        self.kick_location(location).await
    }

    async fn send_block(&mut self, location: Location, action: BlockAction) -> Result<()> {
        self.reject_readonly()?;
        {
            let Record::Block(tracked) = self
                .records
                .get_mut(&location)
                .ok_or_else(|| CliError::fail("block record not open"))?
            else {
                return Err(CliError::fail("block record not open"));
            };
            tracked.instance.send(action);
        }
        self.kick_location(location).await
    }

    async fn send_typing(&mut self, location: Location, action: TypingAction) -> Result<()> {
        self.reject_readonly()?;
        {
            let Record::Typing(tracked) = self
                .records
                .get_mut(&location)
                .ok_or_else(|| CliError::fail("typing state not open"))?
            else {
                return Err(CliError::fail("typing state not open"));
            };
            tracked.instance.send(action);
        }
        self.kick_location(location).await
    }

    async fn send_sticker_pack(
        &mut self,
        location: Location,
        action: StickerPackAction,
    ) -> Result<()> {
        self.reject_readonly()?;
        {
            let Record::Pack(tracked) = self
                .records
                .get_mut(&location)
                .ok_or_else(|| CliError::fail("sticker pack not open"))?
            else {
                return Err(CliError::fail("sticker pack not open"));
            };
            tracked.instance.send(action);
        }
        self.kick_location(location).await
    }

    fn confirmed_copies(&self, location: Location, body: &MessageContent) -> usize {
        match self.records.get(&location) {
            Some(Record::Room(tracked)) => tracked
                .instance
                .confirmed()
                .map(|room| {
                    room.messages(self.me())
                        .filter(|message| message.author == self.me() && &message.body == body)
                        .count()
                })
                .unwrap_or(0),
            _ => 0,
        }
    }

    async fn wait_until(&mut self, mut pred: impl FnMut(&mut Self) -> bool) -> bool {
        if self.offline || self.timeout.is_zero() || pred(self) {
            return pred(self);
        }
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            if pred(self) {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::select! {
                incoming = self.events.recv() => {
                    if let Some(incoming) = incoming {
                        if let Err(error) = self.apply(incoming).await && self.verbose {
                            eprintln!("orange: {error}");
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline) => return pred(self),
            }
        }
    }

    async fn ensure_dm(&mut self, person: Name) -> Result<Location> {
        self.reject_readonly()?;
        if let Some(location) = self.find_dm(person) {
            return Ok(location);
        }
        let init = Id::hash(&person);
        let mut instance = self
            .air
            .create::<ChatRoom>(init)
            .map_err(|error| CliError::fail(format!("{error}")))?;
        if person != self.me() {
            instance.send(ChatRoomAction::Share(person));
        }
        let location = *instance.location();
        self.records.insert(
            location,
            Record::Room(Tracked {
                instance,
                pending_outs: 0,
                notify: true,
                started: false,
            }),
        );
        if !self.readonly {
            self.store.save_air(&self.secret, &self.air)?;
        }
        if person != self.me() {
            self.share_location(person, location).await?;
        }
        self.kick_location(location).await?;
        Ok(location)
    }

    async fn group_create(&mut self, name: &str, members: &[String]) -> Result<Value> {
        self.reject_readonly()?;
        let mut people = Vec::new();
        for member in members {
            people.push(self.resolve_person(member)?);
        }
        people.retain(|name| *name != self.me());
        people.sort_unstable();
        people.dedup();
        if people.len() < 2 {
            return Err(CliError::usage("a group needs two other people"));
        }
        let mut instance = self
            .air
            .create::<ChatRoom>(Id::random())
            .map_err(|error| CliError::fail(format!("{error}")))?;
        instance.send(ChatRoomAction::CreateGroup);
        for person in &people {
            instance.send(ChatRoomAction::AddMember(*person));
        }
        instance.send(ChatRoomAction::RenameGroup(name.trim().to_string()));
        let location = *instance.location();
        self.records.insert(
            location,
            Record::Room(Tracked {
                instance,
                pending_outs: 0,
                notify: true,
                started: false,
            }),
        );
        if !self.readonly {
            self.store.save_air(&self.secret, &self.air)?;
        }
        for person in &people {
            self.share_location(*person, location).await?;
        }
        self.kick_location(location).await?;
        Ok(json!({"ok": true, "id": self.instance_id(location)?.to_string(), "name": name.trim()}))
    }

    async fn group_add(&mut self, room: &str, members: &[String]) -> Result<Value> {
        self.reject_readonly()?;
        let location = self.resolve_room(room)?;
        let current = self.room_of(location)?;
        if !current.is_group() || !current.members.contains(&self.me()) {
            return Err(CliError::fail("you are not a member of that group"));
        }
        let mut people = Vec::new();
        for member in members {
            let person = self.resolve_person(member)?;
            if !current.members.contains(&person) {
                people.push(person);
            }
        }
        people.sort_unstable();
        people.dedup();
        let clips = self.clip_locations(&current);
        for person in &people {
            self.tracked_room_send(location, ChatRoomAction::AddMember(*person))
                .await?;
            self.share_location(*person, location).await?;
            for clip in &clips {
                self.share_location(*person, *clip).await?;
            }
        }
        Ok(json!({"ok": true, "added": people.len()}))
    }

    async fn group_leave(&mut self, room: &str) -> Result<Value> {
        let location = self.resolve_room(room)?;
        self.room_action(location, ChatRoomAction::LeaveGroup)
            .await?;
        if let Some(Record::Room(tracked)) = self.records.get_mut(&location) {
            tracked.notify = false;
        }
        self.kick_location(location).await?;
        Ok(json!({"ok": true, "left": self.instance_id(location)?.to_string()}))
    }

    async fn group_rename(&mut self, room: &str, name: &str) -> Result<Value> {
        let location = self.resolve_room(room)?;
        if name.trim().chars().count() > 80 {
            return Err(CliError::fail("use 80 characters or fewer"));
        }
        self.room_action(location, ChatRoomAction::RenameGroup(name.to_string()))
            .await?;
        Ok(json!({"ok": true, "name": name.trim()}))
    }

    async fn group_photo(&mut self, room: &str, file: &str) -> Result<Value> {
        let location = self.resolve_room(room)?;
        let image = media::load_image(Path::new(file))?;
        self.room_action(
            location,
            ChatRoomAction::SetGroupPhoto(AvatarContent::image(image)),
        )
        .await?;
        Ok(json!({"ok": true}))
    }

    async fn tracked_room_send(
        &mut self,
        location: Location,
        action: ChatRoomAction,
    ) -> Result<()> {
        {
            let Record::Room(tracked) = self
                .records
                .get_mut(&location)
                .ok_or_else(|| CliError::fail("chat not found"))?
            else {
                return Err(CliError::fail("chat not found"));
            };
            tracked.instance.send(action);
        }
        self.kick_location(location).await
    }

    async fn ensure_profile(&mut self, name: Name) -> Result<Location> {
        if let Some(location) = self.find_profile_location(name) {
            return Ok(location);
        }
        let location = air::Instance::<Profile>::generate_location(&self.secret, &name)
            .map_err(|error| CliError::fail(format!("{error}")))?;
        if self.air.list(&Profile::id()).contains(&location) {
            self.open_location(location).await?;
            return Ok(location);
        }
        if self.readonly {
            return Err(CliError::fail("profile is not in the local cache"));
        }
        let mut instance = self
            .air
            .create::<Profile>(name)
            .map_err(|error| CliError::fail(format!("{error}")))?;
        instance.send(ProfileAction::SetUsernameInit(Username::new()));
        self.records.insert(
            location,
            Record::Profile(Tracked {
                instance,
                pending_outs: 0,
                notify: false,
                started: false,
            }),
        );
        self.store.save_air(&self.secret, &self.air)?;
        self.kick_location(location).await?;
        Ok(location)
    }

    async fn ensure_reads(&mut self, room: Id) -> Result<Location> {
        if let Some(location) = self.find_contract(|record| {
            let Record::Reads(tracked) = record else {
                return false;
            };
            tracked
                .instance
                .pending()
                .is_some_and(|state| state.room == room && state.owner == self.me())
        }) {
            return Ok(location);
        }
        let mut instance = self
            .air
            .create::<RoomReadState>(room)
            .map_err(|error| CliError::fail(format!("{error}")))?;
        let location = *instance.location();
        let _ = &mut instance;
        self.records.insert(
            location,
            Record::Reads(Tracked {
                instance,
                pending_outs: 0,
                notify: false,
                started: false,
            }),
        );
        self.store.save_air(&self.secret, &self.air)?;
        self.kick_location(location).await?;
        Ok(location)
    }

    async fn ensure_block(&mut self, name: Name) -> Result<Location> {
        if let Some(location) = self.find_contract(|record| {
            let Record::Block(tracked) = record else {
                return false;
            };
            tracked
                .instance
                .pending()
                .is_some_and(|block| block.blocker == self.me() && block.blocked == name)
        }) {
            return Ok(location);
        }
        let instance = self
            .air
            .create::<Block>(name)
            .map_err(|error| CliError::fail(format!("{error}")))?;
        let location = *instance.location();
        self.records.insert(
            location,
            Record::Block(Tracked {
                instance,
                pending_outs: 0,
                notify: false,
                started: false,
            }),
        );
        self.store.save_air(&self.secret, &self.air)?;
        if name != self.me() {
            self.share_location(name, location).await?;
        }
        self.kick_location(location).await?;
        Ok(location)
    }

    async fn ensure_typing(&mut self, room: Id) -> Result<Location> {
        if let Some(location) = self.find_contract(|record| {
            let Record::Typing(tracked) = record else {
                return false;
            };
            tracked
                .instance
                .pending()
                .is_some_and(|state| state.room == room && state.author == self.me())
        }) {
            return Ok(location);
        }
        let instance = self
            .air
            .create::<TypingState>(room)
            .map_err(|error| CliError::fail(format!("{error}")))?;
        let location = *instance.location();
        self.records.insert(
            location,
            Record::Typing(Tracked {
                instance,
                pending_outs: 0,
                notify: false,
                started: false,
            }),
        );
        self.store.save_air(&self.secret, &self.air)?;
        self.kick_location(location).await?;
        Ok(location)
    }

    async fn ensure_pack(&mut self) -> Result<Location> {
        if let Some(location) = self.find_contract(|record| {
            let Record::Pack(tracked) = record else {
                return false;
            };
            tracked
                .instance
                .pending()
                .is_some_and(|pack| pack.owner == self.me())
        }) {
            return Ok(location);
        }
        if self.readonly {
            return Err(CliError::fail("no stickers in the local cache"));
        }
        let instance = self
            .air
            .create::<StickerPack>(self.me())
            .map_err(|error| CliError::fail(format!("{error}")))?;
        let location = *instance.location();
        self.records.insert(
            location,
            Record::Pack(Tracked {
                instance,
                pending_outs: 0,
                notify: false,
                started: false,
            }),
        );
        self.store.save_air(&self.secret, &self.air)?;
        self.kick_location(location).await?;
        Ok(location)
    }

    async fn publish_clip(&mut self, bytes: std::sync::Arc<[u8]>, members: &[Name]) -> Result<()> {
        let init = <VideoClip as Contract>::Init::new(bytes);
        let clip_id = init.id();
        if let Some(location) = self.find_contract(|record| {
            let Record::Clip(tracked) = record else {
                return false;
            };
            tracked
                .instance
                .pending()
                .is_some_and(|clip| clip.id == clip_id)
        }) {
            let people: Vec<Name> = members
                .iter()
                .copied()
                .filter(|name| *name != self.me())
                .collect();
            for member in people {
                self.share_location(member, location).await?;
            }
            return Ok(());
        }
        let instance = self
            .air
            .create::<VideoClip>(init)
            .map_err(|error| CliError::fail(format!("{error}")))?;
        let location = *instance.location();
        self.records.insert(
            location,
            Record::Clip(Tracked {
                instance,
                pending_outs: 0,
                notify: false,
                started: false,
            }),
        );
        self.store.save_air(&self.secret, &self.air)?;
        let people: Vec<Name> = members
            .iter()
            .copied()
            .filter(|name| *name != self.me())
            .collect();
        for member in people {
            self.share_location(member, location).await?;
        }
        self.kick_location(location).await?;
        Ok(())
    }

    async fn share_location(&mut self, name: Name, location: Location) -> Result<()> {
        if name == self.me() || self.readonly {
            return Ok(());
        }
        self.air.share(&mut self.resolver, name, location).await;
        self.store.save_air(&self.secret, &self.air)?;
        if self.client.is_none() {
            return Ok(());
        }
        let follow = self.air.requests();
        for (kind, request) in follow {
            self.spawn_pipe(PipeId::Catalog(kind), request).await?;
        }
        Ok(())
    }

    async fn profile_action(&mut self, location: Location, action: ProfileAction) -> Result<()> {
        {
            let Record::Profile(tracked) = self
                .records
                .get_mut(&location)
                .ok_or_else(|| CliError::fail("profile not open"))?
            else {
                return Err(CliError::fail("profile not open"));
            };
            tracked.instance.send(action);
        }
        self.kick_location(location).await?;
        Ok(())
    }

    async fn pick_gif(
        &mut self,
        query: &str,
        index: usize,
        force_search: bool,
    ) -> Result<chk::messages::Gif> {
        let token = query.trim();
        if !force_search
            && !token.contains(char::is_whitespace)
            && token.bytes().all(|byte| byte.is_ascii_alphanumeric())
            && token.len() >= 5
        {
            if let Ok(gif) = media::gif_by_id(token) {
                return Ok(gif);
            }
        }
        let gifs = media::search_gifs(Some(token.to_string()), 0).await?;
        gifs.into_iter()
            .nth(index)
            .ok_or_else(|| CliError::fail(format!("GIF index {index} is past the search results")))
    }

    async fn media_bytes(&mut self, message: &Message) -> Result<(Vec<u8>, &'static str)> {
        match &message.body {
            MessageContent::Photo(photo) => Ok((photo.jpeg().to_vec(), "jpg")),
            MessageContent::Sticker(sticker) => Ok((sticker.bytes().to_vec(), "jpg")),
            MessageContent::Gif(gif) => {
                let agent = ureq::AgentBuilder::new()
                    .timeout(Duration::from_secs(20))
                    .build();
                let bytes = agent
                    .get(&gif.url)
                    .call()
                    .map_err(|error| CliError::fail(format!("{error}")))?
                    .into_reader()
                    .bytes()
                    .collect::<std::io::Result<Vec<_>>>()?;
                Ok((bytes, "gif"))
            }
            MessageContent::Video(video) => {
                self.sync().await?;
                let clip = self
                    .find_clip(video.clip)
                    .ok_or_else(|| CliError::fail("video file has not arrived yet"))?;
                Ok((media::clip_bytes(&clip)?, "mp4"))
            }
            _ => Err(CliError::fail(
                "that message has no photo, video, sticker, or GIF to save",
            )),
        }
    }

    fn find_clip(&self, id: Id) -> Option<VideoClip> {
        for record in self.records.values() {
            let Record::Clip(tracked) = record else {
                continue;
            };
            if let Some(clip) = tracked.instance.pending()
                && clip.id == id
            {
                return Some(clip.clone());
            }
        }
        None
    }

    fn clip_locations(&self, room: &ChatRoom) -> Vec<Location> {
        let wanted: Vec<Id> = room
            .messages(self.me())
            .filter_map(|message| match &message.body {
                MessageContent::Video(video) => Some(video.clip),
                _ => None,
            })
            .collect();
        self.records
            .iter()
            .filter_map(|(location, record)| {
                let Record::Clip(tracked) = record else {
                    return None;
                };
                tracked
                    .instance
                    .pending()
                    .is_some_and(|clip| wanted.contains(&clip.id))
                    .then_some(*location)
            })
            .collect()
    }

    fn pack_stickers(&self) -> Vec<pelican_ui::components::Sticker> {
        self.records
            .values()
            .find_map(|record| {
                let Record::Pack(tracked) = record else {
                    return None;
                };
                tracked
                    .instance
                    .pending()
                    .filter(|pack| pack.owner == self.me())
                    .map(|pack| pack.stickers.clone())
            })
            .unwrap_or_default()
    }

    fn find_profile_location(&self, name: Name) -> Option<Location> {
        self.find_contract(|record| {
            let Record::Profile(tracked) = record else {
                return false;
            };
            tracked
                .instance
                .pending()
                .is_some_and(|profile| profile.name == Some(name))
        })
    }

    fn find_contract(&self, mut pred: impl FnMut(&Record) -> bool) -> Option<Location> {
        self.records
            .iter()
            .find_map(|(location, record)| pred(record).then_some(*location))
    }

    fn find_dm(&self, person: Name) -> Option<Location> {
        let me = self.me();
        let mut found = Vec::new();
        for (location, id, room) in self.room_snapshots() {
            if room.is_group() || !room.members.contains(&me) || !room.members.contains(&person) {
                continue;
            }
            if room
                .members
                .iter()
                .any(|name| *name != me && *name != person)
            {
                continue;
            }
            found.push((room.messages(me).next().is_none(), id, location));
        }
        found.sort_by_key(|(empty, id, _)| (*empty, *id));
        found.first().map(|(_, _, location)| *location)
    }

    pub(crate) fn resolve_room(&self, input: &str) -> Result<Location> {
        let input = input.trim();
        if let Ok(id) = input.parse::<Id>() {
            for (location, instance_id, room) in self.room_snapshots() {
                if instance_id == id || room.id == id {
                    return Ok(location);
                }
            }
        }
        let mut names = Vec::new();
        for (location, _, room) in self.room_snapshots() {
            if self.room_name(&room).eq_ignore_ascii_case(input) {
                names.push(location);
            }
        }
        if names.len() == 1 {
            return Ok(names[0]);
        }
        if let Ok(person) = self.resolve_person(input)
            && let Some(location) = self.find_dm(person)
        {
            return Ok(location);
        }
        Err(CliError::fail(format!(
            "no chat matches {input}. Run `orange rooms` for ids."
        )))
    }

    fn resolve_person(&self, input: &str) -> Result<Name> {
        if let Some(name) = self.parse_key_opt(input) {
            return Ok(name);
        }
        let mut matches = Vec::new();
        for profile in self.profile_list() {
            if profile.username.eq_ignore_ascii_case(input.trim())
                && let Some(name) = profile.name
            {
                matches.push(name);
            }
        }
        match matches.as_slice() {
            [name] => Ok(*name),
            [] => Err(CliError::fail(format!(
                "{input} is not an Orange name, invite link, or known username"
            ))),
            _ => Err(CliError::fail(format!(
                "more than one contact is named {input}"
            ))),
        }
    }

    fn parse_key(&self, input: &str) -> Result<Name> {
        self.parse_key_opt(input)
            .ok_or_else(|| CliError::fail(format!("{input} is not an Orange name or invite link")))
    }

    fn parse_key_opt(&self, input: &str) -> Option<Name> {
        crate::contact_links::parse_contact(input)
    }

    fn room_snapshots(&self) -> Vec<(Location, Id, ChatRoom)> {
        self.records
            .iter()
            .filter_map(|(location, record)| {
                let Record::Room(tracked) = record else {
                    return None;
                };
                let room = tracked.instance.pending()?.clone();
                Some((*location, tracked.instance.id(), room))
            })
            .collect()
    }

    fn room_of(&self, location: Location) -> Result<ChatRoom> {
        match self.records.get(&location) {
            Some(Record::Room(tracked)) => tracked
                .instance
                .pending()
                .cloned()
                .ok_or_else(|| CliError::fail("chat is not available yet")),
            _ => Err(CliError::fail("chat not found")),
        }
    }

    fn profile_of(&self, name: Name) -> Result<Profile> {
        let location = self
            .find_profile_location(name)
            .ok_or_else(|| CliError::fail("profile is not available yet"))?;
        match self.records.get(&location) {
            Some(Record::Profile(tracked)) => tracked
                .instance
                .pending()
                .cloned()
                .ok_or_else(|| CliError::fail("profile is not available yet")),
            _ => Err(CliError::fail("profile is not available yet")),
        }
    }

    fn profile_list(&self) -> Vec<Profile> {
        self.records
            .values()
            .filter_map(|record| {
                let Record::Profile(tracked) = record else {
                    return None;
                };
                tracked.instance.pending().cloned()
            })
            .collect()
    }

    fn read_of(&self, location: Location) -> Result<RoomReadState> {
        match self.records.get(&location) {
            Some(Record::Reads(tracked)) => tracked
                .instance
                .pending()
                .cloned()
                .ok_or_else(|| CliError::fail("read state is not available yet")),
            _ => Err(CliError::fail("read state is not available yet")),
        }
    }

    fn instance_id(&self, location: Location) -> Result<Id> {
        self.records
            .get(&location)
            .map(|record| match record {
                Record::Room(tracked) => tracked.instance.id(),
                Record::Profile(tracked) => tracked.instance.id(),
                Record::Reads(tracked) => tracked.instance.id(),
                Record::Block(tracked) => tracked.instance.id(),
                Record::Typing(tracked) => tracked.instance.id(),
                Record::Clip(tracked) => tracked.instance.id(),
                Record::Pack(tracked) => tracked.instance.id(),
            })
            .ok_or_else(|| CliError::fail("record not open"))
    }

    fn room_name(&self, room: &ChatRoom) -> String {
        if room.is_group() {
            return room.group_details().name;
        }
        let me = self.me();
        let member = if room.members.len() == 1 {
            room.members[0]
        } else {
            room.members
                .iter()
                .copied()
                .find(|name| *name != me)
                .unwrap_or(me)
        };
        let username = self.username(member);
        if member == self.me() {
            format!("{username} (You)")
        } else {
            username
        }
    }

    fn username(&self, name: Name) -> String {
        self.profile_list()
            .into_iter()
            .find(|profile| profile.name == Some(name))
            .map(|profile| profile.username)
            .unwrap_or_else(|| present::short_name(&name))
    }

    fn unread_count(&self, room: Id, data: &ChatRoom) -> usize {
        let me = self.me();
        let read = self.records.values().find_map(|record| {
            let Record::Reads(tracked) = record else {
                return None;
            };
            let state = tracked.instance.pending()?;
            (state.room == room && state.owner == me).then_some(state.clone())
        });
        match read {
            Some(state) => state.unread(data).count(),
            None => data
                .messages(me)
                .filter(|message| message.author != me)
                .count(),
        }
    }

    fn member_or_err(&self, location: Location) -> Result<()> {
        let room = self.room_of(location)?;
        if room.members.contains(&self.me()) {
            Ok(())
        } else {
            Err(CliError::fail("you are not a member of that chat"))
        }
    }

    fn blocked_pair(&self, name: Name) -> bool {
        let me = self.me();
        self.records.values().any(|record| {
            let Record::Block(tracked) = record else {
                return false;
            };
            tracked.instance.pending().is_some_and(|block| {
                block.is_blocked
                    && ((block.blocker == me && block.blocked == name)
                        || (block.blocker == name && block.blocked == me))
            })
        })
    }

    fn message_value(&self, room: Id, message: &Message) -> Value {
        let username = self.username(message.author);
        json!({
            "id": message.id.to_string(),
            "room": room.to_string(),
            "sender": message.author.to_string(),
            "sender_username": username,
            "timestamp": message.timestamp,
            "time": present::stamp(message.timestamp),
            "preview": present::preview(&message.body, &username),
            "content": present::content_json(&message.body, &username),
        })
    }

    fn sent_value(&self, location: Location, message: &Message, confirmed: bool) -> Value {
        let mut value = self.message_value(self.instance_id(location).unwrap_or_default(), message);
        value["ok"] = json!(true);
        value["confirmed"] = json!(confirmed);
        value
    }

    fn visit_messages(&self, visit: &mut dyn FnMut(Id, Message)) {
        for (_location, id, room) in self.room_snapshots() {
            for message in room.messages(self.me()) {
                visit(id, message.clone());
            }
        }
        if let Ok(Some(account)) = self.store.load_air(&self.secret) {
            for location in account.list(&ChatRoom::id()) {
                let Ok(Some((instance, _, _))) =
                    self.store.load_instance::<ChatRoom>(&self.secret, location)
                else {
                    continue;
                };
                let Some(room) = instance.pending() else {
                    continue;
                };
                for message in room.messages(self.me()) {
                    visit(instance.id(), message.clone());
                }
            }
        }
    }

    fn take_new(
        &self,
        seen: &mut HashSet<(Id, Id)>,
        include_own: bool,
        only: Option<Location>,
    ) -> Vec<Value> {
        let only_id = only.and_then(|location| self.instance_id(location).ok());
        let mut events = Vec::new();
        self.visit_messages(&mut |room, message| {
            if only_id.is_some_and(|id| id != room) {
                return;
            }
            if !seen.insert((room, message.id)) {
                return;
            }
            let system = matches!(message.body, MessageContent::System(_));
            if !include_own && message.author == self.me() && !system {
                return;
            }
            events.push(watch_event(room, &message, &self.username(message.author)));
        });
        events
    }

    fn reject_readonly(&self) -> Result<()> {
        if self.readonly {
            Err(CliError::fail(
                "orange watch only listens. It does not send, mark read, or write the cache.",
            ))
        } else {
            Ok(())
        }
    }
}

fn take_request(record: &mut Record, readonly: bool) -> Option<air::Request> {
    match record {
        Record::Room(tracked) => next_request(tracked, readonly),
        Record::Profile(tracked) => next_request(tracked, readonly),
        Record::Reads(tracked) => next_request(tracked, readonly),
        Record::Block(tracked) => next_request(tracked, readonly),
        Record::Typing(tracked) => next_request(tracked, readonly),
        Record::Clip(tracked) => next_request(tracked, readonly),
        Record::Pack(tracked) => next_request(tracked, readonly),
    }
}

fn next_request<C: Contract>(tracked: &mut Tracked<C>, readonly: bool) -> Option<air::Request> {
    let backup = tracked.instance.clone();
    let request = if !tracked.started {
        tracked.started = true;
        Some(tracked.instance.start())
    } else {
        tracked.instance.request()
    }?;
    if readonly && !matches!(request, air::Request::Read(_)) {
        tracked.instance = backup;
        return None;
    }
    Some(request)
}

async fn drive_response(
    record: &mut Record,
    resolver: &mut air::DefaultResolver,
    response: air::Response,
) -> Result<Option<air::Request>> {
    // Do not translate this response into Notification::Received. That
    // acknowledgement is what orange-listener uses to retire a wake.
    match record {
        Record::Room(tracked) => finish(&mut tracked.instance, resolver, response).await,
        Record::Profile(tracked) => finish(&mut tracked.instance, resolver, response).await,
        Record::Reads(tracked) => finish(&mut tracked.instance, resolver, response).await,
        Record::Block(tracked) => finish(&mut tracked.instance, resolver, response).await,
        Record::Typing(tracked) => finish(&mut tracked.instance, resolver, response).await,
        Record::Clip(tracked) => finish(&mut tracked.instance, resolver, response).await,
        Record::Pack(tracked) => finish(&mut tracked.instance, resolver, response).await,
    }
}

async fn finish<C: Contract>(
    instance: &mut air::Instance<C>,
    resolver: &mut air::DefaultResolver,
    response: air::Response,
) -> Result<Option<air::Request>> {
    let _kept = instance.response(resolver, response).await;
    Ok(instance.request())
}

fn location_debug(location: Location) -> String {
    format!("{}", location.instance)
}

async fn connect_client(identity: air::Identity) -> Result<air_server::Client> {
    let mut delay = Duration::from_millis(250);
    let mut last = String::from("no attempt");
    for _ in 0..6 {
        match air_server::Client::new(identity.clone(), "wss").await {
            Ok(client) => return Ok(client),
            Err(error) => {
                last = error;
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        }
    }
    Err(CliError::fail(format!(
        "could not reach the Orange network at air.orange.me: {last}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chk::messages::{ChatRoomAction, MessageContent};

    fn options(dir: &Path, readonly: bool, offline: bool) -> Options {
        Options {
            json: true,
            offline,
            verbose: false,
            readonly,
            include_own: false,
            data_dir: Some(dir.to_path_buf()),
            timeout: Duration::from_secs(40),
            positionals: vec![],
            members: vec![],
            since: None,
            limit: None,
            index: 0,
            caption: None,
            out: None,
            username: None,
            notes: None,
            avatar: None,
            stop_typing: false,
            force_search: false,
        }
    }

    #[tokio::test]
    async fn readonly_watch_does_not_touch_the_cache() {
        let dir = std::env::temp_dir().join(format!("orange-cli-ro-{}", air::Id::random()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut created = Session::open(&options(&dir, false, true)).await.unwrap();
        created
            .profile_set(Some("Watcher"), None, None)
            .await
            .unwrap();
        drop(created);
        let before = std::fs::read(dir.join("CACHE")).unwrap();
        let mut session = Session::open(&options(&dir, true, true)).await.unwrap();
        assert_eq!(std::fs::read(dir.join("CACHE")).unwrap(), before);
        let value = session.whoami().await.unwrap();
        assert_eq!(value["username"], "Watcher");
        assert_eq!(std::fs::read(dir.join("CACHE")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "talks to air.orange.me; run with -- --ignored two_identities"]
    async fn two_identities_send_read_watch_and_media() {
        let root = std::env::temp_dir().join(format!("orange-cli-net-{}", air::Id::random()));
        let alice_dir = root.join("alice");
        let bob_dir = root.join("bob");
        let mut alice = Session::open(&options(&alice_dir, false, false))
            .await
            .expect("alice");
        let bob = Session::open(&options(&bob_dir, false, false))
            .await
            .expect("bob");
        let alice_name = alice.me().to_string();
        let bob_name = bob.me().to_string();
        drop(bob);

        let sent = alice.dm(&bob_name, "hello from alice").await.expect("dm");
        assert_eq!(sent["content"]["text"], "hello from alice", "{sent}");
        assert_eq!(
            sent["confirmed"], true,
            "server did not confirm the text: {sent}"
        );

        let mut bob = Session::open(&options(&bob_dir, false, false))
            .await
            .expect("bob reopen");
        let history = bob.read(&alice_name, None, None).await.expect("read");
        let texts: Vec<_> = history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|message| message["content"]["text"].as_str())
            .collect();
        assert!(texts.contains(&"hello from alice"), "{history}");

        let image = ::media::Image::new(
            (2, 2),
            vec![
                ::media::Pixel {
                    red: 200,
                    green: 10,
                    blue: 10,
                    alpha: 255
                };
                4
            ],
        );
        let photo = root.join("dot.png");
        std::fs::create_dir_all(&root).unwrap();
        image
            .write(
                std::fs::File::create(&photo).unwrap(),
                ::media::ImageFormat::Png,
            )
            .unwrap();
        let room = history["id"].as_str().unwrap().to_string();
        let media_sent = alice
            .send_media(&room, photo.to_str().unwrap(), Some("a photo"))
            .await
            .expect("photo");
        assert_eq!(media_sent["content"]["type"], "photo", "{media_sent}");
        assert_eq!(media_sent["confirmed"], true, "{media_sent}");

        let gif = chk::messages::Gif {
            id: "cli-test".into(),
            url: "https://example.invalid/orange.gif".into(),
            width: 8,
            height: 8,
            preview: "https://example.invalid/orange-small.gif".into(),
        };
        let (gif_message, gif_confirmed) = alice
            .room_action(
                alice.resolve_room(&room).unwrap(),
                ChatRoomAction::SendGif(gif),
            )
            .await
            .expect("gif");
        assert!(gif_confirmed, "gif was not confirmed");
        assert!(matches!(gif_message.body, MessageContent::Gif(_)));

        let mut bob = Session::open(&options(&bob_dir, false, false))
            .await
            .expect("bob before watch");
        let settled = bob
            .read(&alice_name, None, None)
            .await
            .expect("settled history");
        assert!(
            settled["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["content"]["type"] == "photo"),
            "{settled}"
        );
        assert!(
            settled["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["content"]["type"] == "gif"),
            "{settled}"
        );

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut listener = bob;
        let watched_name = alice_name.clone();
        let watch = tokio::spawn(async move {
            listener
                .watch_with(Some(&watched_name), false, move |event| {
                    let _ = tx.send(event);
                })
                .await
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        let sent = alice
            .send_text(&room, "while you watch")
            .await
            .expect("watch send");
        assert_eq!(sent["confirmed"], true, "{sent}");
        let event = tokio::time::timeout(Duration::from_secs(40), rx.recv())
            .await
            .expect("timed out waiting for watch")
            .expect("watch ended");
        assert_eq!(event["event"], "message");
        assert_eq!(event["sender"], alice_name);
        assert!(event.get("room").is_some());
        assert!(event.get("timestamp").is_some());
        assert_eq!(event["content"]["text"], "while you watch", "{event}");
        watch.abort();

        let mut bob = Session::open(&options(&bob_dir, false, false))
            .await
            .unwrap();
        let again = bob.read(&alice_name, None, None).await.unwrap();
        let photo_id = again["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|message| {
                (message["content"]["type"] == "photo")
                    .then(|| message["id"].as_str().unwrap().to_string())
            })
            .expect(&format!("photo missing: {again}"));
        let saved = bob
            .download(&alice_name, &photo_id, Some(&root.join("out.jpg")))
            .await
            .expect("download");
        assert!(
            std::fs::metadata(saved["path"].as_str().unwrap())
                .unwrap()
                .len()
                > 0
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
