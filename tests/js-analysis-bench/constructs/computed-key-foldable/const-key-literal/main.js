// @exports
function parse(text) { // @fn parse
  return text.trim();
}

function stringify(value) { // @fn stringify
  return String(value);
}

const PARSE = "parse";
const codec = {
  [PARSE]: parse,
  [`stringify`]: stringify,
};
codec[PARSE](" a "); // @calls parse
codec.stringify(1); // @calls stringify
