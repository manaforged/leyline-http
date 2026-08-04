#!/usr/bin/env node
// Leyline deviation: crates/leyline-bssl* are excluded from the cargo
// workspace (vendored BoringSSL stack, path-deped at pinned versions), so the
// reverse-dependency closure can never select them; any change under those
// dirs triggers the full workspace instead. See `bsslPrefixes` below.
//
// Resolve changed files to affected workspace packages, including every
// workspace reverse-dependency. Output is deterministic and one package/line.

import { execFileSync } from "node:child_process";
import path from "node:path";

const args = process.argv.slice(2);
let base = null;
let head = "HEAD";
let explain = false;
let filesOverride = null;
for (let i = 0; i < args.length; i += 1) {
  if (args[i] === "--base") base = args[++i];
  else if (args[i] === "--head") head = args[++i];
  else if (args[i] === "--files") filesOverride = args[++i].split(",").filter(Boolean);
  else if (args[i] === "--explain") explain = true;
  else {
    process.stderr.write(`affected-packages: unknown argument ${args[i]}\n`);
    process.exit(2);
  }
}

const root = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--format-version", "1"], {
  cwd: root,
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
}));
const workspaceIds = new Set(metadata.workspace_members);
const packages = metadata.packages.filter((pkg) => workspaceIds.has(pkg.id));
const byId = new Map(packages.map((pkg) => [pkg.id, pkg]));
const roots = packages
  .map((pkg) => ({
    id: pkg.id,
    name: pkg.name,
    dir: path.relative(root, path.dirname(pkg.manifest_path)).replaceAll(path.sep, "/"),
  }))
  .sort((a, b) => b.dir.length - a.dir.length || a.name.localeCompare(b.name));

let full = filesOverride === null && (!base || /^0+$/.test(base));
let changed = filesOverride ?? [];
if (!full && filesOverride === null) {
  try {
    changed = execFileSync("git", ["diff", "--name-only", "--diff-filter=ACMRD", base, head, "--"], {
      cwd: root,
      encoding: "utf8",
    }).trim().split("\n").filter(Boolean);
  } catch {
    process.stderr.write(`affected-packages: cannot diff ${base}..${head}; selecting full workspace\n`);
    full = true;
  }
}

const globalRustInputs = new Set([
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "rust-toolchain",
  "deny.toml",
  "LEYLINE_REV",
]);
if (changed.some((file) => globalRustInputs.has(file) || file.startsWith(".cargo/"))) full = true;

// Leyline-only: the owned BoringSSL crates are workspace-excluded, so no
// closure can cover them — a change there rebuilds and retests everything.
const bsslPrefixes = ["crates/leyline-bssl"];
if (changed.some((file) => bsslPrefixes.some((prefix) => file.startsWith(prefix)))) full = true;

const direct = new Set();
if (!full) {
  for (const file of changed) {
    // Any non-markdown file under a package dir can change build output —
    // compiled-in assets (include_str! data files, embedded JSON) are the
    // blind spot of a .rs-only filter.
    if (/\.md$/.test(file)) continue;
    const owner = roots.find((pkg) => file === pkg.dir || file.startsWith(`${pkg.dir}/`));
    if (owner) direct.add(owner.id);
  }
}

const selected = full ? new Set(workspaceIds) : new Set(direct);
if (!full && metadata.resolve?.nodes) {
  const reverse = new Map();
  for (const node of metadata.resolve.nodes) {
    if (!workspaceIds.has(node.id)) continue;
    for (const dep of node.deps ?? []) {
      if (!workspaceIds.has(dep.pkg)) continue;
      if (!reverse.has(dep.pkg)) reverse.set(dep.pkg, new Set());
      reverse.get(dep.pkg).add(node.id);
    }
  }
  const queue = [...selected];
  while (queue.length) {
    const id = queue.shift();
    for (const dependent of reverse.get(id) ?? []) {
      if (!selected.has(dependent)) {
        selected.add(dependent);
        queue.push(dependent);
      }
    }
  }
}

const names = [...selected].map((id) => byId.get(id)?.name).filter(Boolean).sort();
if (explain) {
  process.stderr.write(`affected-packages: ${full ? "full workspace" : `${direct.size} direct -> ${names.length} with reverse dependencies`}\n`);
  if (changed.length) process.stderr.write(`changed files: ${changed.length}\n`);
  process.stderr.write(`packages: ${names.length ? names.join(", ") : "none (non-Rust change)"}\n`);
}
process.stdout.write(names.length ? `${names.join("\n")}\n` : "");
