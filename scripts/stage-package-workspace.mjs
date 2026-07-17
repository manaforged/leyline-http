import fs from "node:fs";
import path from "node:path";

const [stage] = process.argv.slice(2);
if (!stage) {
  throw new Error("usage: stage-package-workspace.mjs <staged-repository>");
}

const manifest = (relative) => path.join(stage, relative, "Cargo.toml");
const rootManifest = manifest("");
let root = fs.readFileSync(rootManifest, "utf8");

const memberAnchor = '    "crates/leyline",\n';
const stagedMembers = [
  "crates/leyline-bssl-sys",
  "crates/leyline-bssl",
  "crates/leyline-bssl-tokio",
]
  .map((member) => `    "${member}",\n`)
  .join("");
if (!root.includes(memberAnchor)) {
  throw new Error("workspace member anchor is missing");
}
root = root.replace(memberAnchor, memberAnchor + stagedMembers);

const excluded = `\nexclude = [
    "crates/leyline-bssl-sys",
    "crates/leyline-bssl",
    "crates/leyline-bssl-tokio",
]
`;
if (!root.includes(excluded)) {
  throw new Error("BoringSSL workspace exclusion block is missing");
}
fs.writeFileSync(rootManifest, root.replace(excluded, "\n"));

for (const member of [
  "crates/leyline-bssl-sys",
  "crates/leyline-bssl",
  "crates/leyline-bssl-tokio",
]) {
  const file = manifest(member);
  const source = fs.readFileSync(file, "utf8");
  const staged = source.replace(/\n\[workspace\]\s*$/, "\n");
  if (staged === source) {
    throw new Error(`${member} does not declare its standalone workspace`);
  }
  fs.writeFileSync(file, staged);
}
