use bindgen::callbacks::{ItemInfo, ParseCallbacks};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

pub const PREFIX: &str = "LEYLINE";

#[derive(Debug)]
pub struct PrefixCallback {
    syms: HashSet<String>,
    label: &'static str,
}

impl PrefixCallback {
    pub fn read(include: &Path, target_os: &str) -> io::Result<Self> {
        let text = fs::read_to_string(include.join("openssl").join("prefix_symbols.h"))?;
        let syms: HashSet<String> = text
            .lines()
            .filter_map(|line| line.strip_prefix("#pragma redefine_extname "))
            .filter_map(|rest| rest.split_whitespace().next())
            .map(str::to_owned)
            .collect();

        if syms.is_empty() {
            return Err(io::Error::other(
                "openssl/prefix_symbols.h listed no renamed symbols",
            ));
        }

        let label = match target_os {
            "macos" | "ios" | "tvos" | "watchos" | "visionos" => "_",
            _ => "",
        };

        Ok(Self { syms, label })
    }
}

impl ParseCallbacks for PrefixCallback {
    fn generated_link_name_override(&self, item: ItemInfo<'_>) -> Option<String> {
        self.syms
            .contains(item.name)
            .then(|| format!("{}{PREFIX}_{}", self.label, item.name))
    }
}
