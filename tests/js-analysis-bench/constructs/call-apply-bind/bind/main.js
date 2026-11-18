// @exports
function greet(greeting, name) { // @fn greet
  return `${greeting} ${name} ${this.suffix()}`; // @calls suffix
}

function suffix() { // @fn suffix
  return "!";
}

const hello = greet.bind({ suffix }, "hello");
hello("world"); // @calls greet
