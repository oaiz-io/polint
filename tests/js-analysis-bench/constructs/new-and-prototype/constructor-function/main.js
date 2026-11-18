// @exports
function Counter(start) { // @fn Counter
  this.value = start;
  this.step = increment;
}

function increment() { // @fn increment
  return 1;
}

Counter.prototype.next = function next() { // @fn next
  return this.value + this.step(); // @calls increment
};

const counter = new Counter(1); // @calls Counter
counter.next(); // @calls next
