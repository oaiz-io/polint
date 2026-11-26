// @exports current swap
function firstImpl() { // @fn firstImpl
  return 1;
}

function secondImpl() { // @fn secondImpl
  return 2;
}

export let current = firstImpl;

export function swap() { // @fn swap
  current = secondImpl;
}
