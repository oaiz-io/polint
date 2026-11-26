// @exports first second default
function make(n) { // @fn make
  return n * 2;
}

function wrap(value) { // @fn wrap
  return { value };
}

export const first = make(1), second = wrap(make(2)); // @calls make wrap
export default wrap(first); // @calls wrap
