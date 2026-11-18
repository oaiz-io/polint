// @exports
class Counter {
  constructor() { // @fn Counter.constructor
    this.count = 0;
  }

  bump() { // @fn bump
    this.count += 1;
    return this.count;
  }
}

class Tally extends Counter {
  constructor() { // @fn Tally.constructor
    super(); // @calls Counter.constructor
  }

  add(n) { // @fn add
    for (let i = 0; i < n; i += 1) {
      this.bump(); // @calls bump
    }
    return this.count;
  }
}

const tally = new Tally(); // @calls Tally.constructor
module.exports = tally;

tally.add(2); // @calls add
tally.bump(); // @calls bump
