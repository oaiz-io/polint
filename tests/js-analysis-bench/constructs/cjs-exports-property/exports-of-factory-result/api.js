// @exports parse
function makeParser() { // @fn makeParser
  return function parser(text) { // @fn parser
    return Number(text);
  };
}
exports.parse = makeParser(); // @calls makeParser
