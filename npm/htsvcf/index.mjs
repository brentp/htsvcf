import { createRequire } from "node:module";
import { platform, arch } from "node:os";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const require = createRequire(import.meta.url);
const __dirname = dirname(fileURLToPath(import.meta.url));

function getPlatformBinary() {
  const p = platform();
  const a = arch();

  let suffix;
  if (p === "linux" && a === "x64") {
    suffix = "linux-x64";
  } else if (p === "linux" && a === "arm64") {
    suffix = "linux-arm64";
  } else if (p === "darwin" && a === "arm64") {
    suffix = "darwin-arm64";
  } else {
    throw new Error(`Unsupported platform: ${p}-${a}`);
  }

  return join(__dirname, `htsvcf.${suffix}.node`);
}

// Try platform-specific binary first, fall back to generic htsvcf.node for local dev
let native;
try {
  native = require(getPlatformBinary());
} catch (e) {
  native = require("./htsvcf.node");
}

export const { Reader, Header, Variant, openReader } = native;

if (Reader && !Reader.prototype[Symbol.asyncIterator]) {
  Reader.prototype[Symbol.asyncIterator] = function () {
    return this;
  };
}
