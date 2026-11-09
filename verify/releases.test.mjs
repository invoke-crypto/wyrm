import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { b58encode } from "../keeper/chain.mjs";

test("releases.json pins the preset hashes wyrm-types pins, in base58", () => {
  const rust = fs.readFileSync(new URL("../contracts/wyrm-types/src/preset.rs", import.meta.url), "utf8");
  const names = Object.fromEntries([...rust.matchAll(/PresetName::(\w+) => "([a-z-]+)"/g)].map(([, variant, name]) => [variant, name]));
  const pinned = Object.fromEntries([...rust.matchAll(/PresetName::(\w+),\s*"([0-9a-f]{64})"/g)].map(([, variant, hex]) => [names[variant], b58encode(Buffer.from(hex, "hex"))]));
  assert.deepEqual(Object.keys(pinned), ["hoarder", "steady", "dip-hunter", "swift", "harvester", "drill"]);
  const { presets } = JSON.parse(fs.readFileSync(new URL("./releases.json", import.meta.url), "utf8")).source;
  assert.deepEqual(presets, pinned);
});
