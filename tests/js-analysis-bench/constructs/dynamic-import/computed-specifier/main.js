// @exports
const locale = process.env.BENCH_LOCALE || "en";
const messages = await import(`./locale-${locale}.js`); // @external
messages.greet(); // @unknown
