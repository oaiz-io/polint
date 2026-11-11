// @exports
function clamp(value, limit) { // @fn clamp
  return Math.min(value, limit); // @external
}

function makeLimiter(limit) { // @fn makeLimiter
  return function limited(value) { // @fn limited
    return clamp(value, limit); // @calls clamp
  };
}

const limitTen = makeLimiter(10); // @calls makeLimiter
limitTen(42); // @calls limited
