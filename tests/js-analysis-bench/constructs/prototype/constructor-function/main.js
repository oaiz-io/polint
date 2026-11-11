// @exports
function Counter(start) { // @fn Counter
  this.value = start;
}

Counter.prototype.increment = function increment() { // @fn increment
  this.value += 1;
  return this.report(); // @calls report
};

Counter.prototype.report = function report() { // @fn report
  return this.value;
};

const counter = new Counter(1); // @calls Counter
counter.increment(); // @calls increment
