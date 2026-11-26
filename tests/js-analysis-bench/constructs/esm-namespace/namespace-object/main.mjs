// @exports
import * as tools from "./tools.mjs";

function useTools(ns) { // @fn useTools
  return ns.format(1); // @calls format
}

useTools(tools); // @calls useTools
tools.parse("1"); // @calls parse
