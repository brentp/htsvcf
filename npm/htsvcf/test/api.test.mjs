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

  assert.ok(Array.isArray(rec.value.filter));

  reader.close();
});

test("Variant.filter returns empty array for no filter", () => {
  const reader = new Reader(vcfPath);
  const rec = reader.nextSync();
  assert.equal(rec.done, false);
  assert.ok(rec.value);
  assert.deepEqual(rec.value.filter, []);
  reader.close();
});

test("Variant setters (id/qual/filter) mutate record", async () => {
  const fs = await import("node:fs/promises");
  const os = await import("node:os");

  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "htsvcf-setters-"));
  const tmpVcf = path.join(tmp, "t.vcf");

  const vcf = [
    "##fileformat=VCFv4.2",
    '##FILTER=<ID=LowQual,Description="Low quality">',
    "##contig=<ID=chr1>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO",
    "chr1\t1\t.\tA\tC\t.\t.\t.",
  ].join("\n");

  await fs.writeFile(tmpVcf, vcf);

  const reader = new Reader(tmpVcf);
  const rec = reader.nextSync();
  assert.equal(rec.done, false);
  const variant = rec.value;

  variant.id = "rs1";
  assert.equal(variant.id, "rs1");

  variant.qual = 42;
  assert.equal(variant.qual, 42);

  variant.qual = null;
  assert.equal(variant.qual, null);

  // passing PASS resets to empty filters
  variant.filter = ["PASS"];
  assert.deepEqual(variant.filter, ["PASS"]);
  variant.filter = ["LowQual"];
  assert.deepEqual(variant.filter, ["LowQual"]);

  // filter must exist in header
  variant.filter = ["LowQual"];
  assert.deepEqual(variant.filter, ["LowQual"]);

  reader.close();
  await fs.rm(tmp, { recursive: true, force: true });
});

test("Variant.format returns per-sample typed values", async () => {
  const fs = await import("node:fs/promises");
  const os = await import("node:os");

  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "htsvcf-format-"));
  const tmpVcf = path.join(tmp, "t.vcf");

  const vcf = [
    "##fileformat=VCFv4.2",
    '##FORMAT=<ID=DP,Number=1,Type=Integer,Description="Depth">',
    '##FORMAT=<ID=AD,Number=2,Type=Integer,Description="Allele Depths">',
    '##FORMAT=<ID=AF,Number=2,Type=Float,Description="Allele Frequencies">',
    '##FORMAT=<ID=NOTE,Number=1,Type=String,Description="Note">',
    "##contig=<ID=chr1>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2",
    "chr1\t1\t.\tA\tC,G\t.\t.\t.\tDP:AD:AF:NOTE\t7:1,2:0.1,0.2:hi\t.:.,.:.,.:.",
  ].join("\n");

  await fs.writeFile(tmpVcf, vcf);

  const reader = new Reader(tmpVcf);
  const rec = reader.nextSync();
  assert.equal(rec.done, false);
  const variant = rec.value;

  assert.deepEqual(variant.format("DP"), [7, null]);
  assert.deepEqual(variant.format("AD"), [[1, 2], [null, null]]);
  assert.ok(Math.abs(variant.format("AF")[0][0] - 0.1) < 1e-6);
  assert.ok(Math.abs(variant.format("AF")[0][1] - 0.2) < 1e-6);
  assert.deepEqual(variant.format("NOTE"), ["hi", null]);
  assert.equal(variant.format("NOPE"), undefined);

  reader.close();
  await fs.rm(tmp, { recursive: true, force: true });
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
