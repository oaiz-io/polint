// @exports parse format
!function(t,e){"object"==typeof exports&&"undefined"!=typeof module?module.exports=e():"function"==typeof define&&define.amd?define(e):(t="undefined"!=typeof globalThis?globalThis:t||self).lib=e()}(this, // @fn umd @calls umd factory
function(){"use strict"; // @fn factory
var n=function(t){return String(t).trim()}; // @fn clean
function r(t){return n(t).split(",").map(n)} // @fn split @calls clean @callback clean
function o(t){return r(t).join(";")} // @fn join @calls split
return{parse:r,format:o}});
module.exports.format(" a, b "); // @calls join
