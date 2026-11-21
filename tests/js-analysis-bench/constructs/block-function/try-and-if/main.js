// @exports describe
function describe(text) { // @fn describe
  try {
    function decode(t) { // @fn decode
      return JSON.parse(t);
    }
  } finally {
    text = String(text);
  }
  if (text.length > 1) {
    function measure(t) { // @fn measure
      return t.length;
    }
  } else {
    function shorten(t) { // @fn shorten
      return t.slice(0, 1);
    }
  }
  return [decode(text), measure(text)]; // @calls decode measure
}

function describeShort(text) { // @fn describeShort
  if (text.length > 1) {
    function unused() { // @fn unused
      return 0;
    }
  } else {
    function single(t) { // @fn single
      return t + t;
    }
  }
  return single(text); // @calls single
}

module.exports.describe = describe;
describe("12"); // @calls describe
describeShort("1"); // @calls describeShort
