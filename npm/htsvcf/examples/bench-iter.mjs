#!/usr/bin/env node
/**
 * Benchmark: Synchronous vs Asynchronous iteration in htsvcf
 *
 * Usage: node examples/bench-iter.mjs [iterations]
 *
 * Creates a temporary VCF with 100,000 variants for benchmarking.
 */

import { Reader } from "../index.mjs";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const iterations = parseInt(process.argv[2] || "5", 10);
const variantCount = 10_000;

// Create temporary VCF file
const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "htsvcf-bench-"));
const vcfPath = path.join(tmpDir, "bench.vcf");

console.log("Generating temporary VCF with 100,000 variants...");

const header = [
  "##fileformat=VCFv4.2",
  '##INFO=<ID=DP,Number=1,Type=Integer,Description="Depth">',
  "##contig=<ID=chr1>",
  "#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO",
].join("\n");

const lines = [header];
for (let i = 1; i <= variantCount; i++) {
  lines.push(`chr1\t${i}\t.\tA\tC\t30\t.\tDP=${i}`);
}
fs.writeFileSync(vcfPath, lines.join("\n") + "\n");

console.log(`Created: ${vcfPath}`);
console.log(`Variants: ${variantCount}`);
console.log(`Iterations: ${iterations}\n`);

// Benchmark synchronous iteration with nextSync()
async function benchSync() {
  const times = [];

  for (let i = 0; i < iterations; i++) {
    const reader = new Reader(vcfPath);
    let count = 0;
    let posSum = 0;

    const start = performance.now();

    let result;
    while (!(result = reader.nextSync()).done) {
      const variant = result.value;
      posSum += variant.pos;
      count++;
    }

    const elapsed = performance.now() - start;
    times.push(elapsed);

    reader.close();

    if (count !== variantCount) {
      throw new Error(`Sync count mismatch: ${count} vs ${variantCount}`);
    }
  }

  return times;
}

// Benchmark asynchronous iteration with for await...of
async function benchAsync() {
  const times = [];

  for (let i = 0; i < iterations; i++) {
    const reader = new Reader(vcfPath);
    let count = 0;
    let posSum = 0;

    const start = performance.now();

    for await (const variant of reader) {
      posSum += variant.pos;
      count++;
    }

    const elapsed = performance.now() - start;
    times.push(elapsed);

    reader.close();

    if (count !== variantCount) {
      throw new Error(`Async count mismatch: ${count} vs ${variantCount}`);
    }
  }

  return times;
}

function stats(times) {
  const sorted = [...times].sort((a, b) => a - b);
  const sum = times.reduce((a, b) => a + b, 0);
  const mean = sum / times.length;
  const median = sorted[Math.floor(sorted.length / 2)];
  const min = sorted[0];
  const max = sorted[sorted.length - 1];
  const stddev = Math.sqrt(
    times.reduce((acc, t) => acc + (t - mean) ** 2, 0) / times.length
  );

  return { mean, median, min, max, stddev };
}

function formatStats(name, s, variants) {
  const throughput = (variants / s.mean) * 1000;
  return [
    `${name}:`,
    `  Mean:    ${s.mean.toFixed(3)} ms`,
    `  Median:  ${s.median.toFixed(3)} ms`,
    `  Min:     ${s.min.toFixed(3)} ms`,
    `  Max:     ${s.max.toFixed(3)} ms`,
    `  Stddev:  ${s.stddev.toFixed(3)} ms`,
    `  Throughput: ${(throughput / 1000).toFixed(1)}k variants/sec`,
  ].join("\n");
}

// Run benchmarks
console.log("Running synchronous benchmark...");
const syncTimes = await benchSync();
const syncStats = stats(syncTimes);

console.log("Running asynchronous benchmark...\n");
const asyncTimes = await benchAsync();
const asyncStats = stats(asyncTimes);

console.log("=".repeat(50));
console.log("RESULTS");
console.log("=".repeat(50));
console.log();
console.log(formatStats("Synchronous (nextSync)", syncStats, variantCount));
console.log();
console.log(formatStats("Asynchronous (for await)", asyncStats, variantCount));
console.log();

const ratio = asyncStats.mean / syncStats.mean;
const faster = ratio > 1 ? "Sync" : "Async";
const speedup = ratio > 1 ? ratio : 1 / ratio;

console.log("-".repeat(50));
console.log(
  `${faster} is ${speedup.toFixed(2)}x faster (${((speedup - 1) * 100).toFixed(1)}% improvement)`
);
console.log("-".repeat(50));

// Cleanup
fs.rmSync(tmpDir, { recursive: true, force: true });
console.log("\nTemporary files cleaned up.");
