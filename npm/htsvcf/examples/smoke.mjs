import assert from "node:assert/strict";
import { Reader, openReader } from "htsvcf";
import path from "node:path";
import { fileURLToPath } from "node:url";
import os from "node:os";
import fs from "node:fs/promises";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const vcfPath = path.join(__dirname, "..", "..", "..", "tests", "t.vcf.gz");

const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), "htsvcf-smoke-"));

const filterVcfPath = path.join(tmpDir, "filter.vcf");
await fs.writeFile(
  filterVcfPath,
  [
    "##fileformat=VCFv4.2",
    "##FILTER=<ID=LowQual,Description=Low Quality>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO",
    "chr1\t10\t.\tA\tC\t.\tPASS\t.",
    "",
  ].join("\n"),
);

const setInfoVcfPath = path.join(tmpDir, "set_info.vcf");
await fs.writeFile(
  setInfoVcfPath,
  [
    "##fileformat=VCFv4.2",
    '##INFO=<ID=DP,Number=1,Type=Integer,Description="Depth">',
    '##INFO=<ID=NOTE,Number=1,Type=String,Description="Note">',
    '##INFO=<ID=SOMATIC,Number=0,Type=Flag,Description="Somatic">',
    "##contig=<ID=chr1>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO",
    "chr1\t10\t.\tA\tC\t.\tPASS\tDP=10;NOTE=hello",
    "",
  ].join("\n"),
);

const sampleVcfPath = path.join(tmpDir, "sample.vcf");
await fs.writeFile(
  sampleVcfPath,
  [
    "##fileformat=VCFv4.2",
    '##FORMAT=<ID=DP,Number=1,Type=Integer,Description="Read Depth">',
    '##FORMAT=<ID=AD,Number=R,Type=Integer,Description="Allele Depth">',
    "##contig=<ID=chr1>",
    "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2",
    "chr1\t10\t.\tA\tC\t.\tPASS\t.\tDP:AD\t11:8,3\t.:.,.",
    "",
  ].join("\n"),
);

const reader = new Reader(vcfPath);
const filterReader = new Reader(filterVcfPath);
const setInfoReader = new Reader(setInfoVcfPath);
const sampleReader = new Reader(sampleVcfPath);

// Header methods
assert.equal(reader.header, reader.header, "header should have stable identity");

const dp = reader.header.get("INFO", "DP");
assert.ok(dp);
assert.equal(dp.id, "DP");
assert.equal(dp.type, "Integer");
assert.equal(dp.number, "1");
assert.equal(dp.description, "Depth");

assert.equal(reader.header.get("INFO", "NOPE"), undefined);

console.log("header stable identity:", reader.header === reader.header);
console.log("header.get(INFO,DP):", dp);
console.log("header.get(INFO,NOPE):", reader.header.get("INFO", "NOPE"));

const headerText = reader.header.toString();
assert.ok(headerText.includes("##fileformat="));
assert.ok(headerText.includes("##INFO=<ID=DP"));
console.log("header.toString() first 5 lines:\n" + headerText.split("\n").slice(0, 5).join("\n"));

const headerRecords = reader.header.records();
assert.ok(Array.isArray(headerRecords));
assert.ok(headerRecords.length > 0);
assert.equal(typeof headerRecords[0].type, "string");
console.log("header.records() count:", headerRecords.length);
console.log("header.records()[0]:", headerRecords[0]);

reader.header.addInfo("XYZ", "1", "Integer", "Example added INFO field");
reader.header.addFormat("ZZ", "1", "String", "Example added FORMAT field");

const xyz = reader.header.get("INFO", "XYZ");
assert.ok(xyz);
assert.equal(xyz.id, "XYZ");
assert.equal(xyz.type, "Integer");
assert.equal(xyz.number, "1");
assert.equal(xyz.description, "Example added INFO field");

const zz = reader.header.get("FORMAT", "ZZ");
assert.ok(zz);
assert.equal(zz.id, "ZZ");
assert.equal(zz.type, "String");
assert.equal(zz.number, "1");
assert.equal(zz.description, "Example added FORMAT field");

console.log("header.get(INFO,XYZ):", xyz);
console.log("header.get(FORMAT,ZZ):", zz);

let n = 0;
for await (const v of reader) {
  assert.equal(typeof v.chrom, "string");
  assert.ok(v.pos > 0);
  assert.equal(typeof v.ref, "string");
  assert.ok(Array.isArray(v.alt));

  const dpValue = v.info("DP");
  assert.ok(typeof dpValue === "number" || Array.isArray(dpValue));

  console.log(v.chrom, v.pos, v.ref, v.alt.join(","), dpValue);
  if (++n >= 5) break;
}
assert.ok(n > 0);

// Filter setter sanity check (requires FILTER header definition)
const it = filterReader.nextSync();
assert.equal(it.done, false);
assert.ok(it.value);
assert.deepEqual(it.value.filter, ["PASS"]);

it.value.filter = ["LowQual"];
assert.deepEqual(it.value.filter, ["LowQual"]);

// INFO setter sanity check
const it2 = setInfoReader.nextSync();
assert.equal(it2.done, false);
assert.ok(it2.value);
assert.equal(it2.value.info("DP"), 10);
assert.equal(it2.value.info("NOTE"), "hello");

// Variant.sample(name) sanity check
const it3 = sampleReader.nextSync();
assert.equal(it3.done, false);
assert.ok(it3.value);

const s1 = it3.value.sample("S1");
assert.ok(s1);
assert.equal(s1.sample_name, "S1");
assert.equal(s1.DP, 11);
assert.deepEqual(s1.AD, [8, 3]);

const s2 = it3.value.sample("S2");
assert.ok(s2);
assert.equal(s2.sample_name, "S2");
assert.equal(s2.DP, null);
assert.deepEqual(s2.AD, [null, null]);

assert.equal(it3.value.sample("NOPE"), undefined);
console.log("sample(S1):", s1);

it2.value.set_info("DP", 32);
assert.equal(it2.value.info("DP"), 32);

it2.value.set_info("NOTE", "hi");
assert.equal(it2.value.info("NOTE"), "hi");

it2.value.set_info("SOMATIC", true);
assert.equal(it2.value.info("SOMATIC"), true);

it2.value.set_info("DP", null);
assert.equal(it2.value.info("DP"), undefined);

if (reader.hasIndex()) {
  await reader.query("chr1:1000-2000");
  const { value, done } = await reader.next();
  assert.equal(typeof done, "boolean");
  if (!done) {
    assert.ok(value);
    assert.equal(typeof value.toString(), "string");
    console.log("first in region:", value.toString());
  }
}

reader.close();
filterReader.close();
setInfoReader.close();
sampleReader.close();
await fs.rm(tmpDir, { recursive: true, force: true });

const reader2 = await openReader(vcfPath);
assert.equal(reader2.hasIndex(), true);
console.log("openReader worked; hasIndex:", reader2.hasIndex());
reader2.close();
