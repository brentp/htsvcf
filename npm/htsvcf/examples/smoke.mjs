import { Reader, openReader } from "htsvcf";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const vcfPath = path.join(__dirname, "..", "..", "..", "tests", "t.vcf.gz");

const reader = new Reader(vcfPath);

// Header methods
console.log("header stable identity:", reader.header === reader.header);
console.log("header.get(INFO,DP):", reader.header.get("INFO", "DP"));
console.log("header.get(INFO,NOPE):", reader.header.get("INFO", "NOPE"));

const headerText = reader.header.toString();
console.log("header.toString() first 5 lines:\n" + headerText.split("\n").slice(0, 5).join("\n"));

const headerRecords = reader.header.records();
console.log("header.records() count:", headerRecords.length);
console.log("header.records()[0]:", headerRecords[0]);

reader.header.addInfo("XYZ", "1", "Integer", "Example added INFO field");
reader.header.addFormat("ZZ", "1", "String", "Example added FORMAT field");
console.log("header.get(INFO,XYZ):", reader.header.get("INFO", "XYZ"));
console.log("header.get(FORMAT,ZZ):", reader.header.get("FORMAT", "ZZ"));

let n = 0;
for await (const v of reader) {
  console.log(v.chrom, v.pos, v.ref, v.alt.join(","), v.info("DP"));
  if (++n >= 5) break;
}

if (reader.hasIndex()) {
  await reader.query("chr1:1000-2000");
  const { value, done } = await reader.next();
  if (!done) console.log("first in region:", value.toString());
}

reader.close();

const reader2 = await openReader(vcfPath);
console.log("openReader worked; hasIndex:", reader2.hasIndex());
reader2.close();
