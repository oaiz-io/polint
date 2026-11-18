// @exports fail
function defineError(name) { // @fn defineError
  function CustomError(message) { // @fn CustomError
    this.message = message;
  }
  CustomError.prototype.name = name;
  return CustomError;
}

function ParseFailure(message) { // @fn ParseFailure
  this.message = message;
}

const ParseError = defineError("ParseError"); // @calls defineError

function fail(text) { // @fn fail
  return new ParseError(text); // @calls CustomError
}

module.exports = { fail: fail, ParseFailure: ParseFailure };

fail("bad"); // @calls fail
