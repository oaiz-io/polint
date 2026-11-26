// @exports load
function load(text) { // @fn load
  try {
    if (text) {
      function remember(t) { // @fn remember
        return { raw: t };
      }
    }
    JSON.parse(text);
  } catch (error) {
    function fallback(t) { // @fn fallback
      return { error: String(t) };
    }
  } finally {
    function close(value) { // @fn close
      return value;
    }
  }
  return close([remember(text), fallback(text)]); // @calls close remember fallback
}

module.exports.load = load;
load("not json"); // @calls load
