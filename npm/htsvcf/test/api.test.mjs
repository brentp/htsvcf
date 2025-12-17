import test from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { Reader, openReader } from "../index.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const vcfPath = path.join(__dirname, "..", "..", "..", "tests", "t.vcf.gz");

test("Header.get includes Description and undefined for missing", () => {
  const reader = new Reader(vcfPath);

  const dp = reader.header.get("INFO", "DP");
  assert.ok(dp);
  assert.equal(dp.id, "DP");
  assert.equal(dp.type, "Integer");
  assert.equal(dp.number, "1");
  assert.equal(dp.description, "Depth");

  const nope = reader.header.get("INFO", "NOPE");
  assert.equal(nope, undefined);

  reader.close();
});

test("openReader returns usable reader", async () => {
  const reader = await openReader(vcfPath);
  assert.equal(reader.hasIndex(), true);

  const dp = reader.header.get("INFO", "DP");
  assert.ok(dp);
  assert.equal(dp.description, "Depth");

  reader.close();
});

test("nextSync returns IteratorResult with Variant", () => {
  const reader = new Reader(vcfPath);

  const rec = reader.nextSync();
  assert.equal(typeof rec.done, "boolean");
  assert.equal(rec.done, false);
  assert.ok(rec.value);
  assert.equal(typeof rec.value.chrom, "string");
  assert.ok(rec.value.pos > 0);

  reader.close();
});

test("nextSync returns done=true at EOF", () => {
  const reader = new Reader(vcfPath);

  for (;;) {
    const rec = reader.nextSync();
    assert.equal(typeof rec.done, "boolean");
    if (rec.done) break;
    assert.ok(rec.value);
  }

  const rec2 = reader.nextSync();
  assert.equal(rec2.done, true);
  assert.equal(rec2.value, undefined);

  reader.close();
});

test("nextSync throws after close", () => {
  const reader = new Reader(vcfPath);
  reader.close();

  assert.throws(() => reader.nextSync(), /reader is closed/);
});
