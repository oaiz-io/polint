// @exports apply
function f(a) { // @fn f
  return a + 1;
}

const m = {
  g(a) { // @fn g
    return a - 1;
  },
};

export function apply(a) { // @fn apply
  const once = (0, f)(a); // @calls f
  const twice = (0, (0, f))(once); // @calls f
  return (0, m.g)(twice); // @calls g
}

apply(1); // @calls apply
