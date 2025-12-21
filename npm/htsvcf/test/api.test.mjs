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

test("Reader supports sync for...of iteration", () => {
  const reader = new Reader(vcfPath);

  let n = 0;
  for (const v of reader) {
    n += 1;
    assert.equal(typeof v.chrom, "string");
  }

  assert.ok(n > 0);
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

test("Variant.genotypes returns parsed genotype objects", () => {
  const genotypesVcf = path.join(__dirname, "..", "..", "..", "tests", "genotypes.vcf");
  const reader = new Reader(genotypesVcf);

  // First record: chr1 100 with genotypes 0/1, 1|1, ./1, 1, 0/1|2
  const rec1 = reader.nextSync();
  assert.equal(rec1.done, false);
  const v1 = rec1.value;

  const gts1 = v1.genotypes();
  assert.ok(Array.isArray(gts1));
  assert.equal(gts1.length, 5); // 5 samples

  // diploid_unphased: 0/1
  assert.deepEqual(gts1[0].alleles, [0, 1]);
  assert.deepEqual(gts1[0].phase, [false]);

  // diploid_phased: 1|1
  assert.deepEqual(gts1[1].alleles, [1, 1]);
  assert.deepEqual(gts1[1].phase, [true]);

  // diploid_missing: ./1
  assert.deepEqual(gts1[2].alleles, [null, 1]);
  assert.deepEqual(gts1[2].phase, [false]);

  // haploid: 1
  assert.deepEqual(gts1[3].alleles, [1]);
  assert.deepEqual(gts1[3].phase, []);

  // triploid: 0/1|2
  assert.deepEqual(gts1[4].alleles, [0, 1, 2]);
  assert.deepEqual(gts1[4].phase, [false, true]);

  // Second record: chr1 200 with genotypes 0/0, 0|1, .|., 0, 1|1|1
  const rec2 = reader.nextSync();
  assert.equal(rec2.done, false);
  const v2 = rec2.value;

  const gts2 = v2.genotypes();
  assert.equal(gts2.length, 5);

  // diploid_unphased: 0/0
  assert.deepEqual(gts2[0].alleles, [0, 0]);
  assert.deepEqual(gts2[0].phase, [false]);

  // diploid_phased: 0|1
  assert.deepEqual(gts2[1].alleles, [0, 1]);
  assert.deepEqual(gts2[1].phase, [true]);

  // diploid_missing: .|.
  assert.deepEqual(gts2[2].alleles, [null, null]);
  assert.deepEqual(gts2[2].phase, [true]);

  // haploid: 0
  assert.deepEqual(gts2[3].alleles, [0]);
  assert.deepEqual(gts2[3].phase, []);

  // triploid: 1|1|1
  assert.deepEqual(gts2[4].alleles, [1, 1, 1]);
  assert.deepEqual(gts2[4].phase, [true, true]);

  reader.close();
});

test("Variant.genotypes with subset returns only specified samples", () => {
  const genotypesVcf = path.join(__dirname, "..", "..", "..", "tests", "genotypes.vcf");
  const reader = new Reader(genotypesVcf);

  const rec = reader.nextSync();
  const v = rec.value;

  // Get only haploid and triploid samples
  const subset = v.genotypes(["haploid", "triploid"]);
  assert.equal(subset.length, 2);

  // haploid: 1
  assert.deepEqual(subset[0].alleles, [1]);
  assert.deepEqual(subset[0].phase, []);

  // triploid: 0/1|2
  assert.deepEqual(subset[1].alleles, [0, 1, 2]);
  assert.deepEqual(subset[1].phase, [false, true]);

  // Reversed order
  const reversed = v.genotypes(["triploid", "haploid"]);
  assert.equal(reversed.length, 2);
  assert.deepEqual(reversed[0].alleles, [0, 1, 2]); // triploid first
  assert.deepEqual(reversed[1].alleles, [1]); // haploid second

  // Unknown samples are skipped
  const withUnknown = v.genotypes(["NOPE", "haploid", "ALSO_NOPE"]);
  assert.equal(withUnknown.length, 1);
  assert.deepEqual(withUnknown[0].alleles, [1]);

  reader.close();
});

test("Variant.sample includes parsed genotype", () => {
  const genotypesVcf = path.join(__dirname, "..", "..", "..", "tests", "genotypes.vcf");
  const reader = new Reader(genotypesVcf);

  const rec = reader.nextSync();
  const v = rec.value;

  const s = v.sample("diploid_phased");
  assert.ok(s);
  assert.equal(s.sample_name, "diploid_phased");
  // Note: s.GT contains htslib's raw encoded value, not a human-readable string.
  // Use s.genotype for parsed alleles/phase instead.
  assert.ok(s.genotype);
  assert.deepEqual(s.genotype.alleles, [1, 1]);
  assert.deepEqual(s.genotype.phase, [true]);

  reader.close();
});

test("Variant.samples includes parsed genotype for each sample", () => {
  const genotypesVcf = path.join(__dirname, "..", "..", "..", "tests", "genotypes.vcf");
  const reader = new Reader(genotypesVcf);

  const rec = reader.nextSync();
  const v = rec.value;

  const samples = v.samples();
  assert.equal(samples.length, 5);

  // Check that each sample has a genotype property
  for (const s of samples) {
    assert.ok(s.genotype, `Sample ${s.sample_name} should have genotype`);
    assert.ok(Array.isArray(s.genotype.alleles));
    assert.ok(Array.isArray(s.genotype.phase));
  }

  // Verify specific samples
  const haploid = samples.find(s => s.sample_name === "haploid");
  assert.deepEqual(haploid.genotype.alleles, [1]);
  assert.deepEqual(haploid.genotype.phase, []);

  reader.close();
});

test("Header.records returns section and type correctly", async () => {
  const fs = await import("node:fs/promises");
  const os = await import("node:os");

  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "htsvcf-records-"));
  const tmpVcf = path.join(tmp, "t.vcf");

  const vcf = [
    "##fileformat=VCFv4.2",
    '##INFO=<ID=DP,Number=1,Type=Integer,Description="Depth">',
    '##FORMAT=<ID=GT,Number=1,Type=String,Description="Genotype">',
    "##contig=<ID=chr1>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1",
    "chr1\t1\t.\tA\tC\t.\t.\tDP=7\tGT\t0/1",
  ].join("\n");

  await fs.writeFile(tmpVcf, vcf);

  const reader = new Reader(tmpVcf);
  const records = reader.header.records();

  // Check INFO/DP record has correct shape (section vs type distinction)
  const dpRecord = records.find(r => r.id === "DP");
  assert.deepEqual(dpRecord, {
    section: "INFO",
    id: "DP",
    number: "1",
    type: "Integer",
    description: "Depth",
  });

  // Check FORMAT/GT record
  const gtRecord = records.find(r => r.id === "GT");
  assert.deepEqual(gtRecord, {
    section: "FORMAT",
    id: "GT",
    number: "1",
    type: "String",
    description: "Genotype",
  });

  reader.close();
  await fs.rm(tmp, { recursive: true, force: true });
});
