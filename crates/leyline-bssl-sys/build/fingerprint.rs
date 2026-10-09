use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::Path;

const STAMP: &str = "inputs.stamp";
const HASHED_EXTENSIONS: [&str; 7] = ["h", "inc", "cc", "S", "asm", "json", "obj"];
const SKIPPED_DIRS: [&str; 1] = [".git"];

fn hash_tree(dir: &Path, root: &Path, hasher: &mut DefaultHasher) -> io::Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if !SKIPPED_DIRS.iter().any(|skip| entry.file_name() == *skip) {
                hash_tree(&path, root, hasher)?;
            }
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| HASHED_EXTENSIONS.contains(&ext))
        {
            path.strip_prefix(root).unwrap_or(&path).hash(hasher);
            fs::read(&path)?.hash(hasher);
        }
    }
    Ok(())
}

pub(crate) fn inputs(trees: &[&Path], compiler: &cc::Tool, extra: &[String]) -> io::Result<String> {
    let mut hasher = DefaultHasher::new();
    for tree in trees.iter().filter(|tree| tree.is_dir()) {
        hash_tree(tree, tree, &mut hasher)?;
    }
    compiler.path().hash(&mut hasher);
    compiler.args().hash(&mut hasher);
    extra.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

pub(crate) fn is_current(out: &Path, stamp: &str, outputs: &[String]) -> bool {
    fs::read_to_string(out.join(STAMP)).is_ok_and(|saved| saved == stamp)
        && outputs.iter().all(|file| out.join(file).is_file())
}

pub(crate) fn record(out: &Path, stamp: &str) -> io::Result<()> {
    fs::write(out.join(STAMP), stamp)
}
