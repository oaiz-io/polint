// Exercises pako's minified bundle through its documented API (the readme and
// the API docs it links): one-shot deflate, inflate, raw and gzip forms, the
// chunked Deflate and Inflate classes, string input and output, the
// compression options, a preset dictionary, gzip headers and broken input.
// Inputs are small and fixed: every function of the bundle carries a probe.
import pako from "/bench/pkg/dist/pako.min.js";

const step = (name, run) => {
  try {
    run();
  } catch (error) {
    console.error(`pako driver step ${name}: ${error.message}`);
  }
};

const text = Array.from({ length: 40 }, (_, i) => `line ${i}: the quick brown fox ${i % 7} jumps\n`).join("");
const bytes = new TextEncoder().encode(text);
// Little repetition, so the encoder also takes its stored and literal paths.
const noisy = Uint8Array.from({ length: 2048 }, (_, i) => (i * 2654435761) >>> 24);

step("deflate and inflate", () => {
  const compressed = pako.deflate(bytes);
  pako.inflate(compressed);
});
step("strings", () => {
  const test = { my: "super", puper: [456, 567], awesome: "pako", text: "ünïcödé ✓" };
  const compressed = pako.deflate(JSON.stringify(test));
  JSON.parse(pako.inflate(compressed, { to: "string" }));
});
step("raw", () => pako.inflateRaw(pako.deflateRaw(bytes)));
step("gzip", () => pako.ungzip(pako.gzip(bytes)));
step("gzip header", () => {
  const compressed = pako.gzip(bytes, {
    header: { text: true, time: 1000000, os: 3, name: "file.txt", comment: "c", extra: [1, 2, 3], hcrc: true },
  });
  const inflator = new pako.Inflate();
  inflator.push(compressed);
  inflator.header;
});
for (const level of [0, 1, 4, 6, 9]) {
  step(`level ${level}`, () => pako.inflate(pako.deflate(noisy, { level })));
}
for (const strategy of [1, 2, 3, 4]) {
  step(`strategy ${strategy}`, () => pako.inflate(pako.deflate(bytes, { strategy })));
}
step("window and memory", () => pako.inflate(pako.deflate(bytes, { windowBits: 9, memLevel: 1 }), { windowBits: 9 }));
step("dictionary", () => {
  const dictionary = new TextEncoder().encode("the quick brown fox jumps");
  const compressed = pako.deflate(bytes, { dictionary });
  pako.inflate(compressed, { dictionary });
});
step("chunked", () => {
  const deflator = new pako.Deflate({ chunkSize: 256 });
  deflator.push(bytes.subarray(0, 500), false);
  deflator.push(bytes.subarray(500), true);
  if (deflator.err) console.error(deflator.msg);
  const inflator = new pako.Inflate({ chunkSize: 128 });
  const output = deflator.result;
  inflator.push(output.subarray(0, 50));
  inflator.push(output.subarray(50));
  if (inflator.err) console.error(inflator.msg);
  inflator.result;
});
step("chunked flush", () => {
  const deflator = new pako.Deflate();
  deflator.push(bytes, 2);
  deflator.push(new Uint8Array(0), 4);
  deflator.result;
});
step("onData and onEnd", () => {
  const parts = [];
  const inflator = new pako.Inflate({ to: "string" });
  inflator.onData = (chunk) => parts.push(chunk);
  inflator.onEnd = (status) => parts.push(status);
  inflator.push(pako.deflate(text));
});
step("broken stream", () => {
  try {
    pako.inflate(new Uint8Array([1, 2, 3, 4, 5]));
  } catch (error) {
    String(error);
  }
  const inflator = new pako.Inflate();
  inflator.push(new Uint8Array([120, 156, 255, 255, 0]));
  inflator.err;
  inflator.msg;
});
step("array buffer input", () => pako.inflate(pako.deflate(bytes.buffer)));
