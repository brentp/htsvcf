const path = require("path");
const os = require("os");

function getPlatformBinary() {
  const platform = os.platform();
  const arch = os.arch();

  let suffix;
  if (platform === "linux" && arch === "x64") {
    suffix = "linux-x64";
  } else if (platform === "linux" && arch === "arm64") {
    suffix = "linux-arm64";
  } else if (platform === "darwin" && arch === "arm64") {
    suffix = "darwin-arm64";
  } else {
    throw new Error(`Unsupported platform: ${platform}-${arch}`);
  }

  return path.join(__dirname, `htsvcf.${suffix}.node`);
}

// Try platform-specific binary first, fall back to generic htsvcf.node for local dev
let native;
try {
  native = require(getPlatformBinary());
} catch (e) {
  native = require("./htsvcf.node");
}

module.exports = native;

// Provide a fast asynchronous iterator backed by nextBatchAsync().
if (native.Reader && !native.Reader.prototype[Symbol.asyncIterator]) {
  native.Reader.prototype[Symbol.asyncIterator] = function () {
    const reader = this;
    let batch = [];
    let index = 0;

    return {
      async next() {
        // Refill batch when exhausted
        if (index >= batch.length) {
          batch = await reader.nextBatchAsync();
          index = 0;
          if (batch.length === 0) {
            return { done: true, value: undefined };
          }
        }
        return { done: false, value: batch[index++] };
      },
      [Symbol.asyncIterator]() {
        return this;
      },
    };
  };
}

// Provide a fast synchronous iterator backed by nextBatchSync().
// This keeps async iteration available via Symbol.asyncIterator.
if (native.Reader && !native.Reader.prototype[Symbol.iterator]) {
  native.Reader.prototype[Symbol.iterator] = function () {
    const reader = this;
    let batch = [];
    let index = 0;

    return {
      next() {
        // Refill batch when exhausted
        if (index >= batch.length) {
          batch = reader.nextBatchSync();
          index = 0;
          if (batch.length === 0) {
            return { done: true, value: undefined };
          }
        }
        return { done: false, value: batch[index++] };
      },
      [Symbol.iterator]() {
        return this;
      },
    };
  };
}
