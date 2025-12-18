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

  const s1 = variant.sample("S1");
  assert.ok(s1);
  assert.equal(s1.sample_name, "S1");
  assert.equal(s1.DP, 7);
  assert.deepEqual(s1.AD, [1, 2]);
  assert.ok(Math.abs(s1.AF[0] - 0.1) < 1e-6);
  assert.ok(Math.abs(s1.AF[1] - 0.2) < 1e-6);
  assert.equal(s1.NOTE, "hi");

  const s2 = variant.sample("S2");
  assert.ok(s2);
  assert.equal(s2.sample_name, "S2");
  assert.equal(s2.DP, null);
  assert.deepEqual(s2.AD, [null, null]);
  assert.deepEqual(s2.AF, [null, null]);
  assert.equal(s2.NOTE, null);

  assert.equal(variant.sample("NOPE"), undefined);

  // Test samples() - returns array of all sample data
  const allSamples = variant.samples();
  assert.ok(Array.isArray(allSamples));
  assert.equal(allSamples.length, 2);

  // First sample matches sample("S1")
  assert.equal(allSamples[0].sample_name, "S1");
  assert.equal(allSamples[0].DP, 7);
  assert.deepEqual(allSamples[0].AD, [1, 2]);
  assert.ok(Math.abs(allSamples[0].AF[0] - 0.1) < 1e-6);
  assert.ok(Math.abs(allSamples[0].AF[1] - 0.2) < 1e-6);
  assert.equal(allSamples[0].NOTE, "hi");

  // Second sample matches sample("S2")
  assert.equal(allSamples[1].sample_name, "S2");
  assert.equal(allSamples[1].DP, null);
  assert.deepEqual(allSamples[1].AD, [null, null]);
  assert.deepEqual(allSamples[1].AF, [null, null]);
  assert.equal(allSamples[1].NOTE, null);

  // Test samples(subset) - returns only specified samples in given order
  const subset1 = variant.samples(["S2"]);
  assert.ok(Array.isArray(subset1));
  assert.equal(subset1.length, 1);
  assert.equal(subset1[0].sample_name, "S2");
  assert.equal(subset1[0].DP, null);

  // Test samples(subset) with reversed order
  const subset2 = variant.samples(["S2", "S1"]);
  assert.equal(subset2.length, 2);
  assert.equal(subset2[0].sample_name, "S2");
  assert.equal(subset2[1].sample_name, "S1");
  assert.equal(subset2[1].DP, 7);

  // Test samples(subset) with unknown sample names (silently skipped)
  const subset3 = variant.samples(["NOPE", "S1", "ALSO_NOPE"]);
  assert.equal(subset3.length, 1);
  assert.equal(subset3[0].sample_name, "S1");

  // Test samples(subset) with all unknown names returns empty array
  const subset4 = variant.samples(["NOPE", "ALSO_NOPE"]);
  assert.equal(subset4.length, 0);

  // Test samples(undefined) returns all samples (same as no argument)
  const subset5 = variant.samples(undefined);
  assert.equal(subset5.length, 2);

  reader.close();
  await fs.rm(tmp, { recursive: true, force: true });
});

test("Variant.samples returns empty array for VCF with no samples", async () => {
  const fs = await import("node:fs/promises");
  const os = await import("node:os");

  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "htsvcf-no-samples-"));
  const tmpVcf = path.join(tmp, "t.vcf");

  const vcf = [
    "##fileformat=VCFv4.2",
    "##contig=<ID=chr1>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO",
    "chr1\t1\t.\tA\tC\t.\t.\t.",
  ].join("\n");

  await fs.writeFile(tmpVcf, vcf);

  const reader = new Reader(tmpVcf);
  const rec = reader.nextSync();
  assert.equal(rec.done, false);
  const variant = rec.value;

  const allSamples = variant.samples();
  assert.ok(Array.isArray(allSamples));
  assert.equal(allSamples.length, 0);

  reader.close();
  await fs.rm(tmp, { recursive: true, force: true });
});

test("Variant.set_info mutates INFO fields", async () => {
  const fs = await import("node:fs/promises");
  const os = await import("node:os");

  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "htsvcf-set-info-"));
  const tmpVcf = path.join(tmp, "t.vcf");

  const vcf = [
    "##fileformat=VCFv4.2",
    '##INFO=<ID=DP,Number=1,Type=Integer,Description="Depth">',
    '##INFO=<ID=AD,Number=2,Type=Integer,Description="Allele Depths">',
    '##INFO=<ID=AF,Number=2,Type=Float,Description="Allele Frequencies">',
    '##INFO=<ID=NOTE,Number=1,Type=String,Description="Note">',
    '##INFO=<ID=SOMATIC,Number=0,Type=Flag,Description="Somatic">',
    "##contig=<ID=chr1>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO",
    "chr1\t1\t.\tA\tC,G\t.\t.\t.",
  ].join("\n");

  await fs.writeFile(tmpVcf, vcf);

  const reader = new Reader(tmpVcf);
  const rec = reader.nextSync();
  assert.equal(rec.done, false);
  const variant = rec.value;

  variant.set_info("DP", 32);
  assert.equal(variant.info("DP"), 32);

  variant.set_info("AD", [1, 2]);
  assert.deepEqual(variant.info("AD"), [1, 2]);

  variant.set_info("AF", [0.1, 0.2]);
  assert.ok(Math.abs(variant.info("AF")[0] - 0.1) < 1e-6);
  assert.ok(Math.abs(variant.info("AF")[1] - 0.2) < 1e-6);

  variant.set_info("NOTE", "hi");
  assert.equal(variant.info("NOTE"), "hi");

  variant.set_info("SOMATIC", true);
  assert.equal(variant.info("SOMATIC"), true);

  variant.set_info("DP", null);
  assert.equal(variant.info("DP"), undefined);

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

test("Header.samples returns array of sample names", async () => {
  const fs = await import("node:fs/promises");
  const os = await import("node:os");

  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "htsvcf-header-samples-"));
  const tmpVcf = path.join(tmp, "t.vcf");

  const vcf = [
    "##fileformat=VCFv4.2",
    '##FORMAT=<ID=GT,Number=1,Type=String,Description="Genotype">',
    "##contig=<ID=chr1>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\tS3",
    "chr1\t1\t.\tA\tC\t.\t.\t.\tGT\t0/1\t0/0\t1/1",
  ].join("\n");

  await fs.writeFile(tmpVcf, vcf);

  const reader = new Reader(tmpVcf);

  const samples = reader.header.samples();
  assert.ok(Array.isArray(samples));
  assert.equal(samples.length, 3);
  assert.equal(samples[0], "S1");
  assert.equal(samples[1], "S2");
  assert.equal(samples[2], "S3");
  assert.deepEqual(samples, ["S1", "S2", "S3"]);

  reader.close();
  await fs.rm(tmp, { recursive: true, force: true });
});

test("Header.samples returns empty array for VCF without samples", async () => {
  const fs = await import("node:fs/promises");
  const os = await import("node:os");

  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "htsvcf-header-no-samples-"));
  const tmpVcf = path.join(tmp, "t.vcf");

  const vcf = [
    "##fileformat=VCFv4.2",
    "##contig=<ID=chr1>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO",
    "chr1\t1\t.\tA\tC\t.\t.\t.",
  ].join("\n");

  await fs.writeFile(tmpVcf, vcf);

  const reader = new Reader(tmpVcf);

  const samples = reader.header.samples();
  assert.ok(Array.isArray(samples));
  assert.equal(samples.length, 0);

  reader.close();
  await fs.rm(tmp, { recursive: true, force: true });
});
