const native = require("./htsvcf.node");

module.exports = native;

if (native.Reader && !native.Reader.prototype[Symbol.asyncIterator]) {
  native.Reader.prototype[Symbol.asyncIterator] = function () {
    return this;
  };
}
