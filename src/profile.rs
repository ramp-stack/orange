use chk::{Context, Page, Root};
use chk::air::profiles::Profile;

pub struct ProfileHome;
impl ProfileHome {
    pub fn build(ctx: &mut Context) -> Root {
        let mut profile = Profile::me(ctx);
        Root::custom(Page::profile(ctx, &mut profile))
    }
}
