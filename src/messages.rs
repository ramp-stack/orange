use chk::{
    Action, Bumper, Context, Display, Flow, Form, FormAction,
    FormComplete, FormItem, FormSubmit, FormValidState, Icons, ListItem, Offset, Page, PageType, Root,
    Selector, State, Theme, Watch
};
use chk::air::{messages::{ChatRoom, RoomReadState}, profiles::Profile};
use ramp::prism::air::{Air, Instance, Instances, Name};
use ramp::prism::service::Service;

use chk::Invitations;

type MessageLists = (Vec<Instance<ChatRoom>>, Vec<Instance<Profile>>, Vec<Instance<RoomReadState>>);

pub struct MessagesHome(Instances<ChatRoom>, Air);
impl MessagesHome {
    pub fn build(invitations: Invitations<Name>) -> Root {
        Root::custom(Page::watching((Watch::list::<ChatRoom>(), Watch::list::<Profile>(), Watch::list::<RoomReadState>()),
            move |ctx: &mut Context, _theme: &Theme, (rooms, _, reads): MessageLists| {
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
                let message_invitations = invitations.clone();
                let new_message = Box::new(move |ctx: &mut Context, theme: &Theme| {
                    Flow::from_form(NewMessageFlow::build(ctx, theme, message_invitations.clone()))
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
    pub fn build(ctx: &mut Context, theme: &Theme, invitations: Invitations<Name>) -> Form {
        let submit = Box::new(|ctx: &mut Context, state: &Vec<State>| {
            let Some(State::Selection(keys)) = state.first() else {
                return FormComplete::None;
            };
            let recipients = match selected_recipients(keys) {
                Ok(recipients) => recipients,
                Err(error) => {
                    return FormComplete::Next(Page::Static(Box::new(PageType::display(
                        "New message",
                        vec![Display::instructions(error)],
                        None,
                        Bumper::Done,
                        Offset::Center,
                    ))));
                }
            };
            let mut room = ChatRoom::create(ctx, recipients);
            room.notify(true);
            FormComplete::Next(Page::messaging(ctx, &mut room))
        }) as Box<dyn FormSubmit>;
        let me = ctx.air.me();
        let items = ctx
            .air
            .instances::<Profile>()
            .list()
            .into_values()
            .filter_map(|profile| profile_row(&profile.pending(), me))
            .collect();
        let selector = Selector::new("profile name", items);
        let add_contact = CreateAndAddContact::build(theme, invitations, selector.clone());
        let selector = selector.with_actions(vec![FormAction::Custom(
            "New contact".into(),
            None,
            Icons::Add,
            Action::flow(Flow::from_form(add_contact)),
        )]);
        Form::flow(
            theme,
            vec![FormItem::selection("New message", selector)],
            None,
            None,
            submit,
        )
    }
}

/// Keys belong to the generic UI; parse the entire selection before creating a room.
/// Invalid keys must never silently turn an intended group into a smaller audience.
fn selected_recipients(keys: &[String]) -> Result<Vec<Name>, &'static str> {
    if keys.is_empty() {
        return Err("Choose at least one contact.");
    }
    let mut recipients = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for key in keys {
        let name = key
            .parse::<Name>()
            .map_err(|_| "One of the selected contacts is invalid. Choose your contacts again.")?;
        if seen.insert(name) {
            recipients.push(name);
        }
    }
    Ok(recipients)
}

fn profile_row(profile: &Profile, me: Name) -> Option<(ListItem, String)> {
    let name = profile.name?;
    let username = if name == me {
        format!("{} (You)", profile.username)
    } else {
        profile.username.clone()
    };
    Some((
        ListItem::avatar(
            profile.avatar.clone(),
            &username,
            &profile.name(),
            None,
            None,
        ),
        name.to_string(),
    ))
}

struct CreateAndAddContact;
impl CreateAndAddContact {
    fn build(theme: &Theme, invitations: Invitations<Name>, selector: Selector) -> Form {
        let submit_invitations = invitations.clone();
        let submit = Box::new(move |ctx: &mut Context, state: &Vec<State>| {
            let Some(State::Text(name)) = state.first() else {
                return FormComplete::None;
            };
            let Some(name) = submit_invitations.parse_contact(name) else {
                return FormComplete::None;
            };
            let profile = Profile::from_name(ctx, name);
            let Some((row, key)) = profile_row(&profile.pending(), ctx.air.me()) else {
                return FormComplete::None;
            };
            let select = selector.select(row, key);
            FormComplete::Return(Box::new(move |ctx: &mut Context, theme: &Theme| {
                (select.get())(ctx, theme)
            }))
        }) as Box<dyn FormSubmit>;
        Form::flow(
            theme,
            vec![FormItem::text(
                "New contact",
                "orange name",
                Some(vec![
                    FormAction::Paste,
                    FormAction::scan_qr_code("Scan a profile QR code"),
                ]),
                move |_ctx: &mut Context, name: String| {
                    if name.is_empty() {
                        FormValidState::Invalid
                    } else if invitations.parse_contact(&name).is_some() {
                        FormValidState::Valid
                    } else {
                        FormValidState::InvalidWithData(
                            "Not a valid Orange name or contact link.".into(),
                        )
                    }
                },
            )],
            None,
            None,
            submit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact_names() -> (Name, Name) {
        (
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                .parse()
                .unwrap(),
            "0379be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                .parse()
                .unwrap(),
        )
    }

    #[test]
    fn selection_keys_round_trip_and_deduplicate_in_selection_order() {
        let (first, second) = contact_names();
        assert_eq!(
            selected_recipients(&[second.to_string(), first.to_string(), second.to_string()]),
            Ok(vec![second, first])
        );
    }

    #[test]
    fn invalid_or_empty_selection_never_becomes_a_partial_recipient_list() {
        let (first, _) = contact_names();
        assert!(selected_recipients(&[]).is_err());
        assert!(selected_recipients(&[first.to_string(), "invalid contact".into()]).is_err());
        assert!(selected_recipients(&[String::new(), first.to_string()]).is_err());
    }
}
