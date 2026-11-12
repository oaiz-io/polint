// @exports
function exclaim(text) { // @fn exclaim
  return text + "!";
}
function choose(kind, custom) { // @fn choose
  if (kind === 1) return exclaim;
  if (kind === 2) return custom;
  return function plain(text) { // @fn plain
    return text;
  };
}
function resolve(kind, custom) { // @fn resolve
  return choose(kind, custom); // @calls choose
}
function shout(text) { // @fn shout
  return text + "!!";
}
function apply(kind, fn, value) { // @fn apply
  fn = resolve(kind, fn); // @calls resolve
  return fn(value); // @calls exclaim shout plain
}
for (const kind of [1, 2, 3]) {
  apply(kind, shout, "x"); // @calls apply
}
