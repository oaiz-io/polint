// @exports
function visit(node) { // @fn visit
  return node;
}

const walker = new Function("cb", "return cb(1);"); // @external
walker(visit); // @unknown
