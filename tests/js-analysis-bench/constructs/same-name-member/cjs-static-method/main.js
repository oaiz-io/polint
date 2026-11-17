// @exports tokenize
var Tokenizer = function Tokenizer() {}; // @fn Tokenizer

Tokenizer.tokenize = function (text) { // @fn staticTokenize
  return text.split(" "); // @external
};

function tokenize(text) { // @fn tokenize
  return Tokenizer.tokenize(text); // @calls staticTokenize
}

module.exports = { tokenize: tokenize };

tokenize("a b"); // @calls tokenize
