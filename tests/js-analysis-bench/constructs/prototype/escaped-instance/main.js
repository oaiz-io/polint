// @exports createReader
function Reader(text) { // @fn Reader
  this.text = text;
  this.pos = 0;
}

Reader.prototype.peek = function peek() { // @fn peek
  return this.text[this.pos];
};

Reader.prototype.next = function next() { // @fn next
  const ch = this.peek(); // @calls peek
  this.pos += 1;
  return ch;
};

function createReader(text) { // @fn createReader
  return new Reader(text); // @calls Reader
}

module.exports = { createReader };

const reader = createReader("ab"); // @calls createReader
reader.next(); // @calls next
