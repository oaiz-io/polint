// Exercises axios through its public entry without leaving the sandbox: a
// custom adapter, the fetch adapter with an in-process fetch, and the Node
// http adapter against a loopback server started here.
import http from "node:http";
import axios from "/bench/pkg/lib/axios.js";

const step = async (name, run) => {
  try {
    await run();
  } catch (error) {
    if (!axios.isCancel(error) && !axios.isAxiosError(error)) console.error(`axios driver step ${name}: ${error.message}`);
  }
};

const echo = (config) =>
  Promise.resolve({ data: JSON.stringify({ url: config.url, method: config.method }), status: 200, statusText: "OK", headers: { "content-type": "application/json" }, config, request: {} });

await step("custom adapter", async () => {
  const client = axios.create({ baseURL: "http://bench.invalid/api", adapter: echo, timeout: 1000, headers: { "X-Bench": "1" } });
  client.interceptors.request.use((config) => {
    config.headers.set("X-Seen", "yes");
    return config;
  });
  const responseId = client.interceptors.response.use((response) => response, (error) => Promise.reject(error));
  await client.get("/items", { params: { page: 2, tags: ["a", "b"], when: new Date(0) } });
  await client.post("/items", { name: "x", nested: { list: [1, 2] } });
  await client.put("/items/1", "plain", { headers: { "Content-Type": "text/plain" } });
  await client.patch("/items/1", new URLSearchParams({ a: "1" }));
  await client.delete("/items/1", { data: { reason: "bench" } });
  await client.head("/items");
  await client.options("/items");
  await client.request({ url: "/form", method: "post", data: { a: 1, b: [1, 2] }, headers: { "Content-Type": "multipart/form-data" } });
  await client.postForm("/form", { a: "1", file: new Blob(["abc"]) });
  await client.putForm("/form", { a: "1" });
  await client({ url: "/call", paramsSerializer: { indexes: true } , params: { list: [1, 2] } });
  client.interceptors.response.eject(responseId);
  client.interceptors.request.clear();
  client.getUri({ url: "/x", params: { q: "y" } });
});

await step("rejections", async () => {
  const failing = axios.create({ adapter: (config) => Promise.resolve({ data: "", status: 500, statusText: "ERR", headers: {}, config, request: {} }) });
  await failing.get("http://bench.invalid/fail");
});

await step("validation", async () => {
  await axios.get("http://bench.invalid/", { adapter: echo, transitional: { silentJSONParsing: "yes" } });
});

await step("cancel token", async () => {
  const source = axios.CancelToken.source();
  source.cancel("stop");
  await axios.get("http://bench.invalid/", { adapter: echo, cancelToken: source.token });
});

await step("abort signal", async () => {
  const controller = new AbortController();
  controller.abort();
  await axios.get("http://bench.invalid/", { adapter: echo, signal: controller.signal });
});

await step("helpers", async () => {
  const headers = new axios.AxiosHeaders({ "content-type": "text/plain", Accept: "a" });
  headers.set("X-A", "1").set({ "X-B": "2" });
  headers.has("x-a");
  headers.get("content-type", true);
  headers.delete("X-B");
  headers.normalize(true);
  headers.toJSON();
  headers.toString();
  [...headers];
  axios.AxiosHeaders.from("A: 1\r\nB: 2");
  axios.AxiosHeaders.concat({ a: "1" }, { b: "2" });
  axios.mergeConfig({ headers: { a: "1" }, params: { x: 1 } }, { headers: { b: "2" }, timeout: 5 });
  axios.toFormData({ a: 1, b: [1, 2], c: { d: true } });
  const form = new FormData();
  form.append("a[b]", "1");
  form.append("a[c]", "2");
  axios.formToJSON(form);
  axios.isAxiosError(new axios.AxiosError("x", "ERR_BENCH"));
  axios.AxiosError.from(new Error("inner"), "ERR_BENCH", {}, {}, {});
  axios.isCancel(new axios.CanceledError());
  axios.spread((a, b) => a + b)([1, 2]);
  axios.getAdapter(["fetch", "http"]);
  await axios.all([Promise.resolve(1)]);
  axios.HttpStatusCode.Ok;
});

await step("fetch adapter", async () => {
  const fetch = async (input) =>
    new Response(JSON.stringify({ ok: true, url: String(input.url ?? input) }), { status: 200, headers: { "content-type": "application/json" } });
  const client = axios.create({ adapter: "fetch", env: { fetch, Request, Response } });
  await client.get("http://bench.invalid/fetch", { params: { a: 1 } });
  await client.post("http://bench.invalid/fetch", { a: 1 }, { responseType: "json" });
  await client.post("http://bench.invalid/fetch", "text", { responseType: "text", onUploadProgress: () => {}, onDownloadProgress: () => {} });
});

await step("http adapter", async () => {
  const server = http.createServer((request, response) => {
    let body = "";
    request.on("data", (chunk) => (body += chunk));
    request.on("end", () => {
      if (request.url.startsWith("/redirect")) {
        response.writeHead(302, { location: "/plain" });
        response.end();
        return;
      }
      response.writeHead(request.url.startsWith("/missing") ? 404 : 200, { "content-type": "application/json" });
      response.end(JSON.stringify({ url: request.url, method: request.method, length: body.length }));
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const client = axios.create({ adapter: "http", baseURL: base, proxy: false });
  await step("get", () => client.get("/plain", { params: { a: 1 } }));
  await step("post", () => client.post("/json", { a: 1 }, { onUploadProgress: () => {}, onDownloadProgress: () => {} }));
  await step("stream", () => client.get("/plain", { responseType: "stream" }).then((response) => response.data.resume()));
  await step("text", () => client.get("/plain", { responseType: "text", maxRedirects: 0 }));
  await step("buffer", () => client.get("/plain", { responseType: "arraybuffer", decompress: false }));
  await step("missing", () => client.get("/missing"));
  await step("redirect", () => client.get("/redirect", { maxRedirects: 0 }));
  await step("form", () => client.postForm("/form", { a: "1", b: "2" }));
  await step("data uri", () => client.get("data:text/plain;base64,aGVsbG8="));
  await step("auth", () => client.get("/plain", { auth: { username: "u", password: "p" } }));
  server.close();
});
