use std::sync::LazyLock;

use super::types::BrowserProfile;

const SOURCE: &str = include_str!("../../profiles/bare.toml");
const VERSION_TOKEN: &str = "{crate_version}";

static BARE: LazyLock<BrowserProfile> = LazyLock::new(BrowserProfile::bare);

impl BrowserProfile {
    bench_pub! {
        fn bare() -> Self {
            let source = SOURCE.replace(VERSION_TOKEN, env!("CARGO_PKG_VERSION"));
            Self::from_toml(&source)
                .expect("bundled bare profile is statically valid (profile_validation)")
        }
    }

    pub(crate) fn bare_static() -> &'static Self {
        &BARE
    }
}
