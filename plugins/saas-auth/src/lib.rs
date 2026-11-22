//! `saas-auth`: a composer. It has no code; the host expands it into
//! auth-core, auth-oauth, auth-mfa and rbac (see `plugin.yaml`).

use keel_sdk::composer::Composer;

pub struct SaasAuth;

impl SaasAuth {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Composer {
        keel_sdk::composer!()
    }
}
