// @exports
function head() { // @fn head
  return "head";
}

function tail() { // @fn tail
  return "tail";
}

function spare() { // @fn spare
  return "spare";
}

const [first, ...others] = [head, tail];
first(); // @calls head
others[0](); // @calls tail
const { inner: [deep = spare] } = { inner: [] };
deep(); // @calls spare
