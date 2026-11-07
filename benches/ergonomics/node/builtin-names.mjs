import { writeFileSync } from "node:fs";

const names = new Set();
for (const proto of [
  Array.prototype,
  String.prototype,
  Number.prototype,
  Object.prototype,
  Promise.prototype,
  Map.prototype,
  Set.prototype,
  Function.prototype,
  Buffer.prototype,
  Array,
  Object,
  Promise,
  JSON,
  Number,
  Buffer,
  console,
  process,
]) {
  for (const name of Object.getOwnPropertyNames(proto)) names.add(name);
}
writeFileSync("builtin-names.json", JSON.stringify([...names].sort()));
