import test from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { openReader, Reader, Writer } from "../index.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const vcfPath = path.join(__dirname, "..", "..", "..", "tests", "t.vcf.gz");

test("Writer writes mutated records (sync read)", async () => {
  const fs = await import("node:fs/promises");
  const os = await import("node:os");

  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), "htsvcf-writer-"));
  const outVcf = path.join(tmp, "out.vcf");

  const reader = new Reader(vcfPath);
  reader.header.addInfo("ZZ", "1", "Integer", "Zed");

  const writer = new Writer(outVcf, reader.header);

  let n = 0;
  for (const v of reader) {
    v.translate(reader.header);
    v.set_info("ZZ", 42);
    writer.write(v);
    n += 1;
    if (n >= 3) break;
  }
  writer.close();
  reader.close();

  const outReader = await openReader(outVcf);
  let m = 0;
  while (true) {
    const { done, value } = await outReader.next();
    if (done) break;
    assert.equal(value.info("ZZ"), 42);
    m += 1;
  }
  outReader.close();

  assert.equal(m, 3);

  await fs.rm(tmp, { recursive: true, force: true });
});

test("Writer supports stdout via path '-'", () => {
  const reader = new Reader(vcfPath);
  const writer = new Writer("-", reader.header);

  const rec = reader.nextSync();
  assert.equal(rec.done, false);
  rec.value.translate(reader.header);
  writer.write(rec.value);

  writer.close();
  reader.close();
});

test("Writer.write after close throws", () => {
  const reader = new Reader(vcfPath);
  const writer = new Writer("-", reader.header);
  writer.close();

  const rec = reader.nextSync();
  assert.equal(rec.done, false);

  assert.throws(() => writer.write(rec.value));
  reader.close();
});

test("Writer.header returns the header", () => {
  const reader = new Reader(vcfPath);
  reader.header.addInfo("ZZ", "1", "Integer", "Zed");
  const writer = new Writer("-", reader.header);

  const writerHeader = writer.header;
  assert.ok(writerHeader);
  assert.deepEqual(writerHeader.samples(), reader.header.samples());

  // Check that the added INFO field is present
  const zzInfo = writerHeader.get("INFO", "ZZ");
  assert.ok(zzInfo);
  assert.equal(zzInfo.id, "ZZ");

  writer.close();
  reader.close();
});
