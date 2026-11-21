// @exports
import { current, swap, deep } from "./barrel.mjs";

current(); // @calls firstImpl
swap(); // @calls swap
current(); // @calls secondImpl
deep(); // @calls deep
