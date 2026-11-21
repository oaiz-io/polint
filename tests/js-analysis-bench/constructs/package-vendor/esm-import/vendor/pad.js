// @exports pad
export function pad(text, width) { // @fn pad
  return repeat(" ", width - text.length) + text; // @calls repeat
}

function repeat(text, count) { // @fn repeat
  return count > 0 ? text.repeat(count) : "";
}
