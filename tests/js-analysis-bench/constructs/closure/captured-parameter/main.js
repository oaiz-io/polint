// @exports
function lazy(target, key, make) { // @fn lazy
  Object.defineProperty(target, key, {
    get() { // @fn get
      return make(this); // @calls build
    },
  });
}

function build(owner) { // @fn build
  return () => owner; // @fn arrow
}

const proto = {};
lazy(proto, "value", build); // @calls lazy
proto.value(); // @calls get arrow
