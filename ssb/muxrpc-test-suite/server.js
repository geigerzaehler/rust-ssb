const { createServer } = require("net");
const { callbackify } = require("util");
const lodash = require("lodash");
const pull = require("pull-stream");
const toPull = require("stream-to-pull-stream");
const muxrpc = require("muxrpc");

const { api, manifest } = require("./api");

const funcs = lodash.mapValues(api, ({ type, func }, method) => {
  if (type === "async") {
    return callbackify(function (...args) {
      console.log("ASYNC REQUEST %s", method, args);
      return func(...args).then(
        (value) => {
          console.log("ASYNC RESPONSE", value);
          return value;
        },
        (err) => {
          console.log("ASYNC RESPONSE ERROR", err);
          throw err;
        },
      );
    });
  } else {
    return func;
  }
});

const port = 19423;

createServer((stream) => {
  console.log("ACCEPTED CONNNECTION");
  stream.on("end", () => {
    console.log("CONNECTION CLOSED");
  });
  stream = toPull.duplex(stream);
  const server = muxrpc(null, manifest)(funcs);
  pull(stream, server.createStream(), stream);
}).listen(port, () => {
  console.log(`Server listening at localhost:${port}`);
});
