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

if (native.Reader && !native.Reader.prototype[Symbol.asyncIterator]) {
  native.Reader.prototype[Symbol.asyncIterator] = function () {
    return this;
  };
}
