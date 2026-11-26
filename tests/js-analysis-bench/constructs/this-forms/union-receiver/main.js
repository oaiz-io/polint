// @exports
class Option {
  choices() { // @fn optionChoices
    this.parseArg = (arg) => `option ${arg}`; // @fn optionParse
    return this;
  }
}

class Argument {
  choices() { // @fn argumentChoices
    this.parseArg = (arg) => `argument ${arg}`; // @fn argumentParse
    return this;
  }
}

function parse(target, value) { // @fn parse
  return target.parseArg(value); // @calls optionParse argumentParse
}

const option = new Option().choices(); // @calls optionChoices
const argument = new Argument().choices(); // @calls argumentChoices
parse(option, "a"); // @calls parse
parse(argument, "b"); // @calls parse
