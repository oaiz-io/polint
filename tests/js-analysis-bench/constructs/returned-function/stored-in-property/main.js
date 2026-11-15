// @exports
const head = (list) => list[0]; // @fn head
function makePicker(which, custom) { // @fn makePicker
  if (which === "head") return head;
  if (which === "custom") return custom;
  return function tail(list) { // @fn tail
    return list[list.length - 1];
  };
}
function middle(list) { // @fn middle
  return list[1];
}
const box = {};
for (const which of ["head", "custom", "tail"]) {
  box.pick = makePicker(which, middle); // @calls makePicker
  box.pick([1, 2, 3]); // @calls head middle tail
}
