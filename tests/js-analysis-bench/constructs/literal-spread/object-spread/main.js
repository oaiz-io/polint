// @exports
const base = {
  greet(name) { // @fn greet
    return `hi ${name}`;
  },
};

const extended = {
  ...base,
  wave(name) { // @fn wave
    return `wave ${name}`;
  },
};

extended.greet("a"); // @calls greet
extended.wave("b"); // @calls wave
const handlers = [base.greet, extended.wave];
const copy = [...handlers];
copy[0]("c"); // @calls greet
copy[1]("d"); // @calls wave
