// @exports label
import { pad } from "./vendor/pad.js";

export function label(text) { // @fn label
  return pad(text, 4); // @calls pad
}

label("a"); // @calls label
