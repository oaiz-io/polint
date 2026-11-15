// @exports parseName
function define(build: (input: string) => string): (input: string) => string { // @fn define
  return (input) => build(input); // @fn wrapped @calls parseBody
}

function normalize(input: string): string { // @fn normalize
  return input.trim(); // @external
}

export const parseName = define((input) => normalize(input)); // @fn parseBody @calls define normalize

parseName(" Ada "); // @calls wrapped
