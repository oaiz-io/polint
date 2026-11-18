// @exports
function dispatch(config) { // @fn dispatch
  return this.adapter(config); // @calls adapter
}

function adapter(config) { // @fn adapter
  return config;
}

function sum(a, b) { // @fn sum
  return a + b;
}

const context = { adapter };
dispatch.call(context, 1); // @calls dispatch
sum.apply(null, [1, 2]); // @calls sum
