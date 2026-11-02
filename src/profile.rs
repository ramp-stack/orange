use chk::{Context, Root};

pub struct ProfileHome;
impl ProfileHome {
    pub fn build(ctx: &mut Context, invitations: &chk::Invitations<ramp::prism::air::Name>) -> Root {
        Root::custom(invitations.my_profile(ctx))
    }
}
