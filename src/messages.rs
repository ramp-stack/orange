use chk::{
    Action, Context, Display, Flow, Form, FormAction,
    FormComplete, FormItem, FormSubmit, FormValidState, Icons, ListItem, Page, PageType, Root,
    State, Theme, Watch
};
use chk::air::{messages::{ChatRoom, RoomReadState}, profiles::Profile};
use ramp::prism::air::{Air, Instance, Instances, Name};
use ramp::prism::service::Service;

use std::str::FromStr;

type MessageLists = (Vec<Instance<ChatRoom>>, Vec<Instance<Profile>>, Vec<Instance<RoomReadState>>);

pub struct MessagesHome(Instances<ChatRoom>, Air);
impl MessagesHome {
    pub fn build() -> Root {
        Root::custom(Page::watching((Watch::list::<ChatRoom>(), Watch::list::<Profile>(), Watch::list::<RoomReadState>()),
            |ctx: &mut Context, _theme: &Theme, (rooms, _, reads): MessageLists| {
                let me = ctx.air.me();
                let reads = reads.into_iter().filter_map(|instance| {
                    let read = instance.pending();
                    (read.owner == me).then(|| (read.room, read.clone()))
                }).collect::<std::collections::HashMap<_, _>>();
                let mut items = rooms.into_iter().filter(|room| room.pending().members.contains(&me)).map(|mut instance| {
                    let chat = Flow::new(vec![Page::messaging(ctx, &mut instance)]);
                    let room = instance.pending();
                    let members = room.members.iter().copied().filter(|name| *name != me).collect::<Vec<_>>();
                    let name = room.name(ctx);
                    let avatar = if room.is_group() {
                        room.group_details().photo
                    } else {
                        let profile = Profile::from_name(ctx, members.first().copied().unwrap_or(me));
                        profile.pending().avatar.clone()
                    };
                    let (timestamp, last) = match room.messages(me).last() {
                        Some(last) => (last.timestamp, last.preview(ctx, room.is_group())),
                        None => (0, "No messages yet.".into()),
                    };
                    let unread = room.messages(me).any(|message| message.author != me
                        && reads.get(&instance.id()).is_none_or(|read| !read.is_read(message)));
                    (timestamp, ListItem::avatar(avatar, &name, &last, None, Some(chat)).with_badge(unread))
                }).collect::<Vec<_>>();
                items.sort_by_key(|item| std::cmp::Reverse(item.0));
                let items = items.into_iter().map(|(_, item)| item).collect();
                let new_message = Box::new(|ctx: &mut Context, theme: &Theme| {
                    Flow::from_form(NewMessageFlow::build(ctx, theme))
                });
                PageType::root("Messages", vec![], vec![Display::list(None, items,
                    Some("No messages yet.\nGet started by messaging a friend."))], None,
                    Some(("New Message".into(), new_message)), None)
            }))
    }
}
impl Service for MessagesHome {
    type Init = ();
    type Predicate = Instance<ChatRoom>;

    async fn init(air: Air, _: ()) -> Self {
        let rooms = air.instances::<ChatRoom>();
        for room in rooms.list().into_values() {air.start::<RoomNotifications>(room);}
        Self(rooms, air)
    }
    async fn predicate(&mut self) -> Instance<ChatRoom> {self.0.next().await}
    async fn handle(&mut self, room: Instance<ChatRoom>) {self.1.start::<RoomNotifications>(room);}
}

// Watch membership even when the conversation is hidden or this device receives
// a leave/rejoin from another device. Only change the push subscription on a transition.
struct RoomNotifications {room: Instance<ChatRoom>, me: Name, enabled: bool}
impl Service for RoomNotifications {
    type Init = Instance<ChatRoom>;
    type Predicate = bool;
    async fn init(air: Air, room: Self::Init) -> Self {
        let me = air.me();
        let enabled = room.pending().members.contains(&me);
        room.notify(enabled);
        Self {room, me, enabled}
    }
    async fn predicate(&mut self) -> bool {
        self.room.next().await;
        self.room.pending().members.contains(&self.me)
    }
    async fn handle(&mut self, enabled: bool) {
        if enabled != self.enabled {
            self.room.notify(enabled);
            self.enabled = enabled;
        }
    }
}

pub struct NewMessageFlow;
impl NewMessageFlow {
    pub fn build(ctx: &mut Context, theme: &Theme) -> Form {
        let submit = Box::new(|ctx: &mut Context, state: &Vec<State>| {
            let State::Search(recipients) = &state[0] else {unreachable!()};
            let mut room = ChatRoom::create(ctx, recipients.clone());
            room.notify(true);
            FormComplete::Next(Page::messaging(ctx, &mut room))
        }) as Box<dyn FormSubmit>;
        let me = ctx.air.me();
        let items = ctx.air.instances::<Profile>().list().into_values().map(|profile| {
            let profile = profile.pending();
            let username = if profile.name == Some(me) {format!("{} (You)", profile.username)}
                else {profile.username.clone()};
            (ListItem::avatar(profile.avatar.clone(), &username, &profile.name(), None, None),
                profile.name.unwrap())
        }).collect();
        Form::flow(theme, vec![FormItem::search("New message", "profile name", items,
            Some(vec![FormAction::Custom("New contact".into(), None, Icons::Add,
                Action::flow(Flow::from_form(CreateAndAddContact::build(theme))))]))], None, None, submit)
    }
}

struct CreateAndAddContact;
impl CreateAndAddContact {
    fn build(theme: &Theme) -> Form {
        let submit = Box::new(|ctx: &mut Context, state: &Vec<State>| {
            let State::Text(name) = &state[0] else {unreachable!()};
            let name = Name::from_str(name).unwrap();
            Profile::create(ctx, name);
            FormComplete::Return(Box::new(move |ctx: &mut Context, theme: &Theme| {
                (Action::choose_search(name).get())(ctx, theme)
            }))
        }) as Box<dyn FormSubmit>;
        Form::flow(theme, vec![FormItem::text("New contact", "orange name", Some(vec![
            FormAction::Paste, FormAction::scan_qr_code("Scan a profile QR code"),
        ]), |_ctx: &mut Context, name: String| {
            if name.is_empty() {FormValidState::Invalid}
            else if Name::from_str(&name).is_ok() {FormValidState::Valid}
            else {FormValidState::InvalidWithData("Not a valid Orange Name.".into())}
        })], None, None, submit)
    }
}
