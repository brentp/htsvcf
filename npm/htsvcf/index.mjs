import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

// For now we expect a local build to place the addon at ./htsvcf.node
// CI/prebuilds will drop the correct binary here.
const native = require("./htsvcf.node");

export const { Reader, Header, Variant, openReader } = native;

if (Reader && !Reader.prototype[Symbol.asyncIterator]) {
  Reader.prototype[Symbol.asyncIterator] = function () {
    return this;
  };
}
