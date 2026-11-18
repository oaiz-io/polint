// @exports
function one() { // @fn one
  return 1;
}

function two() { // @fn two
  return 2;
}

for (const fn of [one, two]) {
  fn(); // @calls one two
}

const deferred = [];
deferred.push(two);
for (const later of deferred) {
  later(); // @calls two
}
