// @exports
function red() { // @fn red
  return "red";
}

function blue() { // @fn blue
  return "blue";
}

function* colours() { // @fn colours
  yield red;
  yield blue;
}

for (const colour of [...colours()]) { // @calls colours
  colour(); // @calls red blue
}

const [firstColour] = [...colours()]; // @calls colours
firstColour(); // @calls red
